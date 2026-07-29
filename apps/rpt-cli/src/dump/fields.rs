//! The field-table view: what a record type's declarative table made of one record.
//!
//! For a record type decoded from a field table, this replaces the scalar-probe grid — the grid
//! probes every offset for a plausible integer, which is the question the table has already
//! answered. It reports each field's value with the bytes it came from, keeps the table's `skip`
//! runs visible as the unknowns they are, and states whether the table consumed the record exactly.
//!
//! Two coordinates appear per field and they are not interchangeable: `range` is an absolute offset
//! into the stream's decoded logical buffer, `joined` is the offset in the joined-runs buffer the
//! hex dump above shows. That buffer splices nested records out, so for a record with children the
//! two diverge by the framed length of every child before that field.

use std::fmt::Write as _;

use rpt_reader::fields::{ByteRange, FieldRead, FieldValue, RecordFields};
use rpt_reader::raw::Dialect;

use super::parse::type_label;

/// Render one record's field-table reading as an aligned table with a verdict line.
pub(super) fn render_fields(out: &mut String, r: &RecordFields, dialect: Dialect, whole: bool) {
    let _ = writeln!(
        out,
        "   field table {} · {} field(s) · {}",
        r.table,
        r.fields.len(),
        if r.complete {
            "every field reached"
        } else {
            "stopped before the last field"
        }
    );
    // `--whole` prints the masked on-disk span instead of the joined runs, so the joined column
    // indexes a buffer that is not on screen.
    if whole {
        let _ = writeln!(out, "   range is an absolute offset in the decoded stream");
        let _ = writeln!(
            out,
            "   joined indexes the demasked joined-runs buffer, not the whole-span hex above"
        );
    } else {
        let _ = writeln!(
            out,
            "   range is an absolute offset in the decoded stream; joined is the offset in the hex above"
        );
    }
    if r.child_count > 0 {
        let _ = writeln!(
            out,
            "   the joined runs splice this record's {} child record(s) out, so the two diverge past each child",
            r.child_count
        );
    }

    let rows: Vec<Row> = r.fields.iter().map(|f| row_of(f, dialect)).collect();
    let width = |pick: fn(&Row) -> &str, header: &str| {
        rows.iter()
            .map(|r| pick(r).chars().count())
            .chain(std::iter::once(header.chars().count()))
            .max()
            .unwrap_or(0)
    };
    let (wl, wr, wf, wk) = (
        width(|r| &r.joined, "joined"),
        width(|r| &r.range, "range"),
        width(|r| &r.field, "field"),
        width(|r| &r.kind, "kind"),
    );
    let _ = writeln!(
        out,
        "     {:<wl$}  {:<wr$}  {:<wf$}  {:<wk$}  value",
        "joined", "range", "field", "kind"
    );
    for row in &rows {
        let _ = writeln!(
            out,
            "     {:<wl$}  {:<wr$}  {:<wf$}  {:<wk$}  {}",
            row.joined, row.range, row.field, row.kind, row.value
        );
    }

    if r.schema_too_new {
        // The table was never applied, so the accounting below would describe a reading that did
        // not happen. Say which it is: pointing the reader at the field table here sends them after
        // the wrong thing entirely.
        let _ = writeln!(
            out,
            "   this record's schema is newer than any layout this reader knows for its type, so \
             it was not read at all"
        );
    } else if r.exact() {
        let _ = writeln!(
            out,
            "   the table consumed the record exactly: no unread bytes, no undeclared child"
        );
    } else {
        let mut why = Vec::new();
        if r.unread > 0 {
            why.push(format!("{} field byte(s) unread", r.unread));
        }
        if r.undeclared_children > 0 {
            why.push(format!(
                "{} undeclared child record(s)",
                r.undeclared_children
            ));
        }
        if r.stop.blocked_by_child {
            why.push("a read hit an undeclared child".into());
        }
        if r.stop.child_mismatch {
            why.push("a declared child was not there".into());
        }
        let _ = writeln!(
            out,
            "   the table does NOT consume this record: {} — it stops at 0x{:x}",
            why.join(", "),
            r.read_end()
        );
    }
    if r.stop.ended && !r.complete {
        let _ = writeln!(
            out,
            "   the record ended before the table's last field; the rest keep their defaults"
        );
    }
}

/// One rendered line of the table.
struct Row {
    joined: String,
    range: String,
    field: String,
    kind: String,
    value: String,
}

/// A field as its five cells. The joined cell is a dash for a field that occupies no field bytes —
/// a nested child record.
fn row_of(f: &FieldRead, dialect: Dialect) -> Row {
    Row {
        joined: if f.joined.is_empty() {
            "—".into()
        } else {
            format!("0x{:04x}", f.joined.start)
        },
        range: format!("0x{:x}..0x{:x}", f.span.start, f.span.end),
        field: f.path.clone(),
        kind: f.kind.label().to_string(),
        value: value_cell(&f.value, dialect),
    }
}

/// A field's value, rendered for a byte-layout view: scalars in both bases, undecoded runs as their
/// bytes, a child as its record type.
fn value_cell(v: &FieldValue, dialect: Dialect) -> String {
    match v {
        FieldValue::Uint(x) if *x < 10 => x.to_string(),
        FieldValue::Uint(x) => format!("{x} (0x{x:x})"),
        FieldValue::Int(x) if x.unsigned_abs() < 10 => x.to_string(),
        // Hex of the widened two's complement, which is what makes a sentinel bit pattern legible.
        FieldValue::Int(x) => format!("{x} (0x{x:x})"),
        FieldValue::Float(x) => format!("{x}"),
        FieldValue::Text(t) => {
            let t: String = t.chars().take(60).collect();
            format!("{t:?}")
        }
        FieldValue::Bytes(b) => {
            let head: Vec<String> = b.iter().take(12).map(|x| format!("{x:02x}")).collect();
            let more = if b.len() > 12 { " …" } else { "" };
            format!("{}B: {}{more}", b.len(), head.join(" "))
        }
        FieldValue::FieldRef { name, kind, index } => match index {
            Some(i) => format!("{name:?} → pool {kind} #{i}"),
            None => format!("{name:?} → unset"),
        },
        FieldValue::Child { rtype, .. } => type_label(*rtype, dialect),
        FieldValue::Repeat(n) => format!("{n} row(s)"),
        // A value shape this view has no cell for yet: `Debug` names it, which beats hiding it in a
        // view whose whole job is to show what the table read.
        other => format!("{other:?}"),
    }
}

/// A byte range as `[start, end)`, for the JSON view.
pub(super) fn range_pair(r: ByteRange) -> [usize; 2] {
    [r.start, r.end]
}
