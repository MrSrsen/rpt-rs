//! The version-gate census, and the ladder that exercises every gate.
//!
//! A record's schema word is a version, and a version can add a field, replace a run of fields, or
//! widen one. The corpus cannot settle those declarations on its own: it holds one version per
//! record type for all but a single type, so a gate that never fires cannot be shown to fire in the
//! right direction, or at all. The census measures exactly how much of the declaration that leaves
//! unwitnessed; the ladder after it exercises every gate on records built at each version.

use super::*;

/// What a version does to a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Form {
    /// Adds it: absent before that version, present at it and after.
    Adds,
    /// Replaces the fields beside it: present at that version alone.
    Replaces,
    /// Widens it: present throughout, at one width before that version and another from it.
    Widens,
}

/// One declaration whose reading follows the record's version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Gate {
    dialect: Dialect,
    rtype: u16,
    table: &'static str,
    field: &'static str,
    /// The version the gate turns on at.
    at: u16,
    form: Form,
}

impl Gate {
    /// Whether a record of this version is on the gate's near side — carrying the added field, the
    /// alternative layout, or the wide form.
    fn fires_at(&self, schema: u16) -> bool {
        match self.form {
            Form::Replaces => schema == self.at,
            Form::Adds | Form::Widens => schema >= self.at,
        }
    }

    /// Whether the gate decides the field is there at all, rather than only how wide it is.
    fn decides_presence(&self) -> bool {
        self.form != Form::Widens
    }
}

/// Every version gate in `fields`, repeat bodies included.
///
/// A schema-driven [`Presence::When`] predicate is **not** found here: a predicate is a function,
/// so the version it tests is not readable from the declaration. One such gate exists — the
/// alternative attribute word of the query engine's logon property — and it is exercised by the
/// ladder along with the three `only_at` fields it stands opposite.
fn gates_in(fields: &'static [Field], t: &'static Table, dialect: Dialect, out: &mut Vec<Gate>) {
    for f in fields {
        let gate = |at, form| Gate {
            dialect,
            rtype: t.rtype,
            table: t.name,
            field: f.name,
            at,
            form,
        };
        match f.presence {
            Presence::FromSchema(v) => out.push(gate(v, Form::Adds)),
            Presence::OnlyAtSchema(v) => out.push(gate(v, Form::Replaces)),
            _ => {}
        }
        match f.kind {
            Kind::WidensAt { at, .. } => out.push(gate(at, Form::Widens)),
            Kind::Repeat { body, .. } => gates_in(body, t, dialect, out),
            _ => {}
        }
    }
}

/// Every version gate the registry declares.
fn all_gates() -> Vec<Gate> {
    let mut out = Vec::new();
    for (dialect, t) in registry() {
        gates_in(t.fields, t, dialect, &mut out);
    }
    out.sort_unstable();
    out
}

/// How many records of each tabled type the corpus holds at each version.
///
/// Keyed the way [`tables::for_record`] routes: by dialect and type.
fn versions_observed() -> BTreeMap<(Dialect, u16), BTreeMap<u16, usize>> {
    let mut obs: BTreeMap<(Dialect, u16), BTreeMap<u16, usize>> = BTreeMap::new();
    corpus::for_each_record(|dialect, node, _, _| {
        if tables::for_record(node.rtype, node.schema, dialect).is_none() {
            return;
        }
        *obs.entry((dialect, node.rtype))
            .or_default()
            .entry(node.schema)
            .or_default() += 1;
    });
    obs
}

