//! Pagination & the band-emit cursor: begin/finish a page (top-of-page band order, page footer pin),
//! walk the group tree and detail rows, and place each band at the cursor, breaking to a new page when
//! a band would overflow the body. Text planning (`text_plan`) and the band's grown height
//! (`band_plans_and_height`) live here too, since they gate where a band lands. The actual per-object
//! draw-ops are emitted by [`crate::place`]; this module owns the vertical flow and page breaks.

use crate::{first_row, font_of, resolve::ResolveState, Formatter, GroupScope, TextPlan};
use rpt_data::{DataContext, GroupInstance, Row};
use rpt_model::{ReportObject, ReportObjectKind, Section};
use rpt_pages::{ObjectKind, Page, PageCheckpoint};

impl Formatter<'_> {
    /// Resolve a text/field object to its wrapped display lines (`None` for non-text objects). Wraps
    /// only when the object has **Can-Grow** set (`obj.format.can_grow`) *and* the band allows growth
    /// (`allow_grow`); otherwise the box clips. Can-grow is inert in a page header/footer — a fixed
    /// repeating band — so those pass `allow_grow = false` (matching the native engine). Computed once
    /// per band so height and emitted runs agree.
    fn text_plan(
        &self,
        obj: &ReportObject,
        ctx: Option<&DataContext>,
        state: &ResolveState,
        allow_grow: bool,
    ) -> Option<TextPlan> {
        use crate::resolve::{cond_color, field_text, text_display};
        let (raw, font, color, kind) = match &obj.kind {
            ReportObjectKind::Field(f) => (
                ctx.map(|c| field_text(f, c, state, &self.locale, &self.diagnostics))
                    .unwrap_or_default(),
                font_of(&f.font_color.font),
                cond_color(&f.font_color.condition_formulas, "Color", ctx)
                    .unwrap_or(f.font_color.color),
                ObjectKind::Field,
            ),
            ReportObjectKind::Text(t) => (
                text_display(t, ctx, state, &self.locale, &self.diagnostics),
                font_of(&t.font_color.font),
                cond_color(&t.font_color.condition_formulas, "Color", ctx)
                    .unwrap_or(t.font_color.color),
                ObjectKind::Text,
            ),
            _ => return None,
        };
        // Explicit line breaks (a multi-line label like "Numeric\nCode") always split into separate
        // runs so every backend renders them as lines. Can-grow additionally word-wraps each line.
        let lines = if obj.format.can_grow && allow_grow {
            self.text_layout
                .wrap(&raw, obj.bounds.width.0 as f64, &font)
        } else {
            raw.split('\n').map(str::to_string).collect()
        };
        Some(TextPlan {
            lines,
            font,
            color,
            align: crate::align_of(obj.format.horizontal_alignment),
            kind,
        })
    }

    /// Emit one band (section) at the cursor, paginating first if it would overflow the body. The
    /// band grows vertically when a `can-grow` text object wraps to more lines than its box holds.
    pub(crate) fn emit_band(&mut self, section: &Section, row: Option<&Row>, state: &ResolveState) {
        if section.format.base.suppress {
            return;
        }
        // NewPageBefore on this band, or a deferred NewPageAfter from the previous band, starts a
        // fresh page — but not when we are already at the top of one (that would leave a blank page).
        if (self.pending_page_break || section.format.base.new_page_before)
            && self.cursor_y > self.content_top
        {
            self.finish_page();
            self.begin_page();
        }
        self.pending_page_break = false;

        let ctx = row.map(|r| self.context(r, state));
        // A body band (Detail / Group / Report Header-Footer) is a flow section: can-grow applies.
        let (plans, height) = self.band_plans_and_height(section, ctx.as_ref(), state, true);
        // A band never splits mid-section: if it would overflow the body, move the whole band to a
        // new page (this also satisfies section-level KeepTogether for a single band).
        if self.cursor_y + height > self.body_bottom && self.cursor_y > self.content_top {
            self.finish_page();
            self.begin_page();
        }
        // PrintAtBottomOfPage: pin the band (a group/report footer) against the bottom of the body,
        // above the page footer — like the page-footer pin, but for a flow band. It never rises above
        // the current cursor (a band already low on the page stays where it is and overflows normally).
        let pin_bottom = section.format.base.print_at_bottom_of_page;
        let origin_y = if pin_bottom {
            (self.body_bottom - height).max(self.cursor_y)
        } else {
            self.cursor_y
        };
        self.emit_band_plans(section, &plans, origin_y, height, ctx.as_ref());
        // "Underlay Following Sections": the band is a background for the sections that follow it.
        // It is emitted first (so painter's order puts its ops underneath the later bands) and does
        // not advance the cursor, so the following section(s) draw over the same vertical region
        // instead of being pushed below. First cut: the underlay backs whatever follows in normal
        // flow; multi-area spans and section opacity are follow-ups.
        if pin_bottom {
            // The band consumed the rest of the page: the next flow band starts a fresh page.
            self.cursor_y = self.body_bottom;
        } else if !section.format.underlay_section {
            self.cursor_y += height;
        }

        // NewPageAfter: defer the break to the next flow band so a trailing one adds no blank page.
        if section.format.base.new_page_after {
            self.pending_page_break = true;
        }
        // ResetPageNumberAfter: restart the page-number counter at the next page top.
        if section.format.base.reset_page_number_after {
            self.pending_page_number_reset = true;
        }
    }

    /// Resolve a band's per-object text plans and its grown height (base section height, extended by
    /// any can-grow object that wrapped to more lines than its box). No pagination, no emit.
    pub(crate) fn band_plans_and_height(
        &self,
        section: &Section,
        ctx: Option<&DataContext>,
        state: &ResolveState,
        allow_grow: bool,
    ) -> (Vec<Option<TextPlan>>, i32) {
        let plans: Vec<Option<TextPlan>> = section
            .objects
            .iter()
            .map(|o| self.text_plan(o, ctx, state, allow_grow))
            .collect();
        let mut height = section.height.0;
        for (o, p) in section.objects.iter().zip(&plans) {
            if let Some(p) = p {
                if o.format.can_grow && p.lines.len() > 1 {
                    let lh = self.text_layout.line_height_twips(&p.font) as i32;
                    height = height.max(o.bounds.top.0 + p.lines.len() as i32 * lh);
                }
            }
        }
        (plans, height)
    }

    /// Keep Group Together: measure the whole group subtree and, if it won't fit in the space left on
    /// the current page but *would* fit on a fresh page, move it to a new page before its header. A
    /// group taller than a whole page is left to paginate naturally (forcing a break wouldn't help).
    /// Measured from static design heights only — resolving can-grow content here would re-fire
    /// `WhilePrintingRecords` variable writes, so growth is deliberately not anticipated.
    fn keep_group_together(&mut self, g: &GroupInstance) {
        let keep = self
            .bands
            .group_formats
            .get(g.level)
            .is_some_and(|f| f.keep_group_together);
        if !keep {
            return;
        }
        let height = self.measure_group_height(g);
        let body_height = self.body_bottom - self.content_top;
        if height <= body_height
            && self.cursor_y + height > self.body_bottom
            && self.cursor_y > self.content_top
        {
            self.finish_page();
            self.begin_page();
        }
    }

    /// The static design height of a group subtree (its header + children + footer), used by
    /// [`Self::keep_group_together`]. Sums section design heights (no can-grow growth); a nested
    /// keep-together subgroup contributes its own subtree height.
    fn measure_group_height(&self, g: &GroupInstance) -> i32 {
        fn band_height(sections: &[&Section]) -> i32 {
            sections
                .iter()
                .filter(|s| !s.format.base.suppress)
                .map(|s| s.height.0)
                .sum()
        }
        let mut height = 0;
        if let Some(hdr) = self.bands.group_headers.get(g.level) {
            height += band_height(hdr);
        }
        if g.subgroups.is_empty() {
            let detail_h = band_height(&self.bands.detail);
            let rows = g.details.len() as i32;
            // Multi-column details lay `columns` records side by side, so the block is roughly
            // `ceil(rows / columns)` band-rows tall.
            let cols = self
                .multi_column
                .map(|mc| mc.columns.max(1) as i32)
                .unwrap_or(1);
            let band_rows = if cols > 1 {
                (rows + cols - 1) / cols
            } else {
                rows
            };
            height += band_rows * detail_h;
        } else {
            for sub in &g.subgroups {
                height += self.measure_group_height(sub);
            }
        }
        if let Some(ftr) = self.bands.group_footers.get(g.level) {
            height += band_height(ftr);
        }
        height
    }

    pub(crate) fn emit_group(&mut self, g: &GroupInstance) {
        self.keep_group_together(g);
        // Enter this group: its scope is now in effect for band/summary resolution and running-total
        // reset detection.
        self.group_stack.push(GroupScope {
            key: g.key.clone(),
            condition_field: g.condition_field.clone(),
            summaries: g.summaries.clone(),
        });
        self.refresh_group_summaries();
        let key = Some(g.key.clone());
        let mut state = self.state(key.clone());
        state.summaries = g.summaries.clone();
        // Group header for this level.
        if let Some(hdr) = self.bands.group_headers.get(g.level).cloned() {
            let first = g.details.first().or_else(|| first_row(g));
            for s in hdr {
                self.emit_band(s, first, &state);
            }
        }
        // Children: subgroups or detail rows.
        if g.subgroups.is_empty() {
            self.emit_details(&g.details, &g.summaries);
        } else {
            for sub in &g.subgroups {
                self.emit_group(sub);
            }
        }
        // Group footer for this level.
        if let Some(ftr) = self.bands.group_footers.get(g.level).cloned() {
            let first = g.details.first().or_else(|| first_row(g));
            for s in ftr {
                self.emit_band(s, first, &state);
            }
        }
        self.group_stack.pop();
        self.refresh_group_summaries();
    }

    pub(crate) fn emit_details(&mut self, rows: &[Row], summaries: &[rpt_data::Summary]) {
        let detail_bands: Vec<&Section> = self.bands.detail.clone();
        if let Some(mc) = self.multi_column {
            if mc.columns > 1 {
                self.emit_details_multicol(rows, summaries, &detail_bands, mc);
                return;
            }
        }
        for row in rows {
            self.record_number += 1;
            let mut state = self.state(None);
            state.summaries = summaries.to_vec();
            // Advance running totals in print order before emitting the band, so a `{#name}` object
            // shows the value accumulated up to and including this record.
            self.advance_running_totals(row, &state);
            for s in &detail_bands {
                self.emit_band(s, Some(row), &state);
            }
        }
    }

    /// Emit detail records flowing across `mc.columns` columns ("Format with Multiple Columns"),
    /// honoring the section's fill order:
    ///
    /// - **across then down** (`mc.across_then_down`): each record sits at the next column offset on a
    ///   shared row-top; after a full row of columns the cursor drops by the tallest record in the row.
    /// - **down then across**: records fill one column top-to-bottom until the next would overflow the
    ///   body, then continue at the top of the next column; when all columns fill, the page breaks.
    ///
    /// Records are processed in print order either way, so running totals accumulate identically.
    fn emit_details_multicol(
        &mut self,
        rows: &[Row],
        summaries: &[rpt_data::Summary],
        bands: &[&Section],
        mc: rpt_model::MultiColumn,
    ) {
        let cols = mc.columns.max(1) as i32;
        let pitch = mc.column_width.0 + mc.gap_h.0;
        let across = mc.across_then_down;
        let mut col: i32 = 0;
        // `col_top` is the shared row-top (across) or every column's top (down); `y` is the current
        // fill position within a column (down); `row_h` tracks the tallest record in a column-row
        // (across); `deepest` tracks the lowest point reached on this page (down).
        let mut col_top = self.cursor_y;
        let mut y = col_top;
        let mut row_h = 0;
        let mut deepest = col_top;
        // Whether the current page already carries body content (a group header, or a prior record),
        // so a NewPageBefore / deferred break doesn't leave a leading blank page — the same guard the
        // single-column `emit_band` gets from `cursor_y > content_top`.
        let mut dirty = self.cursor_y > self.content_top;
        for row in rows {
            self.record_number += 1;
            let mut state = self.state(None);
            state.summaries = summaries.to_vec();
            self.advance_running_totals(row, &state);
            let ctx = self.context(row, &state);
            // Resolve every detail band's plans + height once (reused for pagination and emit).
            let banded: Vec<(&Section, Vec<Option<TextPlan>>, i32)> = bands
                .iter()
                .filter(|s| !s.format.base.suppress)
                .map(|s| {
                    let (plans, h) = self.band_plans_and_height(s, Some(&ctx), &state, true);
                    (*s, plans, h)
                })
                .collect();
            let rec_h: i32 = banded.iter().map(|(_, _, h)| h).sum();

            // NewPageBefore on the detail section, or a break deferred from a prior band, starts a
            // fresh page before this record and resets the column cursor (mirrors `emit_band`).
            let new_page_before = banded.iter().any(|(s, _, _)| s.format.base.new_page_before);
            if (self.pending_page_break || new_page_before) && dirty {
                self.finish_page();
                self.begin_page();
                col = 0;
                col_top = self.cursor_y;
                y = col_top;
                row_h = 0;
                deepest = col_top;
            }
            self.pending_page_break = false;

            let record_top = if across {
                // Page break decided at the start of a column-row (col 0) so a row stays on one page.
                if col == 0 && col_top + rec_h > self.body_bottom && col_top > self.content_top {
                    self.finish_page();
                    self.begin_page();
                    col_top = self.cursor_y;
                }
                col_top
            } else {
                // Overflow the column → next column; overflow the last column → next page.
                if y + rec_h > self.body_bottom && y > col_top {
                    col += 1;
                    if col >= cols {
                        self.finish_page();
                        self.begin_page();
                        col = 0;
                        col_top = self.cursor_y;
                        deepest = col_top;
                    }
                    y = col_top;
                }
                y
            };

            self.col_offset = col * pitch;
            let mut band_y = record_top;
            for (s, plans, h) in &banded {
                self.emit_band_plans(s, plans, band_y, *h, Some(&ctx));
                band_y += *h;
            }
            self.col_offset = 0;
            deepest = deepest.max(band_y);

            if across {
                row_h = row_h.max(band_y - col_top);
                col += 1;
                if col >= cols {
                    col = 0;
                    col_top += row_h + mc.gap_v.0;
                    row_h = 0;
                    self.cursor_y = col_top;
                }
            } else {
                y = band_y + mc.gap_v.0;
            }
            dirty = true;
            // NewPageAfter / ResetPageNumberAfter defer to the next flow band, exactly as `emit_band`
            // does (the break-before check above consumes the deferred page break).
            if banded.iter().any(|(s, _, _)| s.format.base.new_page_after) {
                self.pending_page_break = true;
            }
            if banded
                .iter()
                .any(|(s, _, _)| s.format.base.reset_page_number_after)
            {
                self.pending_page_number_reset = true;
            }
        }
        // Leave the cursor below the deepest emitted content so following bands don't overlap.
        if across {
            if col != 0 {
                self.cursor_y = col_top + row_h;
            }
        } else {
            self.cursor_y = deepest;
        }
    }

    pub(crate) fn begin_page(&mut self) {
        // A section with ResetPageNumberAfter set the counter to restart here: the new page prints
        // as page 1 (the increment below lands on 1 from 0).
        if self.pending_page_number_reset {
            self.page_number = 0;
            self.pending_page_number_reset = false;
        }
        self.page_number += 1;
        self.cur = Page::new(self.page_number as u32, self.page_size);
        self.cur.origin = self.origin;
        self.cursor_y = self.content_top;
        self.checkpoints.push(PageCheckpoint {
            page_number: self.page_number as u32,
            record_position: self.record_number as u64,
            state: Default::default(),
        });
        // Crystal band order at the top of a page: the report header prints once, at the very top
        // of page 1, ABOVE the page header; the page header then repeats on every page below it.
        if self.page_number == 1 {
            let rh: Vec<&Section> = self.bands.report_header.clone();
            for s in rh {
                let state = self.state(None);
                // Report Header is a flow section — can-grow objects extend it.
                self.emit_band_no_paginate(s, None, &state, true);
            }
        }
        let ph: Vec<&Section> = self.bands.page_header.clone();
        for s in ph {
            let state = self.state(None);
            // Page Header is a fixed repeating band — can-grow is inert.
            self.emit_band_no_paginate(s, None, &state, false);
        }
    }

    pub(crate) fn finish_page(&mut self) {
        // Pin the page footer to the bottom.
        let pf: Vec<&Section> = self.bands.page_footer.clone();
        self.cursor_y = self.page_footer_top;
        for s in pf {
            let state = self.state(None);
            // Page Footer is a fixed repeating band — can-grow is inert (it must fit the space
            // reserved at the page bottom).
            self.emit_band_no_paginate(s, None, &state, false);
        }
        let page = std::mem::replace(&mut self.cur, Page::new(0, self.page_size));
        self.pages.push(page);
    }

    /// Emit an already-positioned band without the overflow check. `allow_grow` selects the band
    /// family: a Report Header (a flow section) grows with can-grow content, so it advances the
    /// cursor by the grown height; a Page Header / Page Footer is a fixed repeating band where
    /// can-grow is inert, so it stays at its designed height (native behavior — see the SDK note in
    /// [`Formatter::text_plan`]).
    fn emit_band_no_paginate(
        &mut self,
        section: &Section,
        row: Option<&Row>,
        state: &ResolveState,
        allow_grow: bool,
    ) {
        if section.format.base.suppress {
            return;
        }
        let ctx = row.map(|r| self.context(r, state));
        let (plans, height) = self.band_plans_and_height(section, ctx.as_ref(), state, allow_grow);
        let origin_y = self.cursor_y;
        self.emit_band_plans(section, &plans, origin_y, height, ctx.as_ref());
        // An underlay band backs what follows: keep the cursor at its top so the next band overlays
        // it (see `emit_band`).
        if !section.format.underlay_section {
            self.cursor_y += height;
        }
    }
}
