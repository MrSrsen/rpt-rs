//! Pagination & the band-emit cursor: begin/finish a page (top-of-page band order, page footer pin),
//! walk the group tree and detail rows, and place each band at the cursor, breaking to a new page when
//! a band would overflow the body. Text planning (`text_plan`) and the band's grown height
//! (`band_plans_and_height`) live here too, since they gate where a band lands. The actual per-object
//! draw-ops are emitted by [`crate::place`]; this module owns the vertical flow and page breaks.

use crate::{
    first_row, font_of, resolve::ResolveState, Formatter, GroupScope, LayoutLine, TextPlan,
};
use rpt_data::{DataContext, GroupInstance, Row};
use rpt_model::{
    LineSpacing, Paragraph, ReadingOrder, ReportObject, ReportObjectKind, Section, Twips,
};
use rpt_pages::{FontSpec, ObjectKind, Page, PageCheckpoint, TextAlign, TextLayout};
use std::rc::Rc;

/// Wrap `text` to `avail` twips, but break the **first** line at `avail - first` twips — a
/// paragraph's first-line indent narrows where the first line wraps as well as where it sits.
/// `first <= 0` is exactly a single `wrap(text, avail)` call, so a paragraph with no first-line
/// indent (the common case) is byte-identical to plain wrapping.
fn wrap_first_line_indent(
    layout: &dyn TextLayout,
    text: &str,
    avail: f64,
    first: f64,
    font: &FontSpec,
) -> Vec<String> {
    if first <= 0.0 {
        return layout.wrap(text, avail, font);
    }
    // Break the first line at the narrowed width, then wrap the words it left behind at full width.
    // Greedy wrapping keeps word order, so the first line's word count is the split point.
    let first_line = layout
        .wrap(text, (avail - first).max(1.0), font)
        .into_iter()
        .next()
        .unwrap_or_default();
    let consumed = first_line.split_whitespace().count();
    let rest: Vec<&str> = text.split_whitespace().skip(consumed).collect();
    let mut lines = vec![first_line];
    if !rest.is_empty() {
        lines.extend(layout.wrap(&rest.join(" "), avail, font));
    }
    lines
}

/// A paragraph's own font: the first run that carries an explicit font override, mapped to a
/// [`FontSpec`]. `None` when every run inherits the object font (the common case), so the caller falls
/// back to the object-level font.
fn paragraph_font(p: &Paragraph) -> Option<FontSpec> {
    p.runs
        .iter()
        .find_map(|r| r.font.as_ref())
        .map(crate::font_of)
}

/// A paragraph's rigid character spacing: the first run that sets one. It is a property of a
/// literal-text run (a field run has no such member and always reads zero), and the wrapped lines a
/// paragraph produces are drawn as single runs, so the paragraph carries one value. Zero — the
/// overwhelming default — leaves the natural advances alone.
fn paragraph_character_spacing(p: &Paragraph) -> Twips {
    p.runs
        .iter()
        .map(|r| r.character_spacing)
        .find(|s| s.0 != 0)
        .unwrap_or_default()
}

/// The line pitch (top-to-top vertical advance) for a paragraph, in twips: the font's natural line
/// height scaled by a `Multiple` line spacing (1.0 = single, 1.5, 2.0), or the `Exact` twip pitch when
/// the spacing is exact. Floored at 1 so a line always advances.
fn line_pitch(spacing: LineSpacing, font: &FontSpec, layout: &dyn TextLayout) -> i32 {
    let natural = layout.line_height_twips(font);
    match spacing.multiple() {
        Some(m) => (natural * m).round() as i32,
        None => spacing.exact_twips().unwrap_or(natural as i32),
    }
    .max(1)
}

/// The band that closes an "Underlay Following Sections" span — the *companion* of the section that
/// opened it. Sections between the two draw over the underlay; the companion itself is not underlaid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UnderlayEnd {
    /// Opened by a Report Header section; closed by the Report Footer.
    ReportFooter,
    /// Opened by a Group Header section at this level; closed by the Group Footer of the same level.
    GroupFooter(usize),
}

/// An open underlay span: the y its section would have ended at, and the band that closes it.
#[derive(Debug)]
pub(crate) struct UnderlaySpan {
    bottom: i32,
    end: UnderlayEnd,
}

/// The running position of a multi-column detail flow: the current column, the shared/row top
/// (`col_top`), the in-column fill position (`y`, down-then-across), the tallest record in the
/// current across-row (`row_h`), the lowest point reached on the page (`deepest`), and whether the
/// page already carries body content (so a NewPageBefore / deferred break leaves no leading blank
/// page — the guard the single-column path gets from `cursor_y > 0`).
struct MultiColCursor {
    col: i32,
    col_top: i32,
    y: i32,
    row_h: i32,
    deepest: i32,
    dirty: bool,
}

