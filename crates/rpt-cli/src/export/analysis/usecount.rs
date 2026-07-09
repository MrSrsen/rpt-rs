//! `Field.UseCount` derivation: how many times each database field is referenced across a report
//! level's objects, formulas, groups, sorts, running totals, and chart/cross-tab grid bindings.

use super::add_formula_mentions;
use super::font_color_of;
use super::formula;
use super::summary_fields::brace_groups;
use super::summary_fields::summary_fields;
use rpt::model::{FieldKindData, Report, ReportObjectKind, SubreportLink};
use std::collections::{HashMap, HashSet};

/// Count how many times each of this report level's database fields is *used*, the way the engine's
/// `UseCount` does: one per field object that displays it, one per formula / selection body that
/// names it (regardless of how many times), one per distinct placed summary of it, plus group, sort
/// and running-total references. Subreports are counted at their own level (see [`field_use_counts`]).
pub fn field_use_counts(report: &Report, incoming_links: &[SubreportLink]) -> HashMap<String, i32> {
    // The known database-field references (`{alias.field}`).
    let field_refs: Vec<String> = report
        .database
        .tables
        .iter()
        .flat_map(|t| &t.data_fields)
        .map(|f| format!("{{{}}}", f.long_name.as_deref().unwrap_or(&f.name)))
        .collect();

    let mut counts: HashMap<String, i32> = HashMap::new();
    count_report(&mut counts, &field_refs, report);
    // A subreport's own field references are counted in *its* UseCount (a subreport has its own
    // field schema, even when a table alias collides with the main report's). A *main*-report field
    // exported into one of this level's subreports via a link counts here — one use per link.
    for s in &report.subreports {
        for link in &s.links {
            let key = format!("{{{}}}", link.main_report_field);
            if field_refs.contains(&key) {
                *counts.entry(key).or_default() += 1;
            }
        }
    }
    // Symmetrically, a field this level *imports* from its parent via an incoming subreport link is
    // referenced by the link itself — one extra use of the bound subreport field. For a db-field main
    // link the bound field is recovered from the auto-generated record-selection clause. For a
    // parameter/formula main link (which has no such clause naming a db field) the bound field is the
    // decoded `SubreportFieldName` on `link.subreport_field`; count it when it is a database field
    // (not an `@formula`/`?param`).
    for link in incoming_links {
        if let Some(key) = subreport_link_field(report, &field_refs, &link.main_report_field) {
            *counts.entry(key).or_default() += 1;
        } else if let Some(key) = link_subreport_field_ref(&field_refs, &link.subreport_field) {
            *counts.entry(key).or_default() += 1;
        }
    }
    counts
}

/// The subreport database field bound by an incoming link whose main-report field is `main_field`.
/// When the user links a *database field* main→sub, Crystal auto-names the link parameter
/// `Pm-<main_field>` and generates the record-selection clause `{sub.field} = {?Pm-<main_field>}`;
/// the bound subreport field is the database-field reference on the left of that equality. Only this
/// auto-generated equality form is counted. A parameter (`?…`) or formula (`@…`) main field binds to
/// a subreport parameter/formula, not a database field, so it is skipped; likewise a non-equality
/// clause (`{some_col} >= {?Pm-…}`) or a wrapped one (`= ToNumber({?Pm-…})`) is a user filter
/// referencing the parameter, not the link's bound field. Returns `None` when no such equality clause
/// identifies a database field, so an ambiguous link adds nothing rather than mis-attributing a use.
fn subreport_link_field(
    report: &Report,
    field_refs: &[String],
    main_field: &str,
) -> Option<String> {
    if main_field.starts_with('?') || main_field.starts_with('@') || !main_field.contains('.') {
        return None;
    }
    let body = &report.data_definition.record_selection.as_ref()?.0;
    // The main field's column name (after the last `.`) — used to confirm a non-equality clause is
    // the link's auto-generated comparison on the same column, not an unrelated user filter.
    let main_col = main_field.rsplit('.').next().unwrap_or(main_field);
    let param = format!("{{?Pm-{main_field}}}");
    for (idx, _) in body.match_indices(&param) {
        let before = body[..idx].trim_end();
        // Strip the comparison operator immediately before `{?Pm-…}`. The auto-generated equality
        // form uses `=`; a Boolean/range-filtered subreport link uses `<`, `<=`, `>`, `>=`.
        let (lhs, is_equality) = if let Some(l) = before.strip_suffix('=') {
            // `=` may be the tail of `<=`/`>=`/`<>`/`!=`/`==`; treat those as non-equality.
            match l.chars().last() {
                Some(c @ ('<' | '>' | '!' | '=')) => (l.trim_end_matches(c).trim_end(), false),
                _ => (l.trim_end(), true),
            }
        } else if let Some(l) = before
            .strip_suffix('<')
            .or_else(|| before.strip_suffix('>'))
        {
            (l.trim_end(), false)
        } else {
            continue;
        };
        // The left operand must be a database-field reference (`…{table.field}`).
        let Some(fr) = field_refs.iter().find(|fr| lhs.ends_with(fr.as_str())) else {
            continue;
        };
        // The equality form (`{sub.f} = {?Pm-main}`) is the canonical auto-generated link — accept
        // its LHS unconditionally. A non-equality form is only the link's bound field when its column
        // matches the main field's column (Crystal names the link parameter `Pm-<main>` and compares
        // the *same* column on the subreport side); otherwise it is a user filter that merely re-uses
        // the link parameter (a different column compared against `{?Pm-…}`), which must not attribute
        // a use.
        if is_equality || fr.rsplit('.').next().map(|c| c.trim_end_matches('}')) == Some(main_col) {
            return Some(fr.clone());
        }
    }
    None
}

