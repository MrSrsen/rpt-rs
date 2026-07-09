//! Which database fields a report actually references — so the generated query can project only the
//! used columns and omit tables that contribute none.
//!
//! The native engine fetches ONLY the fields a report uses and includes a table ONLY when it
//! contributes a used field; a declared-but-unused table is left out of the `FROM` entirely (rather
//! than cross-joined into a cartesian). [`used_database_fields`] reproduces that: it walks every site
//! that can reference a database field — placed field/blob/text objects, group/sort keys, running
//! totals, summaries, chart/cross-tab bindings, record- and group-selection formulas, and the bodies
//! of every *live* formula (resolved transitively) — and returns the set of referenced field keys.
//!
//! The walk mirrors the `UseCount` reference walk but only needs the *set* of referenced fields, not
//! per-field counts. It is deliberately **conservative**: when in doubt a field is included, since an
//! extra fetched column is harmless while a missing one breaks rendering.

use crystal_formula::refs::references;
use crystal_formula::RefKind;
use rpt_model::{
    Database, FieldKindData, FontColor, Report, ReportObject, ReportObjectKind, Table,
};
use std::collections::HashSet;

/// The set of database-field reference keys a report references anywhere, normalized to lowercase.
/// Keys are the `table.field` / `alias.field` form fields are named by in formulas and bindings.
///
/// Each report scope (main report / a subreport) is walked on its own — a subreport's fields belong
/// to its own scope. A *main*-report field exported into a subreport via a link is included here (the
/// engine fetches it to feed the link), but the subreport's own references are not.
pub fn used_database_fields(report: &Report) -> HashSet<String> {
    let live = live_formulas(report);
    let mut used = HashSet::new();
    collect(report, &live, &mut used);
    used
}

/// Add a single reference token (`{table.field}`, `table.field`, `{@f}`, `?p`, …) to the used set
/// when it is a database field. Sigil-prefixed references (formula `@`, parameter `?`, running total
/// `#`, SQL expression `%`) and empty tokens are ignored. Handles both the braced and bare forms.
fn add_field_ref(used: &mut HashSet<String>, raw: &str) {
    let inner = strip_braces(raw);
    if inner.is_empty() || inner.starts_with(['@', '?', '#', '%']) {
        return;
    }
    used.insert(inner.to_ascii_lowercase());
}

/// Add every database field a formula *body* names (via the reference tokenizer, so a `{table.field}`
/// inside a `//` comment or string literal yields nothing).
fn add_body_fields(used: &mut HashSet<String>, body: &str) {
    if body.is_empty() {
        return;
    }
    for r in references(body) {
        if r.kind == RefKind::Field {
            add_field_ref(used, &r.name);
        }
    }
}

/// Strip a single pair of surrounding `{}` braces from a reference token, trimming whitespace.
fn strip_braces(raw: &str) -> &str {
    let s = raw.trim();
    s.strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'))
        .map(str::trim)
        .unwrap_or(s)
}