/// How many version gates the corpus witnesses, and how many it cannot.
///
/// The corpus stores **one version per record type**, with a single exception, so for almost every
/// gate every record either satisfies it or none does. A gate seen in only one state is a
/// declaration the corpus cannot contradict: pinning the width of a field that is always present
/// changes nothing, and removing a field that is never present changes nothing either. The numbers
/// below are that exposure, stated rather than assumed, and they move loudly when a gate is added
/// or a report at another version joins the corpus.
///
/// The one gate the corpus does settle is a **width**: the offset entry's count is stored narrow or
/// wide according to the value, so both forms occur in the same stream of the same report.
#[test]
fn the_corpus_witnesses_one_version_gate_in_both_states() {
    /// Gates whose field is present on some corpus record and absent on another.
    const WITNESSED_BOTH_WAYS: usize = 1;
    /// Gates the corpus only ever satisfies, so the field is present on every record.
    const ALWAYS_SATISFIED: usize = 23;
    /// Gates no corpus record satisfies, so the field is present on none.
    const NEVER_SATISFIED: usize = 6;
    /// Gates on a record type the corpus holds no record of at all.
    const NO_RECORD_AT_ALL: usize = 19;

    let obs = versions_observed();
    let (mut both, mut satisfied, mut never, mut absent) = (0, 0, 0, 0);
    let mut report = String::new();
    for g in all_gates() {
        let seen = obs.get(&(g.dialect, g.rtype));
        let (on, off) = seen.map_or((0, 0), |m| {
            m.iter().fold((0usize, 0usize), |(on, off), (s, n)| {
                if g.fires_at(*s) {
                    (on + n, off)
                } else {
                    (on, off + n)
                }
            })
        });
        let state = match (on, off) {
            (0, 0) => {
                absent += 1;
                "no record of the type"
            }
            (_, 0) => {
                satisfied += 1;
                "always satisfied"
            }
            (0, _) => {
                never += 1;
                "never satisfied"
            }
            _ => {
                both += 1;
                "both states"
            }
        };
        report.push_str(&format!(
            "0x{:04x} {:<20} {:<28} {:?} at 0x{:04x}  near={on} far={off}  {state}\n",
            g.rtype, g.table, g.field, g.form, g.at,
        ));
    }

    let total = both + satisfied + never + absent;
    let measured = (both, satisfied, never, absent);
    let pinned = (
        WITNESSED_BOTH_WAYS,
        ALWAYS_SATISFIED,
        NEVER_SATISFIED,
        NO_RECORD_AT_ALL,
    );
    if measured != pinned || std::env::var_os("RPT_FIELD_TABLE_REPORT").is_some() {
        eprintln!("{total} version gates over the corpus:\n{report}");
    }
    assert_eq!(
        measured, pinned,
        "the version-gate exposure moved: (both states, always satisfied, never satisfied, no record) \
         measured {measured:?}, pinned {pinned:?}"
    );
    assert_eq!(
        total - both,
        ALWAYS_SATISFIED + NEVER_SATISFIED + NO_RECORD_AT_ALL,
        "every gate but the witnessed one rests on the record readers rather than on the corpus"
    );
}

/// A value for each wire kind, so a whole record can be built from a table alone.
///
/// The values are arbitrary but distinguishable; what matters is that each occupies the bytes its
/// kind occupies, so the record built from them has the layout the version declares. A field that
/// counts a repeat takes the number of rows that repeat is given, so the two agree.
fn sample_value(kind: Kind, count: bool) -> Option<Cell> {
    Some(match kind {
        Kind::U8 | Kind::U16Be | Kind::U32Be | Kind::VarU16 | Kind::VarU32 | Kind::Bool => {
            Cell::U(1)
        }
        Kind::I8 | Kind::I16Be | Kind::I32Be => Cell::I(if count { 1 } else { -1 }),
        Kind::F32Le | Kind::F32Be => Cell::F32(1.5),
        Kind::F64Le | Kind::F64Be => Cell::F64(1.5),
        Kind::Str => Cell::Str {
            text: "x".into(),
            block: b"x\0".to_vec(),
        },
        Kind::Blob => Cell::Bytes(vec![0xaa, 0xbb]),
        Kind::FieldRef => Cell::Ref {
            text: "x".into(),
            block: b"x\0".to_vec(),
            kind: 1,
            index: Some(2),
        },
        Kind::Skip(n) => Cell::Bytes(vec![0; n]),
        // A nested record is an identity rather than bytes of its own; its framed length is what it
        // occupies in the parent, which nothing here reads.
        Kind::Child(rtype) => Cell::Child(crate::field_table::cursor::ChildRef {
            rtype,
            schema: 0x0700,
            framed_len: 4,
        }),
        // A repeat's rows come from its body's own declaration, in `row_of`; a width follows the
        // version rather than the declaration.
        Kind::Repeat { .. } | Kind::WidensAt { .. } => return None,
    })
}

/// A row carrying every field `table` declares, built from the declaration itself.
///
/// A widened field takes the value both of its widths can hold, and every field a version decides
/// is carried whatever the version, so one row is emitted at each in turn and the version alone
/// decides the layout. Two things the declaration decides for itself are settled here instead: a
/// repeat is given rows — one per row its count states — because a version gate inside a repeat
/// body only moves bytes when the repeat runs, and a field a *predicate* decides is carried only
/// while that predicate holds, since a row carrying a field the predicate excludes would come back
/// without it.
fn full_row(table: &Table, schema: u16) -> Row {
    row_of(table.fields, None, table.rtype, schema)
}