/// Match a decoded `SubreportFieldName` (on `SubreportLink.subreport_field`) to one of this level's
/// database field references for `UseCount`. Returns `None` for an empty SF, a formula (`@…`) or a
/// parameter (`?…`) — only a database field contributes to a db field's count. The SF is the
/// unqualified-or-`table.field` form; it is matched to `field_refs` (`{alias.field}`) exactly, else by
/// its final `.field` segment (the alias may differ).
fn link_subreport_field_ref(field_refs: &[String], subreport_field: &str) -> Option<String> {
    if subreport_field.is_empty() || subreport_field.starts_with(['@', '?']) {
        return None;
    }
    let want = format!("{{{subreport_field}}}");
    if let Some(fr) = field_refs.iter().find(|fr| **fr == want) {
        return Some(fr.clone());
    }
    let field = subreport_field
        .rsplit('.')
        .next()
        .unwrap_or(subreport_field);
    let suffix = format!(".{field}}}");
    let bare = format!("{{{field}}}");
    field_refs
        .iter()
        .find(|fr| fr.ends_with(&suffix) || **fr == bare)
        .cloned()
}

/// Accumulate the `UseCount` contributions of one report (main report or a subreport): display and
/// summary field objects, group conditions, sorts, running totals, the bodies of *placed* formulas
/// and the selection formulas, and every conditional-format formula on its sections/objects/fonts.
fn count_report(counts: &mut HashMap<String, i32>, field_refs: &[String], r: &Report) {
    // Fields referenced as a displayed value — used to gate the `GroupName({field})` group-selector
    // exclusion in `bump_body` (see `displayed_value_fields`).
    let value_fields = displayed_value_fields(r, field_refs);
    // The deduplicated summary list is used in three places below (twice per summary-sorted group);
    // it walks + sorts every section, so compute it once.
    let summaries = summary_fields(r);
    for area in &r.report_definition.areas {
        for sec in &area.sections {
            for (_, body) in &sec.condition_formulas {
                bump_body(counts, field_refs, &value_fields, body);
            }
            for obj in &sec.objects {
                // A field object displaying the field directly. A summary object's references are
                // counted once per *distinct* summary definition below (not per placement), so they
                // are skipped here.
                if let ReportObjectKind::Field(f) = &obj.kind {
                    if field_refs.contains(&f.data_source) {
                        *counts.entry(f.data_source.clone()).or_default() += 1;
                    }
                }
                // A blob-field object (image/blob DB field rendered as a picture) references its
                // bound database field — one use per placed object, like a direct field object.
                if let ReportObjectKind::BlobField(b) = &obj.kind {
                    if field_refs.contains(&b.data_source) {
                        *counts.entry(b.data_source.clone()).or_default() += 1;
                    }
                }
                // Database fields embedded inside a text object (each counts once toward the field).
                if let ReportObjectKind::Text(t) = &obj.kind {
                    for ef in &t.embedded_fields {
                        if ef.starts_with('@') || ef.starts_with('?') || !ef.contains('.') {
                            continue;
                        }
                        let key = format!("{{{ef}}}");
                        if field_refs.contains(&key) {
                            *counts.entry(key).or_default() += 1;
                        }
                    }
                }
                for (_, body) in &obj.format.condition_formulas {
                    bump_body(counts, field_refs, &value_fields, body);
                }
                // Border conditional-format formulas (a box/line border's BackgroundColor /
                // BorderColor condition formulas, stored on `obj.border`). Each is a distinct
                // persistent conditional-format formula; every database field it names contributes one
                // use, the same per-attribute rule as object/section condition formulas.
                for (_, body) in &obj.border.condition_formulas {
                    bump_body(counts, field_refs, &value_fields, body);
                }
                if let Some(fc) = font_color_of(obj) {
                    for (_, body) in &fc.condition_formulas {
                        bump_body(counts, field_refs, &value_fields, body);
                    }
                }
            }
        }
    }

    // Running-total condition formulas (a running total's evaluate/reset condition). Each is a
    // distinct persistent formula the engine holds; every database field it names contributes one to
    // that field's UseCount. They are not attached to any section/object, so they are scanned here
    // (not via the object loop) and do not double-count.
    for body in &r.data_definition.running_total_condition_formulas {
        bump_body(counts, field_refs, &value_fields, body);
    }

    // Section/object conditional-format formulas the report-definition decode did not attach to any
    // section/object (a decode gap: such a body lands only in the flat `condition_formula_bodies`,
    // with an empty `SectionAreaConditionFormulas`). The engine still holds these as persistent
    // conditional-format formulas, so their DB-field references count. `condition_formula_bodies` is a
    // *superset* of the attached condition formulas (the attachment resolves the same body by index),
    // so scan only the bodies not already counted via an attached section/object/border/font condition
    // formula or the running-total condition list — otherwise the attached ones double-count. The flat
    // list stores the same body in several formatting variants (a leading newline, a trailing `;`), so
    // match on a normalized form (trimmed, trailing `;` removed) — an exact-string dedup misses the
    // variant copies and double-counts.
    let norm_cond = |s: &str| s.trim().trim_end_matches(';').trim().to_string();
    let mut attached: HashSet<String> = HashSet::new();
    for area in &r.report_definition.areas {
        for sec in &area.sections {
            for (_, b) in &sec.condition_formulas {
                attached.insert(norm_cond(b));
            }
            for obj in &sec.objects {
                for (_, b) in &obj.format.condition_formulas {
                    attached.insert(norm_cond(b));
                }
                for (_, b) in &obj.border.condition_formulas {
                    attached.insert(norm_cond(b));
                }
                if let Some(fc) = font_color_of(obj) {
                    for (_, b) in &fc.condition_formulas {
                        attached.insert(norm_cond(b));
                    }
                }
            }
        }
    }
    for b in &r.data_definition.running_total_condition_formulas {
        attached.insert(norm_cond(b));
    }
    let mut seen_unattached: HashSet<String> = HashSet::new();
    for body in &r.data_definition.condition_formula_bodies {
        let n = norm_cond(body);
        // Skip a body already counted via an attached/RT condition, and dedup the flat list's own
        // variant copies of the same unattached formula.
        if attached.contains(&n) || !seen_unattached.insert(n) {
            continue;
        }
        bump_body(counts, field_refs, &value_fields, body);
    }

    for g in &r.data_definition.groups {
        let key = format!("{{{}}}", g.condition_field);
        if field_refs.contains(&key) {
            // A group registers its DB condition field twice: as the group condition and as the
            // group sort (both from the same `0xe5`). The sort is normally counted via `record_sorts`
            // below. A Top N / Bottom N group sorts by a summary expression instead, so its
            // `record_sorts` entry no longer names the field — count the sort reference here when the
            // sort field differs.
            *counts.entry(key.clone()).or_default() += 1;
            // A summary-sorted group holds, on its condition field, the group-sort reference plus one
            // reference per group-scoped construct: the group-name definition, each placed GroupName
            // field object, and each group summary whose group-by (2nd) arg is this field. A plain
            // group does not reach here.
            if g.sort.field != g.condition_field {
                // The group-sort reference held on the condition field (condition + sort).
                *counts.entry(key.clone()).or_default() += 1;
                *counts.entry(key.clone()).or_default() += 1; // GroupName definition
                let gn_ds = format!("GroupName ({key})");
                let placed_group_names = r
                    .report_definition
                    .areas
                    .iter()
                    .flat_map(|a| &a.sections)
                    .flat_map(|s| &s.objects)
                    .filter(
                        |o| matches!(&o.kind, ReportObjectKind::Field(f) if f.data_source == gn_ds),
                    )
                    .count();
                let summary_suffix = format!(", {key})");
                let group_summaries = summaries
                    .iter()
                    .filter(|s| s.formula_name.ends_with(&summary_suffix))
                    .count();
                *counts.entry(key).or_default() += (placed_group_names + group_summaries) as i32;

                // The Top N sort basis is a summary (`g.sort.field`) referencing its summarized
                // (1st-arg) field. If the group displays that same summary it is already counted via
                // `summary_fields`; if it displays a different one (e.g. a percentage, while the sort
                // basis stays a plain `Sum`), that sort-basis summary is a separate, otherwise
                // uncounted reference → +1.
                let sort_summary = g.sort.field.as_str();
                let already_counted = summaries.iter().any(|s| s.formula_name == sort_summary);
                if !already_counted {
                    if let Some(first) = brace_groups(sort_summary).into_iter().next() {
                        if let Some(fr) = field_refs.iter().find(|fr| fr.as_str() == first) {
                            *counts.entry(fr.clone()).or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    for s in &r.data_definition.record_sorts {
        let key = format!("{{{}}}", s.field);
        if field_refs.contains(&key) {
            *counts.entry(key).or_default() += 1;
        }
    }

    // Saved-data grouping index. When a report is saved together with its data (a saved-data batch
    // is present in the file), the engine builds a persistent group index over the saved rows and
    // holds one extra reference to each group's condition field so the cached rows can be re-grouped
    // on load. That reference is not emitted anywhere in the XML, so each group's database condition
    // field is undercounted by one for every saved-data report. Add +1 per group whose condition
    // field is a database field. Gated on `has_saved_data` (the batch descriptor is present): a
    // report merely *configured* to save data but holding none builds no index and adds nothing,
    // matching the engine. This is a distinct reference from the group condition/sort counted above,
    // so it does not double-count.
    if r.has_saved_data {
        // A group field that is *also* bound as a chart/cross-tab grid dimension already carries the
        // engine's OLAP-grid index registration (counted via the grid multiplier); that
        // index subsumes the saved-data group index, so the saved-data reference is not additive for
        // it. Skip those to avoid double-counting.
        let grid_bound: HashSet<&str> = grid_binding_refs(r).collect();
        for g in &r.data_definition.groups {
            // Only a plain "for each value" group (sorted by its own condition field) receives the
            // saved-data index reference. A summary-sorted Top N / Bottom N group instead re-indexes
            // the saved rows by its summary expression, and its condition field is already fully
            // accounted for by the summary-sort branch above — adding the saved-data reference would
            // over-count it.
            if g.sort.field != g.condition_field {
                continue;
            }
            if grid_bound.contains(g.condition_field.as_str()) {
                continue;
            }
            let key = format!("{{{}}}", g.condition_field);
            if field_refs.contains(&key) {
                *counts.entry(key).or_default() += 1;
            }
        }
    }

    // Each distinct placed summary (deduplicated by definition) counts one use of its summarized
    // field when that field is a database field — the engine counts the summary definition, not each
    // of its placements.
    for s in &summaries {
        if let Some(first) = brace_groups(&s.formula_name).first() {
            if field_refs.iter().any(|fr| fr == first) {
                *counts.entry((*first).to_string()).or_default() += 1;
            }
        }
    }

    // A formula contributes its field references only if it is *used* — placed in a field object,
    // embedded in a text object, named by a conditional-format or selection formula, or transitively
    // referenced by another used formula. A defined-but-unused formula does not contribute.
    let live = live_formulas(r);

    for fd in &r.data_definition.field_definitions {
        match &fd.kind {
            FieldKindData::Formula(ff) if live.contains(&fd.name) => {
                bump_body(counts, field_refs, &value_fields, &ff.text.0);
            }
            FieldKindData::RunningTotal(rt) => {
                if !rt.summarized_field.is_empty() {
                    let key = format!("{{{}}}", rt.summarized_field);
                    if field_refs.contains(&key) {
                        *counts.entry(key).or_default() += 1;
                    }
                }
                // A running total with an `OnChangeOfField` evaluate/reset condition holds a
                // persistent reference to the field whose change drives it, counting +1 toward that
                // field. `OnChangeOfGroup`/`OnFormula` leave it empty.
                if !rt.on_change_field.is_empty() {
                    let key = format!("{{{}}}", rt.on_change_field);
                    if field_refs.contains(&key) {
                        *counts.entry(key).or_default() += 1;
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(f) = &r.data_definition.record_selection {
        bump_body(counts, field_refs, &value_fields, &f.0);
    }
    if let Some(f) = &r.data_definition.group_selection {
        bump_body(counts, field_refs, &value_fields, &f.0);
    }

    // Chart/cross-tab grid bindings reference their DB field a role-dependent number of times. A
    // formula binding instead makes that formula *live* (handled in `live_formulas`), which then
    // counts the formula's own references, so formula refs are skipped here.
    //
    //   * chart "show value" data field  → +1 (like a placed field object),
    //   * chart "on change of" category  → +2 per chart (internal group: condition + sort),
    //   * cross-tab row/column dimension → +3 per dimension (group condition + sort + OLAP-grid
    //     registration).
    //
    // Each chart/cross-tab object is visited separately, so a field used as a category/dimension in
    // several charts (or several cross-tab dimensions) accrues the multiplier once per occurrence.
    for obj in r
        .report_definition
        .areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|sec| &sec.objects)
    {
        match &obj.kind {
            ReportObjectKind::Chart(c) => {
                for f in &c.data_refs {
                    bump_grid(counts, field_refs, f, 1);
                }
                for f in &c.category_refs {
                    bump_grid(counts, field_refs, f, 2);
                }
                // A chart whose "on change of" category REUSES an existing report group writes no
                // dedicated grid group record (no `Grid #` `0xe5`), so neither its data nor its
                // category binding decodes — both ref lists are empty. The engine still registers the
                // chart against that group's field once (+1, an OLAP-grid-style registration on top of
                // the report group's own cond+sort). Attribute it to the report's first (outermost)
                // group's condition field, the category these reused-group charts bind. A chart with
                // its own grid group has non-empty refs and is handled above, so this never
                // double-counts a resolved chart.
                if c.data_refs.is_empty() && c.category_refs.is_empty() {
                    if let Some(g) = r.data_definition.groups.first() {
                        let key = format!("{{{}}}", g.condition_field);
                        if field_refs.contains(&key) {
                            // A reused-group chart adds +1 over a plain group. On a summary-sorted
                            // group it instead binds a full category (condition + sort) → +2.
                            let n = if g.sort.field != g.condition_field {
                                2
                            } else {
                                1
                            };
                            *counts.entry(key).or_default() += n;
                        }
                    }
                }
            }
            ReportObjectKind::CrossTab(c) => {
                for f in &c.field_refs {
                    bump_grid(counts, field_refs, f, 3);
                }
            }
            _ => {}
        }
    }

    // Dangling summary definitions. The data-definition region lists one `0x7e` summary binding per
    // `ISummaryField` (`summary_binding_fields`); the engine refcounts each summarized field. A
    // summary that is placed, or referenced by an aggregation in a live formula (`SUM({f})`), is
    // *already* counted above — its binding is that same reference, so adding it would double-count.
    // But a summary whose field is referenced **nowhere else** in the report is a dangling definition
    // with no other holder, which the engine still refcounts. So add the binding count only for a
    // field that has accrued **no** other use. This is purely additive for the otherwise-zero fields
    // and cannot move a field counted by any other path.
    let mut binding_count: HashMap<String, i32> = HashMap::new();
    for f in &r.data_definition.summary_binding_fields {
        let key = format!("{{{f}}}");
        if let Some(fr) = field_refs.iter().find(|fr| fr.eq_ignore_ascii_case(&key)) {
            *binding_count.entry(fr.clone()).or_default() += 1;
        }
    }
    for (key, bc) in binding_count {
        if counts.get(&key).copied().unwrap_or(0) == 0 {
            *counts.entry(key).or_default() += bc;
        }
    }
}

/// Add `n` uses of a chart/cross-tab DB-field binding `f` (raw `Table.field` form). Formula bindings
/// (`@name`) are skipped — they count via [`live_formulas`], not here.
fn bump_grid(counts: &mut HashMap<String, i32>, field_refs: &[String], f: &str, n: i32) {
    if f.starts_with('@') {
        return;
    }
    let key = format!("{{{f}}}");
    if field_refs.contains(&key) {
        *counts.entry(key).or_default() += n;
    }
}

/// Every chart / cross-tab field binding (raw engine reference form: `Table.field` or `@formula`)
/// placed at this report level, across all roles. Used to seed formula liveness — a `@formula` bound
/// as a chart data/category or cross-tab dimension is live.
fn grid_binding_refs(r: &Report) -> impl Iterator<Item = &str> {
    r.report_definition
        .areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|sec| &sec.objects)
        .flat_map(|obj| -> Box<dyn Iterator<Item = &str>> {
            match &obj.kind {
                ReportObjectKind::Chart(c) => Box::new(
                    c.data_refs
                        .iter()
                        .chain(&c.category_refs)
                        .map(String::as_str),
                ),
                ReportObjectKind::CrossTab(c) => Box::new(c.field_refs.iter().map(String::as_str)),
                _ => Box::new(std::iter::empty()),
            }
        })
}

/// The set of formula names whose field references count toward `UseCount`. A formula is *used* if
/// it is placed (a field object's `{@name}` DataSource, or an `@name` embedded in a text object),
/// named by a section/object conditional-format formula or a selection formula, or transitively
/// referenced (`{@name}`) by another used formula. Mirrors the engine, which does not count the
/// references of a formula that is defined but never reached.
fn live_formulas(r: &Report) -> HashSet<String> {
    let bodies: Vec<(&str, &str)> = r
        .data_definition
        .field_definitions
        .iter()
        .filter_map(|fd| match &fd.kind {
            FieldKindData::Formula(ff) => Some((fd.name.as_str(), ff.text.0.as_str())),
            _ => None,
        })
        .collect();

    // The set of formula names mentioned (`{@name}`) by any site that can place/name a formula.
    // Tokenizer-driven, so a `{@name}` inside a comment or string literal does not count.
    let mut mentioned: HashSet<String> = HashSet::new();
    for area in &r.report_definition.areas {
        for sec in &area.sections {
            for (_, body) in &sec.condition_formulas {
                add_formula_mentions(body, &mut mentioned);
            }
            for obj in &sec.objects {
                match &obj.kind {
                    ReportObjectKind::Field(f) => {
                        add_formula_mentions(&f.data_source, &mut mentioned)
                    }
                    ReportObjectKind::Text(t) => {
                        for ef in &t.embedded_fields {
                            add_formula_mentions(&format!("{{{ef}}}"), &mut mentioned);
                        }
                    }
                    _ => {}
                }
                for (_, body) in &obj.format.condition_formulas {
                    add_formula_mentions(body, &mut mentioned);
                }
                for (_, body) in &obj.border.condition_formulas {
                    add_formula_mentions(body, &mut mentioned);
                }
                if let Some(fc) = font_color_of(obj) {
                    for (_, body) in &fc.condition_formulas {
                        add_formula_mentions(body, &mut mentioned);
                    }
                }
            }
        }
    }
    if let Some(f) = &r.data_definition.record_selection {
        add_formula_mentions(&f.0, &mut mentioned);
    }
    if let Some(f) = &r.data_definition.group_selection {
        add_formula_mentions(&f.0, &mut mentioned);
    }
    for body in &r.data_definition.running_total_condition_formulas {
        add_formula_mentions(body, &mut mentioned);
    }
    // A formula bound as a chart category / cross-tab dimension (`@name`) is referenced by that grid,
    // so it is live; its `{@name}` token seeds the mentioned set like any other placement.
    for f in grid_binding_refs(r) {
        if f.starts_with('@') {
            add_formula_mentions(&format!("{{{f}}}"), &mut mentioned);
        }
    }
    // A formula used as a subreport link's MAIN-report field (`{@name}` → `{?Pm-@name}`) is evaluated
    // by the engine each time it feeds the link parameter, so it is live even when never placed (its
    // field references still count). The link is stored on the subreport; its main field names this
    // (parent) level's formula.
    for s in &r.subreports {
        for link in &s.links {
            if link.main_report_field.starts_with('@') {
                add_formula_mentions(&format!("{{{}}}", link.main_report_field), &mut mentioned);
            }
        }
    }
    // A running total summarizing a formula (`Sum of {@rec}`) keeps that formula live — its value is
    // recomputed every record the running total spans — so the formula's own field references count
    // even when the formula is never otherwise placed. Seed its `{@name}` token like any placement;
    // the dedup/fixpoint below means N running totals over the same formula still make it live once,
    // so its body's references are counted once (matching the engine). A running total over a *database*
    // field, not a formula, instead adds one direct use of that field (handled in `count_report`).
    for fd in &r.data_definition.field_definitions {
        if let FieldKindData::RunningTotal(rt) = &fd.kind {
            if rt.summarized_field.starts_with('@') {
                add_formula_mentions(&format!("{{{}}}", rt.summarized_field), &mut mentioned);
            }
        }
    }

    // Transitive closure: a formula whose name is mentioned is live, and its body then extends the
    // mentioned set (so formulas it names become live too). Iterate to a fixpoint.
    let mut live: HashSet<String> = HashSet::new();
    loop {
        let mut changed = false;
        for (name, body) in &bodies {
            if !live.contains(*name) && mentioned.contains(*name) {
                live.insert((*name).to_string());
                add_formula_mentions(body, &mut mentioned);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    live
}

/// Add one use for each database field a formula body names (case-insensitively, at most once per
/// field per body). The engine counts a formula's reference to a field once regardless of how many
/// times it appears. Reference extraction is routed through [`formula::references`] (a real
/// tokenizer): `//` comments and string-literal contents yield no references, and a field that
/// appears only as an aggregation group-by argument does not count
/// ([`formula::Ref::is_aggregation_group_arg`]).
fn bump_body(
    counts: &mut HashMap<String, i32>,
    field_refs: &[String],
    value_fields: &HashSet<String>,
    body: &str,
) {
    if body.is_empty() {
        return;
    }
    let mut seen: HashSet<&String> = HashSet::new();
    for r in formula::references(body) {
        if r.kind != formula::RefKind::Field || r.is_aggregation_group_arg() {
            continue;
        }
        let key = format!("{{{}}}", r.name);
        let Some(fr) = field_refs.iter().find(|fr| fr.eq_ignore_ascii_case(&key)) else {
            continue;
        };
        // A `GroupName({field})` argument is a group selector, not a value dependency. When the field
        // is also referenced as a displayed value (a field object / blob / text-embedded field), the
        // engine reuses that reference and the selector adds nothing — so skip it here. When the
        // field's only value use is via `GroupName` (it is otherwise group-only), the selector *does*
        // create the reference, so it still counts.
        if r.is_group_name_arg() && value_fields.contains(fr) {
            continue;
        }
        if seen.insert(fr) {
            *counts.entry(fr.clone()).or_default() += 1;
        }
    }
}

/// Database fields referenced as a **displayed value**: the `DataSource` of a field/blob object or a
/// field embedded in a text object. These are the value references that absorb a `GroupName({field})`
/// group-selector argument (see [`bump_body`]). Group / sort / chart / cross-tab / running-total
/// references are excluded — they do not absorb the selector, so a field used only as a
/// group/grid/running-total field still counts its `GroupName` arguments.
fn displayed_value_fields(r: &Report, field_refs: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut note = |s: &str| {
        if let Some(fr) = field_refs.iter().find(|fr| fr.as_str() == s) {
            out.insert(fr.clone());
        }
    };
    for area in &r.report_definition.areas {
        for sec in &area.sections {
            for obj in &sec.objects {
                match &obj.kind {
                    ReportObjectKind::Field(f) => note(&f.data_source),
                    ReportObjectKind::BlobField(b) => note(&b.data_source),
                    ReportObjectKind::Text(t) => {
                        for ef in &t.embedded_fields {
                            if ef.contains('.') && !ef.starts_with('@') && !ef.starts_with('?') {
                                note(&format!("{{{ef}}}"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}