impl MultiColCursor {
    fn new(top: i32, dirty: bool) -> Self {
        MultiColCursor {
            col: 0,
            col_top: top,
            y: top,
            row_h: 0,
            deepest: top,
            dirty,
        }
    }

    /// Reset the cursor to the top of a freshly-begun page.
    fn reset_to(&mut self, top: i32) {
        self.col = 0;
        self.col_top = top;
        self.y = top;
        self.row_h = 0;
        self.deepest = top;
    }
}

impl<'a> Formatter<'a> {
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
        use crate::resolve::{cond, cond_color, field_text_marked, text_display};
        // Set by the `Field` arm below when the field prints one currency symbol per page; the plan
        // carries it so the emit path can record a fixup without resolving the value again.
        let mut currency = None;
        // `reading_order` (Text/FieldHeading only) sets the paragraph's base direction; `paragraphs`
        // carries per-paragraph indentation for a text object (fields/headings have none).
        let (raw, font, color, kind, reading_order, paragraphs): (
            _,
            _,
            _,
            _,
            _,
            Option<&[Paragraph]>,
        ) = match &obj.kind {
            ReportObjectKind::Field(f) => (
                {
                    let (text, mark) = field_text_marked(
                        self.report,
                        f,
                        ctx,
                        state,
                        &self.locale,
                        &self.diagnostics,
                    );
                    currency = mark;
                    text
                },
                font_of(&f.font_color.font),
                cond_color(&f.font_color.condition_formulas, cond::FONT_COLOR, ctx)
                    .unwrap_or(f.font_color.color),
                ObjectKind::Field,
                ReadingOrder::LeftToRight,
                None,
            ),
            ReportObjectKind::Text(t) => (
                text_display(self.report, t, ctx, state, &self.locale, &self.diagnostics),
                font_of(&t.font_color.font),
                cond_color(&t.font_color.condition_formulas, cond::FONT_COLOR, ctx)
                    .unwrap_or(t.font_color.color),
                ObjectKind::Text,
                t.reading_order,
                Some(&t.paragraphs),
            ),
            // A field heading is a static column-label text object: its literal is stored (needing
            // no row), drawn with its own font/color, so it resolves like a text object.
            ReportObjectKind::FieldHeading(h) => (
                h.text.clone(),
                font_of(&h.font_color.font),
                cond_color(&h.font_color.condition_formulas, cond::FONT_COLOR, ctx)
                    .unwrap_or(h.font_color.color),
                ObjectKind::Text,
                h.reading_order,
                None,
            ),
            _ => return None,
        };
        // A requested family that can't render some char (e.g. a symbol like ⚠ in an Arial run) is
        // served by the bundled fallback face; record that once so a caller knows the render used a
        // substitute. ASCII always resolves in the Latin faces, so only scan non-ASCII text; the sink
        // de-dupes on the (family, chars) message, collapsing thousands of identical detail rows to one.
        if !raw.is_ascii() {
            let subbed = self.text_layout.substituted_chars(&raw, &font);
            if !subbed.is_empty() {
                use rpt_pages::{Diagnostic, DiagnosticKind};
                // Append each glyph's U+XXXX codepoint so the warning is greppable even when the
                // terminal/log can't render the symbol itself (e.g. "'⚠' (U+26A0)").
                let codepoints = subbed
                    .chars()
                    .map(|c| format!("U+{:04X}", c as u32))
                    .collect::<Vec<_>>()
                    .join(" ");
                crate::push_diag(
                    &self.diagnostics,
                    // No `source`: the message already names the family, and the CLI appends
                    // `source` as a trailing "(…)" meant for an *object* name — repeating the
                    // family there reads as if it were the fallback face (it is not).
                    Diagnostic::warn(
                        DiagnosticKind::FontSubstituted,
                        format!(
                            "font '{}' lacks glyph(s) '{subbed}' ({codepoints}); rendered from a fallback face",
                            font.family
                        ),
                    ),
                );
            }
        }
        let align = crate::resolved_align(self.report, obj, reading_order);
        // Explicit line breaks (a multi-line label like "Numeric\nCode") always split into separate
        // runs so every backend renders them as lines. Can-grow additionally word-wraps each line.
        // Each `\n`-segment is one paragraph, aligned by position with `paragraphs` to pick up its
        // per-paragraph font (a mixed-size text object renders each paragraph at its own size), line
        // spacing, and indentation: the left/right indent narrows the wrap width and shifts the run;
        // the first-line indent shifts only the paragraph's first wrapped line. All indents zero and a
        // single object-level font (the common case) gives `x_offset = 0`, full-width runs, and the
        // object font on every line — byte-identical to the pre-paragraph behaviour.
        // A quarter-turn rotation (90 / 270°) flows the text along the box's tall axis, so it wraps
        // against the box height and its lines stack across the width (the placement in `place` maps
        // each line to its rotated column). Upright text wraps against the width as usual.
        let rotated_quarter = obj.format.text_rotation.degrees() % 180 == 90;
        let body_w = if rotated_quarter {
            obj.bounds.height.0
        } else {
            obj.bounds.width.0
        };
        let mut lines: Vec<LayoutLine> = Vec::new();
        for (i, seg) in raw.split('\n').enumerate() {
            let para = paragraphs.and_then(|ps| ps.get(i));
            let indent = para.map(|p| p.indent).unwrap_or_default();
            let (left, right, first) = (
                indent.left_indent.0,
                indent.right_indent.0,
                indent.first_line_indent.0,
            );
            // The paragraph's own font (a run override, else the object font) drives its wrap width,
            // line pitch, and ascent.
            let para_font = para
                .and_then(paragraph_font)
                .unwrap_or_else(|| font.clone());
            let line_height = Twips(line_pitch(
                indent.line_spacing,
                &para_font,
                self.text_layout,
            ));
            let ascent = Twips(self.text_layout.ascent_twips(&para_font) as i32);
            let avail = (body_w - left - right).max(1);
            // A spaced paragraph must break at the width it is drawn at, so the wrap measures through
            // the same adjusted advance the emitted run reports. Unspaced text (the default) uses the
            // injected layout untouched.
            let spacing = para.map(paragraph_character_spacing).unwrap_or_default();
            let spaced;
            let wrap_layout: &dyn TextLayout = if spacing.0 == 0 {
                self.text_layout
            } else {
                spaced = crate::text::SpacedLayout::new(self.text_layout, spacing);
                &spaced
            };
            let wrapped: Vec<String> = if obj.format.can_grow && allow_grow {
                // The first-line indent narrows where the first line breaks, not just where it sits:
                // the first line wraps at `avail - first`, the rest at the paragraph's full `avail`.
                wrap_first_line_indent(wrap_layout, seg, avail as f64, first as f64, &para_font)
            } else {
                vec![seg.to_string()]
            };
            let last_line = wrapped.len().saturating_sub(1);
            for (j, text) in wrapped.into_iter().enumerate() {
                // The first wrapped line of the paragraph also carries the first-line indent.
                let x_offset = left + if j == 0 { first } else { 0 };
                let width = (body_w - x_offset - right).max(1);
                // Justified stretches every wrapped line to both edges except the paragraph's last,
                // which typography leaves ragged (flush-left).
                let line_align = if matches!(align, TextAlign::Justified) && j == last_line {
                    TextAlign::Left
                } else {
                    align
                };
                lines.push(LayoutLine {
                    text,
                    x_offset: Twips(x_offset),
                    width: Twips(width),
                    font: para_font.clone(),
                    line_height,
                    ascent,
                    align: line_align,
                    character_spacing: spacing,
                });
            }
        }
        if lines.is_empty() {
            lines.push(LayoutLine {
                text: String::new(),
                x_offset: Twips(0),
                width: obj.bounds.width,
                font: font.clone(),
                line_height: Twips(self.text_layout.line_height_twips(&font) as i32),
                ascent: Twips(self.text_layout.ascent_twips(&font) as i32),
                align,
                character_spacing: Twips(0),
            });
        }
        Some(TextPlan {
            lines,
            color,
            kind,
            currency,
        })
    }

    /// Emit one band (section) at the cursor, paginating first if it would overflow the body. The
    /// band grows vertically when a `can-grow` text object wraps to more lines than its box holds.
    ///
    /// Returns whether the band produced any output: `false` when it was suppressed (statically,
    /// conditionally, or as a blank section), which is what makes a detail record *invisible* for
    /// the "Records per page" cap.
    ///
    /// `underlay_end` names the companion band that would close an underlay span opened here (see
    /// [`Self::open_underlay`]); `None` for a band whose kind has no companion.
    pub(crate) fn emit_band(
        &mut self,
        section: &Section,
        row: Option<&Row>,
        state: &ResolveState,
        underlay_end: Option<UnderlayEnd>,
    ) -> bool {
        if section.format.base.suppress {
            return false;
        }
        // A Section_Visibility condition suppresses the whole band per record, like the static flag.
        // Evaluated on a probe context (the incoming record) so a suppressed band forces no page
        // break and fires no side effects, matching the static path above.
        if !section.condition_formulas.is_empty() {
            let empty = Row::default();
            let probe = self.context(row.unwrap_or(&empty), state);
            if crate::resolve::cond_bool(
                &section.condition_formulas,
                crate::resolve::cond::SECTION_VISIBILITY,
                Some(&probe),
            )
            .unwrap_or(false)
            {
                return false;
            }
        }
        // NewPageBefore on this band, or a deferred NewPageAfter from the previous band, starts a
        // fresh page — but not when we are already at the top of one (that would leave a blank page).
        if (self.pending_page_break || section.format.base.new_page_before) && self.cursor_y > 0 {
            self.finish_page();
            self.begin_page();
        }
        self.pending_page_break = false;

        // Always resolve against a record: a band with none in scope (the report/page header/footer
        // of an empty dataset) still lays out its static skeleton, with field references falling to
        // null against a synthetic empty row.
        let empty = Row::default();
        let ctx = Some(self.context(row.unwrap_or(&empty), state));
        // Format inline subreports once, ahead of pagination, so a subreport taller than its box grows
        // the band (its Shared/Global writes fire here, exactly once — the cache is reused at emit).
        let subs = self.run_band_subreports(section, ctx.as_ref());
        // A body band (Detail / Group / Report Header-Footer) is a flow section: can-grow applies.
        // `text_height` is the band's height from its own (non-subreport) content; `height` also fits
        // any inline subreport that lands atomically.
        let (plans, text_height) = self.band_plans_and_height(section, ctx.as_ref(), state, true);
        let height = self.grow_for_subreports(section, &subs, text_height);
        // Suppress If Blank: a section that resolved to no visible output is dropped and reserves no
        // vertical space, so it does not push following bands down or force an extra page. Its
        // formulas have already evaluated (above), so their record-time side effects still fire.
        if section.format.suppress_if_blank && self.section_is_blank(section, &plans, ctx.as_ref())
        {
            return false;
        }
        // Route to the cross-page-flow path only when an inline subreport can't land atomically on a
        // page at all (see [`Self::try_emit_flowing_subreport`]).
        if self.try_emit_flowing_subreport(
            section,
            &plans,
            text_height,
            height,
            ctx.as_ref(),
            &subs,
        ) {
            // The flow path emitted the band and advanced the cursor past the last slice.
        } else {
            // PrintAtBottomOfPage: pin the band (a group/report footer) against the bottom of the body,
            // above the page footer — like the page-footer pin, but for a flow band. It never rises
            // above the current cursor (a band already low stays where it is and overflows normally).
            //
            // A band never splits mid-section: if it would overflow the body, move the whole band to a
            // new page (this also satisfies section-level KeepTogether for a single band).
            if self.cursor_y + height > self.body_bottom && self.cursor_y > 0 {
                self.finish_page();
                self.begin_page();
            }
            let pin_bottom = section.format.base.print_at_bottom_of_page;
            let origin_y = if pin_bottom {
                (self.body_bottom - height).max(self.cursor_y)
            } else {
                self.cursor_y
            };
            self.emit_band_plans(section, &plans, origin_y, height, ctx.as_ref(), &subs, None);
            if pin_bottom {
                // The band consumed the rest of the page: the next flow band starts a fresh page.
                self.cursor_y = self.body_bottom;
            } else if section.format.underlay_section {
                self.open_underlay(origin_y + height, underlay_end);
            } else {
                self.cursor_y += height;
            }
        }

        // NewPageAfter: defer the break to the next flow band so a trailing one adds no blank page.
        if section.format.base.new_page_after {
            self.pending_page_break = true;
        }
        // ResetPageNumberAfter: restart the page-number counter at the next page top.
        if section.format.base.reset_page_number_after {
            self.pending_page_number_reset = true;
        }
        true
    }

    /// Open an "Underlay Following Sections" span for a band just emitted at `bottom - height`.
    ///
    /// The band is a background for the sections that follow it: it draws first (so painter's order
    /// puts its ops underneath) and does not advance the cursor, so the following sections cover the
    /// same vertical region — across area boundaries — instead of being pushed below it. The span
    /// ends at the section's **companion** band, which is not underlaid: reaching it drops the cursor
    /// to `bottom` (see [`Self::close_underlay_spans`]). A band with no companion (`end` is `None` —
    /// a detail, a footer, or a page header, whose companion page footer is pinned to the body bottom
    /// anyway) simply underlays the rest of the page.
    fn open_underlay(&mut self, bottom: i32, end: Option<UnderlayEnd>) {
        if let Some(end) = end {
            self.underlay_spans.push(UnderlaySpan { bottom, end });
        }
    }

    /// Close every underlay span that `end` is the companion of, dropping the cursor to the lowest
    /// point they reached. A span that ended above the cursor leaves it alone — the overlaying
    /// content already ran past the underlay.
    pub(crate) fn close_underlay_spans(&mut self, end: UnderlayEnd) {
        let mut floor = self.cursor_y;
        self.underlay_spans.retain(|span| {
            if span.end == end {
                floor = floor.max(span.bottom);
                false
            } else {
                true
            }
        });
        self.cursor_y = floor;
    }

    /// Flow an inline subreport across pages when it can't land atomically on one — it produced
    /// several child pages (each needing its own parent page) or a single chunk taller than the whole
    /// body (so even a fresh page can't hold it). Emits the band's own content once at the current
    /// cursor, then places the subreport separately below (splitting it across pages), leaving the
    /// cursor beneath the last slice. Returns `true` when it took this path; `false` (the atomic
    /// case — the subreport fits a page, so it moves whole to the next page if the space left is too
    /// small) leaves the band for the caller's normal emit. Shared by [`Self::emit_band`] and the
    /// report-header path so a tall subreport flows identically wherever it sits.
    fn try_emit_flowing_subreport(
        &mut self,
        section: &Section,
        plans: &[Option<TextPlan>],
        text_height: i32,
        height: i32,
        ctx: Option<&DataContext>,
        subs: &[Option<crate::SubreportRender>],
    ) -> bool {
        let sub_idx = subs.iter().position(Option::is_some);
        let needs_flow = sub_idx.is_some_and(|i| {
            subs[i]
                .as_ref()
                .is_some_and(|r| r.chunks.len() > 1 || height > self.body_bottom)
        });
        if !needs_flow {
            return false;
        }
        let i = sub_idx.expect("needs_flow implies a subreport index");
        let origin_y = self.cursor_y;
        // The band's own content (section background + other objects) emits once, at the current
        // cursor; the subreport is placed separately below, flowing across pages.
        self.emit_band_plans(section, plans, origin_y, text_height, ctx, subs, Some(i));
        let rect = self.page_rect(&section.objects[i].bounds, origin_y);
        let chunks = &subs[i].as_ref().expect("subreport at sub_idx").chunks;
        self.place_subreport_flowing(chunks, rect);
        true
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
                    // Sum each line's own pitch: mixed paragraph fonts and 1.5/double/exact line
                    // spacing make lines unequal, so the grown height is their total, not count × one.
                    let text_h: i32 = p.lines.iter().map(|l| l.line_height.0).sum();
                    height = height.max(o.bounds.top.0 + text_h);
                }
            }
        }
        (plans, height)
    }

    /// Whether a suppress-if-blank section would draw nothing: every object is either suppressed or an
    /// empty text/field with no border or fill. Any shape, picture, blob, chart, cross-tab, subreport,
    /// or non-empty text makes the section non-blank — the engine's "Suppress Blank Section" rule. A
    /// visible border/fill or a conditional-format formula on an otherwise-empty box counts as output,
    /// so such a section is kept (the conservative direction: only the plainly-empty band is dropped).
    fn section_is_blank(
        &self,
        section: &Section,
        plans: &[Option<TextPlan>],
        ctx: Option<&DataContext>,
    ) -> bool {
        use crate::resolve::{cond, cond_bool};
        section.objects.iter().enumerate().all(|(i, obj)| {
            // A statically- or conditionally-suppressed object draws nothing.
            let suppressed =
                cond_bool(&obj.format.condition_formulas, cond::OBJECT_VISIBILITY, ctx)
                    .unwrap_or(obj.format.suppress.value);
            if suppressed {
                return true;
            }
            match &obj.kind {
                ReportObjectKind::Field(_)
                | ReportObjectKind::Text(_)
                | ReportObjectKind::FieldHeading(_) => {
                    let border = &obj.border;
                    let has_edge = [border.top, border.bottom, border.left, border.right]
                        .iter()
                        .any(|s| !matches!(s, rpt_model::LineStyle::NoLine));
                    // A transparent or plain-white fill is default decoration, not content — only a
                    // visible (opaque, non-white) fill keeps the box.
                    let visible_fill = border
                        .background_color
                        .is_some_and(|c| c.a > 0 && (c.r, c.g, c.b) != (255, 255, 255));
                    if has_edge || visible_fill || !border.condition_formulas.is_empty() {
                        return false;
                    }
                    plans
                        .get(i)
                        .and_then(Option::as_ref)
                        .map(|p| p.lines.iter().all(|l| l.text.trim().is_empty()))
                        .unwrap_or(true)
                }
                // Shapes, images, charts, cross-tabs, blobs, and subreports always draw.
                _ => false,
            }
        })
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
        let body_height = self.body_bottom;
        if height <= body_height && self.cursor_y + height > self.body_bottom && self.cursor_y > 0 {
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
        // A hierarchical subtree prints inside this instance's header/footer bracket, so it is part of
        // the group's height for Keep Group Together.
        for child in &g.hierarchy_children {
            height += self.measure_group_height(child);
        }
        if let Some(ftr) = self.bands.group_footers.get(g.level) {
            height += band_height(ftr);
        }
        height
    }

    /// The left indent one level of a hierarchically grouped tree adds to every object in the group's
    /// bands ("Group Indent", stored per group). `0` for a group without hierarchical sorting.
    fn hierarchy_indent_step(&self, level: usize) -> i32 {
        self.report
            .data_definition
            .groups
            .get(level)
            .and_then(|g| g.hierarchical_options.as_ref())
            .filter(|h| h.enabled)
            .map_or(0, |h| h.group_indent.0)
    }

    /// Whether the current page already holds the Detail area's full quota of visible records
    /// ("Records per page"). A stored `0` is the designer's unchecked state — no limit.
    fn record_limit_reached(&self) -> bool {
        let limit = self.bands.records_per_page;
        limit > 0 && self.records_on_page >= limit
    }

    /// Whether the current page already holds this group level's full quota of group instances
    /// ("Groups per page"). A stored `0` is the designer's unchecked state — no limit.
    fn group_limit_reached(&self, level: usize) -> bool {
        let limit = self
            .bands
            .group_formats
            .get(level)
            .map_or(0, |f| f.visible_groups_per_page);
        limit > 0 && self.groups_on_page.get(level).copied().unwrap_or(0) >= limit
    }

    /// Break to a fresh page when the current one has no quota left for the group about to start —
    /// either its own level's "Groups per page" cap, or the Detail area's "Records per page" cap,
    /// which the engine applies here too rather than orphaning the group header on a full page.
    fn break_for_paging_limits(&mut self, level: usize) {
        if (self.group_limit_reached(level) || self.record_limit_reached()) && self.cursor_y > 0 {
            self.finish_page();
            self.begin_page();
        }
    }

    pub(crate) fn emit_group(&mut self, g: &'a GroupInstance) {
        self.break_for_paging_limits(g.level);
        self.keep_group_together(g);
        // Counted after every break this group may force, since opening a page rebuilds the counters.
        if let Some(count) = self.groups_on_page.get_mut(g.level) {
            *count += 1;
        }
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
        state.summaries = Rc::new(g.summaries.clone());
        // Group header for this level.
        if let Some(hdr) = self.bands.group_headers.get(g.level).cloned() {
            let first = g.details.first().or_else(|| first_row(g));
            for s in hdr {
                self.emit_band(s, first, &state, Some(UnderlayEnd::GroupFooter(g.level)));
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
        // Hierarchically grouped children print between this instance's own content and its footer —
        // the group's header/footer bands therefore bracket the whole subtree — each one step further
        // indented ("Group Indent"). See [`Self::hierarchy_indent_step`].
        if !g.hierarchy_children.is_empty() {
            let base = self.col_offset;
            self.col_offset = base + self.hierarchy_indent_step(g.level);
            for child in &g.hierarchy_children {
                self.emit_group(child);
            }
            self.col_offset = base;
        }
        // Group footer for this level: resolved against the group's last record (Crystal's
        // group-footer record context), so its field/formula objects populate. It is this level's
        // underlay companion, so any span its header opened closes first — the footer prints below
        // the underlay, not over it.
        self.close_underlay_spans(UnderlayEnd::GroupFooter(g.level));
        if let Some(ftr) = self.bands.group_footers.get(g.level).cloned() {
            let last = g.details.last().or_else(|| crate::last_row(g));
            for s in ftr {
                self.emit_band(s, last, &state, None);
            }
        }
        self.group_stack.pop();
        self.refresh_group_summaries();
    }

    /// Emit the group-band skeleton once for a report that defines groups but produced none (an empty
    /// dataset). The engine still lays out the group headers (outermost→innermost) and footers
    /// (innermost→outermost) a single time, so the static content authored into those bands — a
    /// letterhead, column labels, totals captions — renders instead of a blank page. Field/formula
    /// objects resolve against a synthetic empty row (falling to null), and no detail band is emitted
    /// (there is no record).
    pub(crate) fn emit_empty_group_skeleton(&mut self) {
        let state = self.state(None);
        let headers: Vec<Vec<&'a Section>> = self.bands.group_headers.clone();
        let footers: Vec<Vec<&'a Section>> = self.bands.group_footers.clone();
        for (level, hdr) in headers.iter().enumerate() {
            for s in hdr {
                self.emit_band(s, None, &state, Some(UnderlayEnd::GroupFooter(level)));
            }
        }
        for (level, ftr) in footers.iter().enumerate().rev() {
            self.close_underlay_spans(UnderlayEnd::GroupFooter(level));
            for s in ftr {
                self.emit_band(s, None, &state, None);
            }
        }
    }

    pub(crate) fn emit_details(&mut self, rows: &'a [Row], summaries: &[rpt_data::Summary]) {
        let detail_bands: Vec<&Section> = self.bands.detail.clone();
        if let Some(mc) = self.multi_column {
            if mc.columns > 1 {
                self.emit_details_multicol(rows, summaries, &detail_bands, mc);
                return;
            }
        }
        // Group-constant: share one Rc across every detail row instead of deep-cloning per record.
        let summaries = Rc::new(summaries.to_vec());
        for row in rows {
            // "Records per page": a page holding its full quota breaks before the next record, so
            // the cap wins over the space still left on the page.
            if self.record_limit_reached() && self.cursor_y > 0 {
                self.finish_page();
                self.begin_page();
            }
            self.record_number += 1;
            self.current_row = Some(row);
            let mut state = self.state(None);
            state.summaries = Rc::clone(&summaries);
            // Advance running totals in print order before emitting the band, so a `{#name}` object
            // shows the value accumulated up to and including this record.
            self.advance_running_totals(row, &state);
            let mut visible = false;
            for s in &detail_bands {
                visible |= self.emit_band(s, Some(row), &state, None);
            }
            // Only a record that printed counts against the cap ("*Visible* records per page").
            if visible {
                self.records_on_page += 1;
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
        rows: &'a [Row],
        summaries: &[rpt_data::Summary],
        bands: &[&Section],
        mc: rpt_model::MultiColumn,
    ) {
        let cols = mc.columns.max(1) as i32;
        let pitch = mc.column_width.0 + mc.gap_h.0;
        let across = mc.across_then_down;
        // Group-constant: share one Rc across every detail row instead of deep-cloning per record.
        let summaries = Rc::new(summaries.to_vec());
        let mut cur = MultiColCursor::new(self.cursor_y, self.cursor_y > 0);
        for row in rows {
            self.record_number += 1;
            self.current_row = Some(row);
            let mut state = self.state(None);
            state.summaries = Rc::clone(&summaries);
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
            // A "Records per page" cap breaks before the record it has no room for, exactly as in the
            // single-column path.
            if (self.pending_page_break || new_page_before || self.record_limit_reached())
                && cur.dirty
            {
                self.finish_page();
                self.begin_page();
                cur.reset_to(self.cursor_y);
            }
            self.pending_page_break = false;

            let record_top = if across {
                self.place_across(&mut cur, rec_h)
            } else {
                self.place_down(&mut cur, rec_h, cols)
            };

            // The column offset composes with whatever offset the enclosing band already carries (a
            // hierarchical group's indent), so it is added to it and restored, not assigned.
            let base = self.col_offset;
            self.col_offset = base + cur.col * pitch;
            let mut band_y = record_top;
            for (s, plans, h) in &banded {
                self.emit_band_plans(s, plans, band_y, *h, Some(&ctx), &[], None);
                band_y += *h;
            }
            self.col_offset = base;
            cur.deepest = cur.deepest.max(band_y);
            // Only a record that printed counts against the cap ("*Visible* records per page").
            if !banded.is_empty() {
                self.records_on_page += 1;
            }

            if across {
                cur.row_h = cur.row_h.max(band_y - cur.col_top);
                cur.col += 1;
                if cur.col >= cols {
                    cur.col = 0;
                    cur.col_top += cur.row_h + mc.gap_v.0;
                    cur.row_h = 0;
                    self.cursor_y = cur.col_top;
                }
            } else {
                cur.y = band_y + mc.gap_v.0;
            }
            cur.dirty = true;
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
            if cur.col != 0 {
                self.cursor_y = cur.col_top + cur.row_h;
            }
        } else {
            self.cursor_y = cur.deepest;
        }
    }

    /// Across-then-down record placement: the page break is decided at the start of a column-row
    /// (`col == 0`) so a row never splits across a page. Returns the row's shared top.
    fn place_across(&mut self, cur: &mut MultiColCursor, rec_h: i32) -> i32 {
        if cur.col == 0 && cur.col_top + rec_h > self.body_bottom && cur.col_top > 0 {
            self.finish_page();
            self.begin_page();
            cur.col_top = self.cursor_y;
        }
        cur.col_top
    }

    /// Down-then-across record placement: overflowing the column advances to the next column, and
    /// overflowing the last column starts a new page. Returns this record's top within its column.
    fn place_down(&mut self, cur: &mut MultiColCursor, rec_h: i32, cols: i32) -> i32 {
        if cur.y + rec_h > self.body_bottom && cur.y > cur.col_top {
            cur.col += 1;
            if cur.col >= cols {
                self.finish_page();
                self.begin_page();
                cur.col = 0;
                cur.col_top = self.cursor_y;
                cur.deepest = cur.col_top;
            }
            cur.y = cur.col_top;
        }
        cur.y
    }

    /// Open a fresh page: advance the page number (honouring a pending reset), start a new [`Page`],
    /// reset the cursor, and record its checkpoint. The top-of-page bands (report/page header) are
    /// emitted separately so the report header — a page-1 flow band that may itself paginate a tall
    /// subreport — does not recurse through page opening.
    pub(crate) fn open_page(&mut self) {
        // A section with ResetPageNumberAfter set the counter to restart here: the new page prints
        // as page 1 (the increment below lands on 1 from 0).
        if self.pending_page_number_reset {
            self.page_number = 0;
            self.pending_page_number_reset = false;
        }
        self.page_number += 1;
        self.cur = Page::new(self.page_number as u32, self.page_size);
        self.cur.origin = self.origin;
        self.cursor_y = 0;
        // An underlay section is drawn once, on the page it lands on; a span left open by a page
        // turn has nothing left to back, so the new page starts with none.
        self.underlay_spans.clear();
        // Both paging caps are per-page. The record count starts over; the group counts start at 1 for
        // every level the page opens *inside*, because a group carried over by a break still occupies
        // one of the new page's group slots.
        self.records_on_page = 0;
        let depth = self.group_stack.len();
        for (level, count) in self.groups_on_page.iter_mut().enumerate() {
            *count = i32::from(depth > level);
        }
        self.checkpoints.push(PageCheckpoint {
            page_number: self.page_number as u32,
            record_position: self.record_number as u64,
            state: Default::default(),
        });
    }

    /// Emit the page header (the repeating top-of-page band) at the current cursor. Resolves against
    /// the record at the top of this page (the first record it will carry): before any record has
    /// printed this is the report's first record, otherwise the record now in scope.
    fn emit_page_header(&mut self) {
        let page_row = self
            .current_row
            .or_else(|| self.dataset.iter_detail_rows().first().copied());
        let ph: Vec<&Section> = self.bands.page_header.clone();
        for s in ph {
            let state = self.state(None);
            // Page Header is a fixed repeating band — can-grow is inert.
            self.emit_band_no_paginate(s, page_row, &state, false);
        }
    }

    /// Open a continuation page and emit its repeating page header. Every page turn *after* the first
    /// goes through here; the first page is opened by [`Self::run`], which emits the report header
    /// above the page header. (The report header is page-1-only, so it never belongs on a page turn.)
    ///
    /// A page turn taken *inside* the report header carries no page header: the page header sits
    /// below the report header, so it does not print on any page the header occupies.
    pub(crate) fn begin_page(&mut self) {
        self.open_page();
        if !self.in_report_header {
            self.emit_page_header();
        }
    }

    /// Emit the report header once, at the very top of page 1, above the page header (Crystal's
    /// top-of-page band order). A report header is a flow section: a can-grow object extends it, and
    /// an inline subreport taller than a page flows across continuation pages. The page header sits
    /// *below* the report header, so it prints on none of the pages the header occupies — it is
    /// emitted once here, at the cursor the header leaves behind, and repeats from the next page turn
    /// on. For a header that fits, that cursor is partway down page 1; for one that flows, it is the
    /// top of the first page the body reaches.
    pub(crate) fn emit_report_header(&mut self) {
        // The report header resolves against the report's first record (Crystal's report-header
        // record context) so its field/formula objects populate.
        let first = self.dataset.iter_detail_rows().first().copied();
        let rh: Vec<&Section> = self.bands.report_header.clone();
        self.in_report_header = true;
        for s in rh {
            let state = self.state(None);
            if s.format.base.suppress {
                continue;
            }
            let empty = Row::default();
            let ctx = Some(self.context(first.unwrap_or(&empty), &state));
            let subs = self.run_band_subreports(s, ctx.as_ref());
            let (plans, text_height) = self.band_plans_and_height(s, ctx.as_ref(), &state, true);
            let height = self.grow_for_subreports(s, &subs, text_height);
            if !self.try_emit_flowing_subreport(s, &plans, text_height, height, ctx.as_ref(), &subs)
            {
                // Fits on the page: emit at the cursor, growing the band to any inline subreport's
                // content height (the atomic detail-band behavior) instead of clipping it to the
                // placeholder box. Byte-identical to the plain emit when the band has no subreport —
                // `grow_for_subreports` returns `text_height` and the empty `subs` draws nothing extra.
                let origin_y = self.cursor_y;
                self.emit_band_plans(s, &plans, origin_y, height, ctx.as_ref(), &subs, None);
                if s.format.underlay_section {
                    self.open_underlay(origin_y + height, Some(UnderlayEnd::ReportFooter));
                } else {
                    self.cursor_y += height;
                }
            }
        }
        self.in_report_header = false;
        self.emit_page_header();
    }

    pub(crate) fn finish_page(&mut self) {
        // Pin the page footer to the bottom. It resolves against the last record printed on this page
        // (Crystal's page-footer record context).
        let page_row = self.current_row;
        let pf: Vec<&Section> = self.bands.page_footer.clone();
        self.cursor_y = self.body_bottom;
        for s in pf {
            let state = self.state(None);
            // Page Footer is a fixed repeating band — can-grow is inert (it must fit the space
            // reserved at the page bottom).
            self.emit_band_no_paginate(s, page_row, &state, false);
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
        let empty = Row::default();
        let ctx = Some(self.context(row.unwrap_or(&empty), state));
        let (plans, height) = self.band_plans_and_height(section, ctx.as_ref(), state, allow_grow);
        let origin_y = self.cursor_y;
        self.emit_band_plans(section, &plans, origin_y, height, ctx.as_ref(), &[], None);
        // A page header/footer has no underlay companion that can close the span: the page footer is
        // pinned to the body bottom, so an underlaid page header backs the whole page (see
        // [`Self::open_underlay`]).
        if section.format.underlay_section {
            self.open_underlay(origin_y + height, None);
        } else {
            self.cursor_y += height;
        }
    }
}