fn row_of(fields: &'static [Field], outer: Option<&Row>, rtype: u16, schema: u16) -> Row {
    let counted: Vec<&str> = fields
        .iter()
        .filter_map(|f| match f.kind {
            Kind::Repeat {
                count: Count::FromField(n),
                ..
            } => Some(n),
            _ => None,
        })
        .collect();
    let mut row = Row::declaring(fields);
    for f in fields {
        if let Presence::When(p) = f.presence {
            let admits = p(&Ctx {
                rtype,
                schema,
                row: &row,
                outer,
                index: 0,
            });
            if !admits {
                continue;
            }
        }
        if let Kind::Repeat { count, body } = f.kind {
            let n = match count {
                Count::Fixed(n) => n,
                Count::FromField(_) => 1,
            };
            let rows: Vec<Row> = (0..n)
                .map(|_| row_of(body, Some(&row), rtype, schema))
                .collect();
            row.push(f.name, f.kind, Cell::Seq(rows), Span::NONE);
            continue;
        }
        let kind = match f.kind {
            Kind::WidensAt { narrow, .. } => narrow.kind(),
            k => k,
        };
        if let Some(v) = sample_value(kind, counted.contains(&f.name)) {
            row.push(f.name, kind, v, Span::NONE);
        }
    }
    row
}

/// Every version gate, exercised on a record built at each version the declaration names.
///
/// The corpus cannot do this — it holds one version per type — so the record is built from the
/// table and emitted at each version in turn. What is checked at every version is that the record
/// reads back accounted for exactly, carries precisely the fields that version has and no others,
/// and re-emits byte for byte; and that a version boundary changes the record's length, so a gate
/// that quietly never fired would be caught rather than passing as agreement.
///
/// This tests the mechanism, not the layouts: the version a field belongs to is not falsifiable
/// from the corpus, and only a file written by an older engine could confirm it.
#[test]
fn a_version_gate_moves_a_records_layout_at_the_version_it_names() {
    let gates = all_gates();
    let mut tables_checked = 0usize;
    let mut boundaries = 0usize;
    for (dialect, table) in registry() {
        let mut thresholds: Vec<u16> = gates
            .iter()
            .filter(|g| g.dialect == dialect && g.rtype == table.rtype)
            .map(|g| g.at)
            .collect();
        if thresholds.is_empty() {
            continue;
        }
        thresholds.sort_unstable();
        thresholds.dedup();
        tables_checked += 1;

        // Each threshold, and the version just below it: the two sides of every boundary.
        let mut versions: Vec<u16> = thresholds
            .iter()
            .flat_map(|v| [v - 1, *v])
            .chain(thresholds.last().map(|v| v + 1))
            .collect();
        versions.sort_unstable();
        versions.dedup();

        let mut len_at: BTreeMap<u16, usize> = BTreeMap::new();
        for &schema in &versions {
            let row = full_row(table, schema);
            let content = RecordContent {
                rtype: table.rtype,
                schema,
                pieces: write_as(table, &row, schema, StringFormat::Enhanced),
            };
            let at = format!("0x{:04x} {} at 0x{schema:04x}", table.rtype, table.name);
            len_at.insert(schema, content.field_byte_len());

            let back = read_strings(table, &content, StringFormat::Enhanced);
            assert!(back.exact(), "{at}: not accounted for exactly: {back:?}");
            assert!(back.complete, "{at}: read short: {back:?}");

            for f in table.fields {
                // What the version says the record carries: its structural gates, and — for the
                // one gate stated as a predicate rather than a version — the predicate itself.
                let carried = gates
                    .iter()
                    .filter(|g| {
                        g.dialect == dialect
                            && g.rtype == table.rtype
                            && g.field == f.name
                            && g.decides_presence()
                    })
                    .all(|g| g.fires_at(schema))
                    && match f.presence {
                        Presence::When(p) => p(&Ctx {
                            rtype: table.rtype,
                            schema,
                            row: &row,
                            outer: None,
                            index: 0,
                        }),
                        _ => true,
                    };
                assert_eq!(
                    back.row.get(f.name).is_some(),
                    carried,
                    "{at}: field `{}` is {} but the version says otherwise",
                    f.name,
                    if carried { "absent" } else { "present" }
                );
                if carried {
                    assert_eq!(
                        back.row.get(f.name),
                        row.get(f.name),
                        "{at}: field `{}` came back changed",
                        f.name
                    );
                }
            }
            assert_eq!(
                write_as(table, &back.row, schema, StringFormat::Enhanced),
                content.pieces,
                "{at}: did not re-emit byte for byte"
            );
        }

        // A boundary that changes no bytes is a gate that did not fire.
        for v in thresholds {
            assert_ne!(
                len_at[&v],
                len_at[&(v - 1)],
                "0x{:04x} {}: crossing 0x{v:04x} left the record's length unchanged",
                table.rtype,
                table.name
            );
            boundaries += 1;
        }
    }
    assert!(
        tables_checked >= 7 && boundaries >= 14,
        "{tables_checked} versioned tables over {boundaries} version boundaries"
    );
}