/// Walk one report scope, collecting the database fields it references.
fn collect(r: &Report, live: &HashSet<String>, used: &mut HashSet<String>) {
    let dd = &r.data_definition;

    // Field definitions: a *live* formula contributes its body's field references; a running total
    // references its summarized field and (for an OnChangeOfField reset) its change field directly.
    for fd in &dd.field_definitions {
        match &fd.kind {
            FieldKindData::Formula(ff) if live.contains(&fd.name.to_ascii_lowercase()) => {
                add_body_fields(used, &ff.text.0);
            }
            FieldKindData::RunningTotal(rt) => {
                add_field_ref(used, &rt.summarized_field);
                add_field_ref(used, &rt.on_change_field);
            }
            _ => {}
        }
    }

    // Selection formulas (always active).
    if let Some(f) = &dd.record_selection {
        add_body_fields(used, &f.0);
    }
    if let Some(f) = &dd.group_selection {
        add_body_fields(used, &f.0);
    }

    // Running-total condition formulas and any conditional-format bodies not attached to a
    // section/object (both hold persistent field references).
    for b in &dd.running_total_condition_formulas {
        add_body_fields(used, b);
    }
    for b in &dd.condition_formula_bodies {
        add_body_fields(used, b);
    }

    // Grouping, sorting, and summary bindings.
    for g in &dd.groups {
        add_field_ref(used, &g.condition_field);
    }
    for s in &dd.record_sorts {
        add_field_ref(used, &s.field);
    }
    for f in &dd.summary_binding_fields {
        add_field_ref(used, f);
    }

    // Placed objects (display fields, embedded text fields, chart/cross-tab bindings) and every
    // object/section conditional-format formula.
    for area in &r.report_definition.areas {
        for sec in &area.sections {
            for (_, b) in &sec.condition_formulas {
                add_body_fields(used, b);
            }
            for obj in &sec.objects {
                collect_object(obj, used);
            }
        }
    }

    // A main-report field exported to a subreport via a link is fetched at this level.
    for s in &r.subreports {
        for link in &s.links {
            add_field_ref(used, &link.main_report_field);
        }
    }
}

/// Collect the database fields one placed object references (its data binding plus its
/// conditional-format formulas).
fn collect_object(obj: &ReportObject, used: &mut HashSet<String>) {
    match &obj.kind {
        ReportObjectKind::Field(f) => add_field_ref(used, &f.data_source),
        ReportObjectKind::BlobField(b) => add_field_ref(used, &b.data_source),
        ReportObjectKind::Text(t) => {
            for ef in &t.embedded_fields {
                add_field_ref(used, ef);
            }
        }
        ReportObjectKind::Chart(c) => {
            for f in c.data_refs.iter().chain(&c.category_refs) {
                add_field_ref(used, f);
            }
        }
        ReportObjectKind::CrossTab(c) => {
            for f in &c.field_refs {
                add_field_ref(used, f);
            }
        }
        _ => {}
    }
    for (_, b) in &obj.format.condition_formulas {
        add_body_fields(used, b);
    }
    for (_, b) in &obj.border.condition_formulas {
        add_body_fields(used, b);
    }
    if let Some(fc) = font_color_of(obj) {
        for (_, b) in &fc.condition_formulas {
            add_body_fields(used, b);
        }
    }
}

/// The object's font/color block, when it has one (the only objects carrying conditional-format font
/// formulas).
fn font_color_of(obj: &ReportObject) -> Option<&FontColor> {
    match &obj.kind {
        ReportObjectKind::Text(t) => Some(&t.font_color),
        ReportObjectKind::Field(f) => Some(&f.font_color),
        ReportObjectKind::FieldHeading(h) => Some(&h.font_color),
        _ => None,
    }
}

/// The set of formula names (lowercased) that are *live*: placed (a field object's `{@name}` or a
/// text object's embedded `@name`), named by a conditional-format / selection formula, bound to a
/// chart/cross-tab or running total, exported to a subreport link, or transitively referenced by
/// another live formula. Only a live formula's field references are fetched — a defined-but-unused
/// formula contributes nothing, matching the engine.
fn live_formulas(r: &Report) -> HashSet<String> {
    let dd = &r.data_definition;
    let bodies: Vec<(String, &str)> = dd
        .field_definitions
        .iter()
        .filter_map(|fd| match &fd.kind {
            FieldKindData::Formula(ff) => Some((fd.name.to_ascii_lowercase(), ff.text.0.as_str())),
            _ => None,
        })
        .collect();

    let mut mentioned: HashSet<String> = HashSet::new();

    // Formula mentions inside a body (`{@name}`), via the tokenizer.
    let mention_body = |body: &str, m: &mut HashSet<String>| {
        for r in references(body) {
            if r.kind == RefKind::Formula {
                m.insert(r.name.to_ascii_lowercase());
            }
        }
    };
    // A single reference token that names a formula (`{@name}` / `@name`).
    let mention_token = |raw: &str, m: &mut HashSet<String>| {
        let inner = strip_braces(raw);
        if let Some(name) = inner.strip_prefix('@') {
            m.insert(name.to_ascii_lowercase());
        }
    };

    for area in &r.report_definition.areas {
        for sec in &area.sections {
            for (_, body) in &sec.condition_formulas {
                mention_body(body, &mut mentioned);
            }
            for obj in &sec.objects {
                match &obj.kind {
                    ReportObjectKind::Field(f) => mention_token(&f.data_source, &mut mentioned),
                    ReportObjectKind::Text(t) => {
                        for ef in &t.embedded_fields {
                            mention_token(ef, &mut mentioned);
                        }
                    }
                    ReportObjectKind::Chart(c) => {
                        for f in c.data_refs.iter().chain(&c.category_refs) {
                            mention_token(f, &mut mentioned);
                        }
                    }
                    ReportObjectKind::CrossTab(c) => {
                        for f in &c.field_refs {
                            mention_token(f, &mut mentioned);
                        }
                    }
                    _ => {}
                }
                for (_, body) in &obj.format.condition_formulas {
                    mention_body(body, &mut mentioned);
                }
                for (_, body) in &obj.border.condition_formulas {
                    mention_body(body, &mut mentioned);
                }
                if let Some(fc) = font_color_of(obj) {
                    for (_, body) in &fc.condition_formulas {
                        mention_body(body, &mut mentioned);
                    }
                }
            }
        }
    }
    if let Some(f) = &dd.record_selection {
        mention_body(&f.0, &mut mentioned);
    }
    if let Some(f) = &dd.group_selection {
        mention_body(&f.0, &mut mentioned);
    }
    for b in &dd.running_total_condition_formulas {
        mention_body(b, &mut mentioned);
    }
    for fd in &dd.field_definitions {
        if let FieldKindData::RunningTotal(rt) = &fd.kind {
            mention_token(&rt.summarized_field, &mut mentioned);
        }
    }
    for s in &r.subreports {
        for link in &s.links {
            mention_token(&link.main_report_field, &mut mentioned);
        }
    }

    // Transitive closure: a mentioned formula is live, and its body then extends the mention set.
    let mut live: HashSet<String> = HashSet::new();
    loop {
        let mut changed = false;
        for (name, body) in &bodies {
            if !live.contains(name) && mentioned.contains(name) {
                live.insert(name.clone());
                mention_body(body, &mut mentioned);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    live
}

/// A report-pruned copy of `database`: only the tables that contribute a used field (or lie on a link
/// path connecting two such tables) are kept, and each kept table projects only its used columns.
///
/// When no field is detected as used (an empty set — e.g. a report the walk can't attribute), the
/// database is returned unpruned, so a collector blind spot never drops a needed table.
pub fn prune_database(database: &Database, used: &HashSet<String>) -> Database {
    // Per-table used columns, and the seed tables that contribute at least one.
    let used_fields: Vec<Vec<usize>> = database
        .tables
        .iter()
        .map(|t| {
            t.data_fields
                .iter()
                .enumerate()
                .filter(|(_, f)| field_is_used(t, f, used))
                .map(|(i, _)| i)
                .collect()
        })
        .collect();
    let seed: HashSet<usize> = used_fields
        .iter()
        .enumerate()
        .filter(|(_, cols)| !cols.is_empty())
        .map(|(i, _)| i)
        .collect();

    if seed.is_empty() {
        return database.clone();
    }

    let included = included_tables(database, &seed);

    // Rebuild the table list in the original order, projecting only each kept table's used columns.
    let mut tables: Vec<Table> = Vec::new();
    for (i, t) in database.tables.iter().enumerate() {
        if !included.contains(&i) {
            continue;
        }
        let mut kept = t.clone();
        kept.data_fields = used_fields[i]
            .iter()
            .map(|&fi| t.data_fields[fi].clone())
            .collect();
        tables.push(kept);
    }

    // Keep only links whose both endpoints survive; a link to a dropped table can't apply.
    let kept_aliases: HashSet<&str> = tables.iter().map(|t| t.alias.as_str()).collect();
    let links = database
        .links
        .iter()
        .filter(|l| {
            kept_aliases.contains(l.source_table_alias.as_str())
                && kept_aliases.contains(l.target_table_alias.as_str())
        })
        .cloned()
        .collect();

    Database { tables, links }
}

/// Whether a table field is referenced by the report. Matched against the used set by its qualified
/// forms (`long_name`, `alias.field`, `name.field`) — the way fields are named in formulas/bindings.
/// Qualified-only matching is intentional: a bare column name (e.g. `id`) is shared across tables, so
/// matching it would keep every same-named column's table.
fn field_is_used(table: &Table, f: &rpt_model::DbFieldDef, used: &HashSet<String>) -> bool {
    let hit = |k: String| used.contains(&k.to_ascii_lowercase());
    if let Some(ln) = &f.long_name {
        if hit(ln.clone()) {
            return true;
        }
    }
    hit(format!("{}.{}", table.alias, f.name)) || hit(format!("{}.{}", table.name, f.name))
}

/// The set of table indices to include: every seed table, plus tables that lie on a link path
/// connecting two seed tables (needed to join them). Two seeds in different connected components stay
/// unbridged — the component-aware join builder cross-joins them, matching the unlinked-but-used case.
fn included_tables(database: &Database, seed: &HashSet<usize>) -> HashSet<usize> {
    let n = database.tables.len();
    let index_of = |alias: &str| database.tables.iter().position(|t| t.alias == alias);

    // Undirected adjacency over the link graph (by table index).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for link in &database.links {
        if let (Some(a), Some(b)) = (
            index_of(&link.source_table_alias),
            index_of(&link.target_table_alias),
        ) {
            adj[a].push(b);
            adj[b].push(a);
        }
    }

    let mut included = seed.clone();
    // Connect each pair of seeds with a shortest path (BFS): every table on the path joins the two
    // used tables, so it must be fetched even though it contributes no column. Pairs in different
    // components have no path and are left to the cross-join bridge.
    let seeds: Vec<usize> = {
        let mut v: Vec<usize> = seed.iter().copied().collect();
        v.sort_unstable();
        v
    };
    for (i, &s) in seeds.iter().enumerate() {
        for &t in &seeds[i + 1..] {
            if let Some(path) = shortest_path(&adj, s, t) {
                included.extend(path);
            }
        }
    }
    included
}

/// Breadth-first shortest path between two table indices over the link adjacency, returned as the
/// sequence of nodes (inclusive of both ends), or `None` when they are in different components.
fn shortest_path(adj: &[Vec<usize>], from: usize, to: usize) -> Option<Vec<usize>> {
    if from == to {
        return Some(vec![from]);
    }
    let mut prev: Vec<Option<usize>> = vec![None; adj.len()];
    let mut seen: HashSet<usize> = HashSet::from([from]);
    let mut queue = std::collections::VecDeque::from([from]);
    while let Some(node) = queue.pop_front() {
        for &next in &adj[node] {
            if seen.insert(next) {
                prev[next] = Some(node);
                if next == to {
                    // Reconstruct the path back to `from`.
                    let mut path = vec![to];
                    let mut cur = node;
                    loop {
                        path.push(cur);
                        match prev[cur] {
                            Some(p) => cur = p,
                            None => break,
                        }
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(next);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_query_for_report, Dialect};
    use rpt_model::{
        Area, DataDefinition, DbFieldDef, FieldDef, FieldObject, Formula, FormulaField, Report,
        ReportDefinition, Section, Subreport, SubreportLink, TableJoinType, TableLink,
    };

    fn tbl(name: &str, fields: &[&str]) -> Table {
        Table {
            name: name.into(),
            alias: name.into(),
            data_fields: fields
                .iter()
                .map(|f| DbFieldDef {
                    name: (*f).into(),
                    long_name: Some(format!("{name}.{f}")),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn link(src: &str, tgt: &str) -> TableLink {
        TableLink {
            join_type: TableJoinType::Equal,
            source_table_alias: src.into(),
            target_table_alias: tgt.into(),
            source_fields: vec!["id".into()],
            target_fields: vec!["id".into()],
        }
    }

    fn field_obj(data_source: &str) -> ReportObject {
        ReportObject {
            kind: ReportObjectKind::Field(FieldObject {
                data_source: data_source.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// A report placing `objs` in a single details section, with `formulas` as `(name, body)`.
    fn report(db: Database, objs: Vec<ReportObject>, formulas: &[(&str, &str)]) -> Report {
        let field_definitions = formulas
            .iter()
            .map(|(n, b)| FieldDef {
                name: (*n).into(),
                kind: FieldKindData::Formula(FormulaField {
                    text: Formula((*b).into()),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        Report {
            database: db,
            data_definition: DataDefinition {
                field_definitions,
                ..Default::default()
            },
            report_definition: ReportDefinition {
                areas: vec![Area {
                    sections: vec![Section {
                        objects: objs,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
            },
            ..Default::default()
        }
    }

    #[test]
    fn one_used_table_prunes_away_stray_tables() {
        // A report uses only `orders`; the other declared tables (a stray unlinked one included) must
        // be dropped, not cross-joined — the cartesian-explosion fix.
        let db = Database {
            tables: vec![
                tbl("orders", &["id", "total", "state"]),
                tbl("stray", &["id", "x"]),
                tbl("misc", &["id", "y"]),
            ],
            links: vec![],
        };
        let r = report(db, vec![field_obj("{orders.total}")], &[]);
        let used = used_database_fields(&r);
        assert!(used.contains("orders.total"));

        let pruned = prune_database(&r.database, &used);
        assert_eq!(
            pruned
                .tables
                .iter()
                .map(|t| t.alias.as_str())
                .collect::<Vec<_>>(),
            vec!["orders"]
        );
        let q = build_query_for_report(&r, &[], None, &[], Dialect::Postgres).unwrap();
        assert_eq!(
            q.sql,
            r#"SELECT "orders"."total"::text FROM "orders" AS "orders""#
        );
    }

    #[test]
    fn transitive_formula_reference_keeps_field() {
        // A placed `{@a}` → formula `a` calls `{@b}` → formula `b` names `{orders.total}`. The
        // transitive resolution must mark `orders.total` used and keep `orders`.
        let db = Database {
            tables: vec![tbl("orders", &["id", "total"]), tbl("stray", &["id"])],
            links: vec![],
        };
        let r = report(
            db,
            vec![field_obj("{@a}")],
            &[("a", "{@b} + 1"), ("b", "{orders.total} * 2")],
        );
        let used = used_database_fields(&r);
        assert!(
            used.contains("orders.total"),
            "transitive field not collected: {used:?}"
        );
        let pruned = prune_database(&r.database, &used);
        assert_eq!(
            pruned
                .tables
                .iter()
                .map(|t| t.alias.as_str())
                .collect::<Vec<_>>(),
            vec!["orders"]
        );
    }

    #[test]
    fn dead_formula_does_not_keep_its_table() {
        // A formula that is defined but never placed/referenced is dead; the field it names must not
        // be fetched (matching the engine), so its stray table is dropped.
        let db = Database {
            tables: vec![tbl("orders", &["id", "total"]), tbl("stray", &["id", "z"])],
            links: vec![],
        };
        let r = report(
            db,
            vec![field_obj("{orders.total}")],
            &[("dead", "{stray.z} + 1")],
        );
        let used = used_database_fields(&r);
        assert!(
            !used.contains("stray.z"),
            "dead formula field leaked: {used:?}"
        );
        let pruned = prune_database(&r.database, &used);
        assert_eq!(
            pruned
                .tables
                .iter()
                .map(|t| t.alias.as_str())
                .collect::<Vec<_>>(),
            vec!["orders"]
        );
    }

    #[test]
    fn unlinked_but_used_two_tables_cross_join_preserved() {
        // Two used tables with no link between them are both kept and cross-joined (the
        // unlinked-but-used case the component-aware join builder handles).
        let db = Database {
            tables: vec![tbl("a", &["id", "va"]), tbl("b", &["id", "vb"])],
            links: vec![],
        };
        let r = report(db, vec![field_obj("{a.va}"), field_obj("{b.vb}")], &[]);
        let used = used_database_fields(&r);
        let pruned = prune_database(&r.database, &used);
        assert_eq!(pruned.tables.len(), 2, "both used tables kept");
        let q = build_query_for_report(&r, &[], None, &[], Dialect::Postgres).unwrap();
        assert!(
            q.sql.contains("ON TRUE"),
            "cross join preserved; sql: {}",
            q.sql
        );
    }

    #[test]
    fn connector_table_kept_to_join_two_used_tables() {
        // Used tables `a` and `b` are linked only through the unused `mid`. `mid` must be kept (it is
        // on the link path) so the two can equijoin rather than cross-join, even though it projects
        // no column.
        let db = Database {
            tables: vec![
                tbl("a", &["id", "va"]),
                tbl("mid", &["id", "a_id", "b_id"]),
                tbl("b", &["id", "vb"]),
            ],
            links: vec![link("a", "mid"), link("mid", "b")],
        };
        let r = report(db, vec![field_obj("{a.va}"), field_obj("{b.vb}")], &[]);
        let used = used_database_fields(&r);
        let pruned = prune_database(&r.database, &used);
        let mut kept: Vec<&str> = pruned.tables.iter().map(|t| t.alias.as_str()).collect();
        kept.sort_unstable();
        assert_eq!(kept, vec!["a", "b", "mid"], "connector kept");
        // The connector projects no column (no used field), but joins the two used tables.
        let mid = pruned.tables.iter().find(|t| t.alias == "mid").unwrap();
        assert!(mid.data_fields.is_empty(), "connector projects nothing");
        let q = build_query_for_report(&r, &[], None, &[], Dialect::Postgres).unwrap();
        assert!(
            !q.sql.contains("ON TRUE"),
            "connected, not cross-joined; sql: {}",
            q.sql
        );
    }

    #[test]
    fn no_used_fields_keeps_full_database() {
        // A report the walk attributes no field to (e.g. only static text) must not drop tables — the
        // conservative fallback.
        let db = Database {
            tables: vec![tbl("orders", &["id"]), tbl("other", &["id"])],
            links: vec![link("orders", "other")],
        };
        let r = report(db, vec![], &[]);
        let used = used_database_fields(&r);
        assert!(used.is_empty());
        let pruned = prune_database(&r.database, &used);
        assert_eq!(pruned.tables.len(), 2, "fallback keeps all tables");
    }

    #[test]
    fn subreport_link_field_used_in_parent() {
        // A main-report field exported to a subreport via a link is fetched at the parent level even
        // when it is displayed nowhere in the parent.
        let db = Database {
            tables: vec![tbl("orders", &["id", "partner_id"])],
            links: vec![],
        };
        let mut r = report(db, vec![], &[]);
        r.subreports = vec![Subreport {
            name: "sub".into(),
            report: Box::new(Report::default()),
            links: vec![SubreportLink {
                main_report_field: "orders.partner_id".into(),
                ..Default::default()
            }],
        }];
        let used = used_database_fields(&r);
        assert!(
            used.contains("orders.partner_id"),
            "link field not collected: {used:?}"
        );
    }
}
