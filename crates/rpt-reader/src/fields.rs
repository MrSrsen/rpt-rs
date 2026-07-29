//! A record's reading under its declarative field table, as plain data.
//!
//! [`read`] walks the field table for a record's type over that record's content and reports what
//! the table made of it: every field it decoded — name, wire type, value, and the bytes it came
//! from — and whether the table accounted for the record exactly. It is the reading the decoder
//! itself performs, exposed for inspection tooling; the table machinery stays private.
//!
//! [`edit`] is the same table read backwards: it encodes a replacement value at a named field's
//! declared wire type and reports the bytes it occupies, so an edit addresses a field rather than
//! an offset that moves with the string before it.
//!
//! Which table applies is keyed on the record's [`Dialect`] as well as its type number: the report
//! definition and the query-engine session both write a `0x0003`, and reading one under the other's
//! table decodes a record that is not there.
//!
//! # Coordinate space
//!
//! Two different coordinates describe the same byte, and confusing them is the whole reason this
//! module reports both:
//!
//! - [`FieldRead::span`] is an **absolute offset into the stream's decoded logical buffer** — the
//!   coordinate [`RecordNode::offset`](crate::raw::RecordNode) and its content bounds use. A
//!   record's nested children occupy part of that span, so a field written after a child begins
//!   past the child's last byte.
//! - [`FieldRead::joined`] is the same bytes' position in the record's **joined runs**
//!   ([`RecordNode::joined_runs`](crate::raw::RecordNode)) — a buffer built by splicing the children
//!   out and concatenating the field-byte runs either side, so it exists nowhere in the file.
//!
//! The two coincide (up to the content start) only for a record with no children. For any other
//! record the joined coordinate skips forward at each child, so an offset read off that buffer is
//! not a position in the file.
//!
//! # Editing safety
//!
//! One property makes a field-addressed edit safe, and it has two halves, both decided here.
//! [`round_trips`] holds before the write: the table re-emits the record it just read byte for
//! byte, so it has accounted for everything an edit could desynchronize. [`verify_edit`] holds
//! after it: the rewritten record still reads exactly under the table, with the named field at its
//! new value and every other field unchanged. A table cannot state the second half on its own — a
//! field's value can decide how many times a later repeat runs, or whether a later field is written
//! at all, and those dependencies live in the table's predicates rather than in anything a caller
//! can inspect. Reading the record back settles them.

use crate::codec::{Dialect, RecordNode};
use crate::error::{EditErrorKind, Error, Result};
use crate::field_table::cursor::{Encoder, Piece, StringFormat};
use crate::field_table::table::{self, Cell, Kind, Row, Span};
use crate::field_table::{content_of, strings_format_of, tables};

/// Why the table's walk stopped before the end of its field list — the cursor's own stop state,
/// which is what a reading reports verbatim.
pub use crate::field_table::cursor::Stop as FieldStop;

/// A half-open byte range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// First byte covered.
    pub start: usize,
    /// One past the last byte covered.
    pub end: usize,
}

impl ByteRange {
    /// The number of bytes covered.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// True when the range covers no bytes.
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// A field's wire type, as its table declares it.
///
/// Every multi-byte kind names its byte order: the format has both, so a bare width would mean
/// big-endian by convention alone. A single byte has no order and carries no suffix.
///
/// The kinds are the encodings the tables have had to describe, and a record type that stores a
/// field in an encoding not listed here adds another — hence `#[non_exhaustive]`. The match that
/// must name them all is the reader that decodes a field, and it lives here, where it is still
/// checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldKind {
    /// A fixed-width unsigned integer.
    U8,
    /// A fixed-width unsigned integer, big-endian.
    U16Be,
    /// A fixed-width unsigned integer, big-endian.
    U32Be,
    /// A fixed-width two's-complement integer.
    I8,
    /// A fixed-width two's-complement integer, big-endian.
    I16Be,
    /// A fixed-width two's-complement integer, big-endian.
    I32Be,
    /// A single-precision float, little-endian.
    F32Le,
    /// A single-precision float, big-endian.
    F32Be,
    /// A double, little-endian.
    F64Le,
    /// A double, big-endian.
    F64Be,
    /// A **narrowing** integer: one byte, or two with the top bit set as the marker.
    VarU16,
    /// A **narrowing** integer: two bytes, or four.
    VarU32,
    /// A string, in the wire form the record's own header declared.
    Text,
    /// A boolean, two bytes big-endian.
    Bool,
    /// A byte count and that many raw bytes.
    Blob,
    /// A field reference: a name plus the handle that resolves it.
    FieldRef,
    /// A run kept verbatim because its meaning is not decoded.
    Skip,
    /// A nested record.
    Child,
    /// A repeated body; its rows follow as their own entries.
    Repeat,
}

impl FieldKind {
    /// The wire type's short label, for display.
    pub fn label(self) -> &'static str {
        match self {
            FieldKind::U8 => "u8",
            FieldKind::U16Be => "u16be",
            FieldKind::U32Be => "u32be",
            FieldKind::I8 => "i8",
            FieldKind::I16Be => "i16be",
            FieldKind::I32Be => "i32be",
            FieldKind::F32Le => "f32le",
            FieldKind::F32Be => "f32be",
            FieldKind::F64Le => "f64le",
            FieldKind::F64Be => "f64be",
            FieldKind::VarU16 => "varu16",
            FieldKind::VarU32 => "varu32",
            FieldKind::Text => "str",
            FieldKind::Bool => "bool",
            FieldKind::Blob => "blob",
            FieldKind::FieldRef => "fieldref",
            FieldKind::Skip => "skip",
            FieldKind::Child => "child",
            FieldKind::Repeat => "repeat",
        }
    }

    /// True for a run the table keeps verbatim because nothing is known about it — an unknown, not
    /// a decoded value.
    pub fn is_unknown(self) -> bool {
        matches!(self, FieldKind::Skip)
    }
}

/// A decoded field value — one shape per family of [`FieldKind`], not one per kind, so a new width
/// of an encoding already read needs no variant here. A new *shape* does, which is why this is
/// `#[non_exhaustive]` alongside the kinds it mirrors.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum FieldValue {
    /// Any unsigned scalar, fixed-width or narrowing.
    Uint(u32),
    /// Any signed scalar, widened from the width it was stored at.
    Int(i32),
    /// A floating-point value, widened from the width it was stored at.
    Float(f64),
    /// A string's text, up to the first NUL.
    Text(String),
    /// A field reference: the field's name, the pool its handle names, and its index in that pool
    /// (`None` when the reference names no field).
    FieldRef {
        /// The referenced field's name.
        name: String,
        /// The pool the handle names.
        kind: u32,
        /// The index within that pool, or `None` when the reference is unset.
        index: Option<u16>,
    },
    /// An undecoded run, verbatim.
    Bytes(Vec<u8>),
    /// A nested record's identity.
    Child {
        /// The nested record's type tag.
        rtype: u16,
        /// The nested record's schema word.
        schema: u16,
    },
    /// A repeated body, and how many times it ran.
    Repeat(usize),
}

/// One field the table read, and the bytes it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldRead {
    /// The field's name qualified by the repeat rows enclosing it (`element_styles[3].weight`).
    pub path: String,
    /// The field's own name in its table.
    pub name: &'static str,
    /// How many repeat bodies enclose this field (`0` at the top level).
    pub depth: usize,
    /// The field's declared wire type.
    pub kind: FieldKind,
    /// What the table decoded there.
    pub value: FieldValue,
    /// Absolute span in the stream's decoded logical buffer.
    pub span: ByteRange,
    /// The same bytes' position in the record's joined runs. Empty for a child record, which
    /// contributes none.
    pub joined: ByteRange,
}

/// What a record's field table made of it.
#[derive(Debug, Clone)]
pub struct RecordFields {
    /// The record's type tag.
    pub rtype: u16,
    /// The record's schema word.
    pub schema: u16,
    /// The table's name for this record type.
    pub table: &'static str,
    /// The record's content span, absolute.
    pub content: ByteRange,
    /// Nested child records inside that span.
    pub child_count: usize,
    /// The fields the table read, in wire order.
    pub fields: Vec<FieldRead>,
    /// Field bytes the table did not account for.
    pub unread: usize,
    /// Child records the table did not declare.
    pub undeclared_children: usize,
    /// Every field in the table was reached.
    pub complete: bool,
    /// The record's schema is newer than the newest layout this reader knows for its type, so it
    /// was refused rather than read. Every other field here then describes an empty reading.
    ///
    /// This is a different verdict from "the table did not account for the record": the table was
    /// never applied. A newer version may have widened a field or added one mid-sequence, and
    /// refusing is what the format's own reader does.
    pub schema_too_new: bool,
    /// Why the walk stopped short of the table's last field, if it did.
    pub stop: FieldStop,
}

impl RecordFields {
    /// The table describes this record exactly: no field bytes left over, no undeclared child, and
    /// no read blocked by one.
    pub fn exact(&self) -> bool {
        !self.schema_too_new
            && self.unread == 0
            && self.undeclared_children == 0
            && !self.stop.blocked_by_child
            && !self.stop.child_mismatch
    }

    /// Where the walk stopped, absolute: the end of the last field it read.
    pub fn read_end(&self) -> usize {
        self.fields
            .iter()
            .map(|f| f.span.end)
            .max()
            .unwrap_or(self.content.start)
    }
}

/// The name of `rtype`'s field table in `dialect`, or `None` when that record type has no table.
pub fn table_name(rtype: u16, dialect: Dialect) -> Option<&'static str> {
    tables::set(dialect)
        .iter()
        .find(|t| t.rtype == rtype)
        .map(|t| t.name)
}

/// Every record type in `dialect` that has a field table, with its table name, in registry order.
pub fn tabled_types(dialect: Dialect) -> Vec<(u16, &'static str)> {
    tables::set(dialect)
        .iter()
        .map(|t| (t.rtype, t.name))
        .collect()
}

/// Read `node`'s content under its record type's field table in `dialect` — the vocabulary of the
/// stream the node was read from ([`RecordStream::dialect`](crate::raw::RecordStream::dialect)).
///
/// Returns `None` when the record type has no table there — the caller's cue that this record is not
/// described declaratively and has to be read as bytes.
///
/// `logical` is the decoded stream buffer the node's offsets index.
pub fn read(node: &RecordNode, logical: &[u8], dialect: Dialect) -> Option<RecordFields> {
    let table = tables::for_record(node.rtype, node.schema, dialect)?;
    let content = content_of(node, logical);
    // The record's own header says which string wire form its content is framed in; the table says
    // only that a field is a string.
    let strings = strings_format_of(node, logical);
    let reading = table::read_strings(table, &content, strings);

    // The joined runs' byte `i` came from logical offset `map[i]`. Both are the record's runs in
    // wire order, so the two stay in step — including where a run falls outside the buffer and
    // contributes no bytes to either.
    let mut map: Vec<usize> = Vec::with_capacity(content.field_byte_len());
    for (from, to) in node.run_spans() {
        if logical.get(from..to).is_some() {
            map.extend(from..to);
        }
    }

    let mut project = Project {
        map,
        children: &node.children,
        next_child: 0,
        content_end: node.content_end,
        out: Vec::new(),
    };
    project.row(&reading.row, "", 0);

    Some(RecordFields {
        rtype: node.rtype,
        schema: node.schema,
        table: table.name,
        content: ByteRange {
            start: node.content_start,
            end: node.content_end,
        },
        child_count: node.children.len(),
        fields: project.out,
        unread: reading.unread,
        undeclared_children: reading.undeclared_children,
        schema_too_new: reading.schema_too_new,
        complete: reading.complete,
        stop: reading.stop,
    })
}

/// A replacement value for a field-addressed edit.
///
/// A variant names a family of wire types rather than one, because the width a value is stored at
/// is the field's business and not the caller's: the same [`FieldEdit::Int`] serves a byte, a
/// big-endian word and a narrowing integer. [`FieldEdit::parse`] builds the variant a given
/// [`FieldKind`] takes.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldEdit {
    /// An integer, for every fixed-width, narrowing and boolean field.
    Int(i64),
    /// A floating-point value, for every float and double field.
    Float(f64),
    /// A string's text. The terminating NUL and the length prefix belong to the encoding.
    Text(String),
    /// Raw bytes, for a blob or for a run whose meaning is not decoded.
    Bytes(Vec<u8>),
}

impl FieldEdit {
    /// Interpret `text` as a value of `kind`.
    ///
    /// An integer field takes a decimal or `0x`-prefixed number, a boolean additionally `true` and
    /// `false`, a float a decimal fraction, a string its literal text, and an undecoded run a hex
    /// byte string. `None` when `kind` takes no editable value, or `text` is not one of `kind`.
    pub fn parse(text: &str, kind: FieldKind) -> Option<FieldEdit> {
        match kind {
            FieldKind::Bool => match text {
                "true" => Some(FieldEdit::Int(1)),
                "false" => Some(FieldEdit::Int(0)),
                _ => parse_int(text).map(FieldEdit::Int),
            },
            FieldKind::U8
            | FieldKind::U16Be
            | FieldKind::U32Be
            | FieldKind::I8
            | FieldKind::I16Be
            | FieldKind::I32Be
            | FieldKind::VarU16
            | FieldKind::VarU32 => parse_int(text).map(FieldEdit::Int),
            FieldKind::F32Le | FieldKind::F32Be | FieldKind::F64Le | FieldKind::F64Be => {
                text.parse().ok().map(FieldEdit::Float)
            }
            FieldKind::Text => Some(FieldEdit::Text(text.to_string())),
            FieldKind::Blob | FieldKind::Skip => parse_hex(text).map(FieldEdit::Bytes),
            FieldKind::FieldRef | FieldKind::Child | FieldKind::Repeat => None,
        }
    }

    /// True when `value`, read back at `kind`, is what this edit asked to store.
    ///
    /// A single-precision field is compared at its own width: a value that does not survive the
    /// round trip through `f32` was never asking for the double it was written as.
    pub fn matches(&self, value: &FieldValue, kind: FieldKind) -> bool {
        match (self, value) {
            (FieldEdit::Int(x), FieldValue::Uint(v)) => i64::from(*v) == *x,
            (FieldEdit::Int(x), FieldValue::Int(v)) => i64::from(*v) == *x,
            (FieldEdit::Float(x), FieldValue::Float(v)) => {
                if matches!(kind, FieldKind::F32Le | FieldKind::F32Be) {
                    *v as f32 == *x as f32
                } else {
                    v == x
                }
            }
            (FieldEdit::Text(s), FieldValue::Text(t)) => t == s,
            (FieldEdit::Bytes(b), FieldValue::Bytes(v)) => v == b,
            _ => false,
        }
    }
}

/// A decimal or `0x`-prefixed integer, either sign.
fn parse_int(text: &str) -> Option<i64> {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let magnitude = match rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok()?,
        None => rest.parse().ok()?,
    };
    Some(sign * magnitude)
}

/// An even-length hex string as bytes. The empty string is an empty run.
fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// What a field-addressed edit writes, and where.
#[derive(Debug, Clone)]
pub struct FieldPatch {
    /// The field's reading before the edit — its declared wire type, its current value, and the
    /// region of the record's joined runs the replacement takes over ([`FieldRead::joined`]).
    pub field: FieldRead,
    /// The replacement bytes, encoded at the field's declared wire type. Not necessarily the same
    /// length as the region they replace: a string and a narrowing integer both change width with
    /// their value.
    pub bytes: Vec<u8>,
    /// The record's whole reading before the edit, which is what the written record is checked
    /// against: every field but the edited one must still read the same.
    pub before: RecordFields,
}

impl FieldPatch {
    /// True when the replacement is the same width as what it replaces, so the record's length and
    /// every enclosing record's length are unchanged.
    pub fn same_width(&self) -> bool {
        self.bytes.len() == self.field.joined.len()
    }
}

/// Encode `value` for the field `path` names in `node`, at that field's declared wire type.
///
/// This is the write half of [`read`]: the table that says where a field's bytes are also says how
/// wide they are and in what order, so an edit names a field instead of an offset that moves with
/// the string before it. `path` is a [`FieldRead::path`] — a field's own name, qualified by the
/// repeat rows enclosing it.
///
/// # Errors
///
/// [`Error::Edit`] in every case, and always before any bytes exist:
///
/// - the record type has no field table in `dialect`, so it has no field names to address;
/// - no field of that path was read — the name is not the table's, or the record ended before it;
/// - the field's wire type takes no value of this shape, or the value does not fit it.
pub fn edit(
    node: &RecordNode,
    logical: &[u8],
    dialect: Dialect,
    path: &str,
    value: &FieldEdit,
) -> Result<FieldPatch> {
    let (reading, field) = locate(node, logical, dialect, path)?;
    let bytes =
        encode_at(field.kind, value, strings_format_of(node, logical)).ok_or_else(|| {
            refuse(format!(
                "`{path}` is a {}, which stores no {value:?} — the value is of the wrong shape for \
                 it, or outside the range its width holds",
                field.kind.label()
            ))
        })?;
    // An undecoded run is declared at a fixed width, so a replacement of another length moves every
    // field after it and the table stops describing the record.
    if field.kind == FieldKind::Skip && bytes.len() != field.joined.len() {
        return Err(refuse(format!(
            "`{path}` is an undecoded run of {} bytes and its width is part of the layout; the \
             replacement is {} bytes",
            field.joined.len(),
            bytes.len()
        )));
    }
    Ok(FieldPatch {
        field,
        bytes,
        before: reading,
    })
}

/// The field `path` names in `node`, read under its record type's table.
///
/// The lookup [`edit`] performs before it encodes anything, so a caller can see a field's current
/// value and declared wire type — and get the same refusal, in the same words — without writing.
///
/// # Errors
///
/// [`Error::Edit`] when the record type has no field table in `dialect`, or no field of that path
/// was read.
pub fn field(node: &RecordNode, logical: &[u8], dialect: Dialect, path: &str) -> Result<FieldRead> {
    locate(node, logical, dialect, path).map(|(_, f)| f)
}

/// A refusal from the field-addressed write path.
fn refuse(detail: String) -> Error {
    Error::Edit {
        kind: EditErrorKind::FieldEdit,
        detail,
    }
}

/// The record's whole reading, and the one field `path` names in it.
fn locate(
    node: &RecordNode,
    logical: &[u8],
    dialect: Dialect,
    path: &str,
) -> Result<(RecordFields, FieldRead)> {
    let reading = read(node, logical, dialect).ok_or_else(|| {
        refuse(format!(
            "record type {:#06x} has no field table, so it has no field to address. Only a tabled \
             record type can be edited by name",
            node.rtype
        ))
    })?;
    let field = reading
        .fields
        .iter()
        .find(|f| f.path == path)
        .cloned()
        .ok_or_else(|| {
            let names: Vec<&str> = reading.fields.iter().map(|f| f.path.as_str()).collect();
            refuse(format!(
                "record type {:#06x} ({}) read no field `{path}`. It read: {}",
                node.rtype,
                reading.table,
                names.join(", ")
            ))
        })?;
    Ok((reading, field))
}

/// Emit `v` at `kind`'s wire type, or `None` when the two do not go together.
///
/// Every write here is the encoder's own, which is the one the table's write direction uses, so a
/// field-addressed edit and a whole-record re-emission agree about widths and byte order.
fn encode_at(kind: FieldKind, v: &FieldEdit, strings: StringFormat) -> Option<Vec<u8>> {
    /// The narrowing forms carry their width marker in the top bit, so the value has one bit less
    /// than its width.
    fn narrowing_fits(x: i64, wide_bytes: u32) -> Option<u32> {
        let v = u32::try_from(x).ok()?;
        (v < (1u32 << (8 * wide_bytes - 1))).then_some(v)
    }
    let mut enc = Encoder::with_strings(strings);
    match (kind, v) {
        (FieldKind::U8, FieldEdit::Int(x)) => enc.u8(u8::try_from(*x).ok()?),
        (FieldKind::U16Be, FieldEdit::Int(x)) => enc.u16_be(u16::try_from(*x).ok()?),
        (FieldKind::U32Be, FieldEdit::Int(x)) => enc.u32_be(u32::try_from(*x).ok()?),
        (FieldKind::Bool, FieldEdit::Int(x)) => enc.u16_be(u16::try_from(*x).ok()?),
        (FieldKind::I8, FieldEdit::Int(x)) => enc.i8(i8::try_from(*x).ok()?),
        (FieldKind::I16Be, FieldEdit::Int(x)) => enc.i16_be(i16::try_from(*x).ok()?),
        (FieldKind::I32Be, FieldEdit::Int(x)) => enc.i32_be(i32::try_from(*x).ok()?),
        (FieldKind::F32Le, FieldEdit::Float(x)) => enc.f32_le(*x as f32),
        (FieldKind::F32Be, FieldEdit::Float(x)) => enc.f32_be(*x as f32),
        (FieldKind::F64Le, FieldEdit::Float(x)) => enc.f64_le(*x),
        (FieldKind::F64Be, FieldEdit::Float(x)) => enc.f64_be(*x),
        (FieldKind::VarU16, FieldEdit::Int(x)) => enc.narrowing(1, narrowing_fits(*x, 2)?),
        (FieldKind::VarU32, FieldEdit::Int(x)) => enc.narrowing(2, narrowing_fits(*x, 4)?),
        (FieldKind::Text, FieldEdit::Text(s)) => {
            // A string's stored block ends in the NUL its length counts.
            let mut block = s.as_bytes().to_vec();
            block.push(0);
            enc.string(&block);
        }
        (FieldKind::Blob, FieldEdit::Bytes(b)) => enc.blob(b),
        (FieldKind::Skip, FieldEdit::Bytes(b)) => enc.bytes(b),
        _ => return None,
    }
    match enc.finish().as_slice() {
        [Piece::Run(bytes)] => Some(bytes.clone()),
        // Nothing above emits a child record, so anything else is the encoder disagreeing with this
        // function about what it just wrote.
        _ => None,
    }
}

/// True when re-emitting `node`'s reading under its own table reproduces the record's content byte
/// for byte.
///
/// The first half of what makes a tabled record safe to edit: a table that reproduces the record it
/// just read has accounted for every byte of it, so nothing in the record is left for an edit to
/// desynchronize. A record whose table does not round-trip is one the table only partly describes.
pub fn round_trips(node: &RecordNode, logical: &[u8], dialect: Dialect) -> bool {
    let Some(table) = tables::for_record(node.rtype, node.schema, dialect) else {
        return false;
    };
    let content = content_of(node, logical);
    let strings = strings_format_of(node, logical);
    let reading = table::read_strings(table, &content, strings);
    reading.exact() && table::write_as(table, &reading.row, node.schema, strings) == content.pieces
}

/// Read an edited record back out of the bytes about to be written, and require that it says what
/// the edit asked for — the second half of what makes a tabled record safe to edit.
///
/// `node` is the edited record as it stands in `logical`, the replacement stream bytes, and `patch`
/// the [`edit`] that produced them. Its [`before`](FieldPatch::before) reading is what every field
/// other than `path` is held to: they are compared by name and value and not by span, because a
/// width-changing edit moves everything after it and moving is exactly what is allowed.
///
/// # Errors
///
/// [`Error::Edit`] with [`EditErrorKind::EditNotVerified`] when the record no longer reads exactly
/// under its table, `path` does not read back as the value the edit asked for, or any other field's
/// value changed.
pub fn verify_edit(
    node: &RecordNode,
    logical: &[u8],
    dialect: Dialect,
    path: &str,
    value: &FieldEdit,
    patch: &FieldPatch,
) -> Result<()> {
    let Some(after) = read(node, logical, dialect) else {
        return Err(not_verified(format!(
            "editing `{path}` left a record its own table cannot read"
        )));
    };
    if !after.exact() {
        return Err(not_verified(format!(
            "after editing `{path}`, record {:#06x} no longer reads exactly under its table \
             ({} unread byte(s), {} undeclared child record(s))",
            node.rtype, after.unread, after.undeclared_children
        )));
    }
    let Some(edited) = after.fields.iter().find(|f| f.path == path) else {
        return Err(not_verified(format!(
            "editing `{path}` removed it from the record's reading"
        )));
    };
    if edited.joined.len() != patch.bytes.len() || !value.matches(&edited.value, edited.kind) {
        return Err(not_verified(format!(
            "`{path}` did not read back as the value the edit asked for (it reads {:?})",
            edited.value
        )));
    }
    if other_fields(&patch.before, path) != other_fields(&after, path) {
        return Err(not_verified(format!(
            "editing `{path}` changed another field of record {:#06x}: the value it carries \
             decides what the record holds after it",
            node.rtype
        )));
    }
    Ok(())
}

/// A refusal from the read-back check on a written record.
pub(crate) fn not_verified(detail: String) -> Error {
    Error::Edit {
        kind: EditErrorKind::EditNotVerified,
        detail,
    }
}

/// Every field of `r` but the one `path` names, as the (name, value) pairs the read-back check
/// compares.
fn other_fields<'a>(r: &'a RecordFields, path: &str) -> Vec<(&'a str, &'a FieldValue)> {
    r.fields
        .iter()
        .filter(|f| f.path != path)
        .map(|f| (f.path.as_str(), &f.value))
        .collect()
}

/// Projects the row a table read onto the public reading, giving each value the coordinates of the
/// bytes it came from.
///
/// The row is the only source: it carries, per field, the wire type it was read at and the run of
/// joined bytes the cursor consumed for it, so nothing here re-reads the record and nothing can
/// attribute a value to bytes some other field was decoded from.
struct Project<'a> {
    /// The joined runs' byte `i` came from logical offset `map[i]`.
    map: Vec<usize>,
    children: &'a [RecordNode],
    next_child: usize,
    content_end: usize,
    out: Vec<FieldRead>,
}

impl Project<'_> {
    /// The absolute span of a joined-runs region.
    fn absolute(&self, span: Span) -> ByteRange {
        let start = self
            .map
            .get(span.start)
            .copied()
            .unwrap_or(self.content_end);
        let end = if span.end > span.start {
            self.map.get(span.end - 1).map_or(start, |x| x + 1)
        } else {
            start
        };
        ByteRange { start, end }
    }

    /// Emit a [`FieldRead`] per value in `row`, descending into each repeat's rows.
    fn row(&mut self, row: &Row, prefix: &str, depth: usize) {
        for e in row.entries() {
            let path = if prefix.is_empty() {
                e.name.to_string()
            } else {
                format!("{prefix}.{}", e.name)
            };
            let joined = ByteRange {
                start: e.span.start,
                end: e.span.end,
            };
            match (e.kind, &e.value) {
                (Kind::Repeat { .. }, Cell::Seq(rows)) => {
                    let slot = self.out.len();
                    self.out.push(FieldRead {
                        path: path.clone(),
                        name: e.name,
                        depth,
                        kind: FieldKind::Repeat,
                        value: FieldValue::Repeat(rows.len()),
                        span: ByteRange { start: 0, end: 0 },
                        joined,
                    });
                    for (n, inner) in rows.iter().enumerate() {
                        self.row(inner, &format!("{path}[{n}]"), depth + 1);
                    }
                    // The repeat covers its rows: from where it started to the last byte any of
                    // them reached — which a child record can carry past the repeat's own bytes.
                    let start = self.absolute(e.span).start;
                    let end = self.out[slot + 1..]
                        .iter()
                        .map(|r| r.span.end)
                        .max()
                        .unwrap_or(start);
                    self.out[slot].span = ByteRange { start, end };
                }
                // A nested record contributes no field bytes, so its span is its own framed one:
                // the k-th child the table read is the k-th nested node.
                (Kind::Child(_), Cell::Child(c)) => {
                    let node = self.children.get(self.next_child);
                    self.next_child += 1;
                    self.out.push(FieldRead {
                        path,
                        name: e.name,
                        depth,
                        kind: FieldKind::Child,
                        value: FieldValue::Child {
                            rtype: c.rtype,
                            schema: c.schema,
                        },
                        span: node.map_or(self.absolute(e.span), |n| ByteRange {
                            start: n.offset,
                            end: n.content_end,
                        }),
                        joined,
                    });
                }
                (kind, value) => self.out.push(FieldRead {
                    path,
                    name: e.name,
                    depth,
                    kind: FieldKind::from(kind),
                    value: FieldValue::from(value),
                    span: self.absolute(e.span),
                    joined,
                }),
            }
        }
    }
}

/// The public name for a table's wire type. A schema-widened field is recorded at the half its
/// record's version selected, so the declaration that offers both never reaches here.
impl From<Kind> for FieldKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::WidensAt { narrow, .. } => FieldKind::from(narrow.kind()),
            Kind::U8 => FieldKind::U8,
            Kind::U16Be => FieldKind::U16Be,
            Kind::U32Be => FieldKind::U32Be,
            Kind::I8 => FieldKind::I8,
            Kind::I16Be => FieldKind::I16Be,
            Kind::I32Be => FieldKind::I32Be,
            Kind::F32Le => FieldKind::F32Le,
            Kind::F32Be => FieldKind::F32Be,
            Kind::F64Le => FieldKind::F64Le,
            Kind::F64Be => FieldKind::F64Be,
            Kind::VarU16 => FieldKind::VarU16,
            Kind::VarU32 => FieldKind::VarU32,
            Kind::Str => FieldKind::Text,
            Kind::Bool => FieldKind::Bool,
            Kind::Blob => FieldKind::Blob,
            Kind::FieldRef => FieldKind::FieldRef,
            Kind::Skip(_) => FieldKind::Skip,
            Kind::Child(_) => FieldKind::Child,
            Kind::Repeat { .. } => FieldKind::Repeat,
        }
    }
}

/// The public shape of a decoded value: one per family of wire type, so a width is the field's
/// business rather than the value's.
impl From<&Cell> for FieldValue {
    fn from(v: &Cell) -> Self {
        match v {
            Cell::U(x) => FieldValue::Uint(*x),
            Cell::I(x) => FieldValue::Int(*x),
            Cell::F64(x) => FieldValue::Float(*x),
            Cell::F32(x) => FieldValue::Float(f64::from(*x)),
            Cell::Str { text, .. } => FieldValue::Text(text.clone()),
            Cell::Ref {
                text, kind, index, ..
            } => FieldValue::FieldRef {
                name: text.clone(),
                kind: *kind,
                index: *index,
            },
            Cell::Bytes(b) => FieldValue::Bytes(b.clone()),
            Cell::Child(c) => FieldValue::Child {
                rtype: c.rtype,
                schema: c.schema,
            },
            Cell::Seq(rows) => FieldValue::Repeat(rows.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field_table::cursor::StringFormat;

    /// A `0x0088` header: the flag word (a 2-byte length, a schema, the enhanced string form and
    /// the running XOR mask), the schema, and the length field.
    const HEADER: [u8; 6] = [0xb8, 0x88, 0x07, 0x00, 0x00, 0x14];

    /// Build a logical buffer holding one `0x0088 GroupAreaFormat` at `start`, with a child record
    /// spliced into the middle of its field run — the shape whose two coordinates differ.
    fn group_area_format(start: usize) -> (Vec<u8>, RecordNode) {
        let mut logical = vec![0xcc; start];
        // The record's own header sits ahead of its content and declares, among other things, the
        // string wire form the content is framed in.
        if start >= HEADER.len() {
            logical[start - HEADER.len()..start].copy_from_slice(&HEADER);
        }
        // Two flags and the indent, then a six-byte child, then the group limit and an unset
        // field reference.
        let head: [u8; 8] = [0, 1, 0, 1, 0, 0, 0, 9];
        let child: [u8; 6] = [0x51, 0x01, 0, 0, 0, 0];
        let tail: [u8; 12] = [0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0xff, 0xff];
        logical.extend_from_slice(&head);
        let child_at = logical.len();
        logical.extend_from_slice(&child);
        logical.extend_from_slice(&tail);
        let end = logical.len();
        let node = RecordNode {
            rtype: 0x0088,
            schema: 0x0700,
            offset: start.saturating_sub(6),
            content_start: start,
            content_end: end,
            mask: 0,
            children: vec![RecordNode {
                rtype: 0x0151,
                schema: 0x0700,
                offset: child_at,
                content_start: child_at + 6,
                content_end: child_at + 6,
                mask: 0,
                children: Vec::new(),
            }],
        };
        (logical, node)
    }

    #[test]
    fn a_record_type_without_a_table_reads_as_nothing() {
        let (logical, mut node) = group_area_format(0x100);
        node.rtype = 0x7fff;
        assert!(read(&node, &logical, Dialect::Contents).is_none());
        assert!(table_name(0x7fff, Dialect::Contents).is_none());
        assert_eq!(
            table_name(0x0088, Dialect::Contents),
            Some("GroupAreaFormat")
        );
        assert!(!tabled_types(Dialect::Contents).is_empty());
    }

    /// A type number picks a different table in each stream. `0x0003` is the report definition's
    /// saved printer and the query engine's table, and `0x0008` the definition's font and the
    /// engine's index, so each number resolves to two unrelated tables.
    #[test]
    fn the_table_is_chosen_by_dialect_as_well_as_type() {
        assert_eq!(table_name(0x0003, Dialect::Contents), Some("Printer"));
        assert_eq!(table_name(0x0003, Dialect::QeSession), Some("QeTable"));
        assert_eq!(table_name(0x0008, Dialect::Contents), Some("Font"));
        assert_eq!(table_name(0x0008, Dialect::QeSession), Some("QeIndex"));
        // The saved-data catalog is a third vocabulary of its own: `0x0041` is a record there and
        // nowhere else.
        assert_eq!(
            table_name(0x0041, Dialect::Catalog),
            Some("SavedFieldHeader")
        );
        assert_eq!(table_name(0x0041, Dialect::QeSession), None);
    }

    /// Where a stream writes two unrelated records under one number, the schema word decides which,
    /// and the table for one must not be applied to the other.
    #[test]
    fn a_table_does_not_reach_across_versions() {
        let (logical, mut node) = group_area_format(0x100);
        node.rtype = 0x0003;
        assert!(read(&node, &logical, Dialect::Contents).is_some());
        node.schema = 0x0701;
        assert!(read(&node, &logical, Dialect::Contents).is_none());
    }

    /// The property the joined runs cannot express: past a child record, a field's position in the
    /// file is its joined offset plus the child's whole framed length.
    #[test]
    fn a_field_after_a_child_is_reported_at_its_file_position() {
        let start = 0x100;
        let (logical, node) = group_area_format(start);
        let r = read(&node, &logical, Dialect::Contents).expect("0x0088 has a table");
        assert!(r.exact() && r.complete);

        let f = |name: &str| {
            r.fields
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("{name} was read"))
        };
        // Before the child the two coordinates differ only by the content start.
        let indent = f("group_indent");
        assert_eq!(indent.joined, ByteRange { start: 4, end: 8 });
        assert_eq!(
            indent.span,
            ByteRange {
                start: start + 4,
                end: start + 8
            }
        );
        assert_eq!(indent.value, FieldValue::Int(9));

        // The child is reported by its own framed span, and consumes no field bytes.
        let child = f("xml_definition");
        assert_eq!(
            child.span,
            ByteRange {
                start: start + 8,
                end: start + 14
            }
        );
        assert!(child.joined.is_empty());

        // After it, the joined offset is six bytes short of the file position.
        let after = f("visible_groups_per_page");
        assert_eq!(after.value, FieldValue::Int(7));
        assert_eq!(after.joined, ByteRange { start: 8, end: 12 });
        assert_eq!(
            after.span,
            ByteRange {
                start: start + 14,
                end: start + 18
            }
        );
        assert_eq!(after.span.start - after.joined.start, start + 6);
    }

    /// A record that ends early stops the walk where its bytes stop, and says so.
    ///
    /// Cut between the two flags, which every record of the type carries: a record cut after them
    /// is short of nothing, since everything past them is optional.
    #[test]
    fn a_short_record_stops_where_its_bytes_do() {
        let (logical, node) = group_area_format(0);
        let short = RecordNode {
            content_end: 2,
            children: Vec::new(),
            ..node.clone()
        };
        let r = read(&short, &logical, Dialect::Contents).expect("0x0088 has a table");
        assert!(!r.complete);
        assert!(r.stop.ended);
        assert!(r.exact(), "an early end leaves no unread bytes");
        assert_eq!(r.fields.len(), 1);
        assert_eq!(r.read_end(), 2);

        // Past the flags the record carries only fields it need not carry, so stopping there is
        // the record being short of nothing.
        let flags_only = RecordNode {
            content_end: 4,
            children: Vec::new(),
            ..node
        };
        let r = read(&flags_only, &logical, Dialect::Contents).expect("0x0088 has a table");
        assert!(r.complete && r.exact());
        assert_eq!(r.fields.len(), 2);
    }

    /// The string wire form comes from the record's own header, not from its field table: clear the
    /// header's format bit and the same table reads the same bytes as a NUL-terminated string,
    /// which the counted framing does not describe.
    #[test]
    fn the_records_header_selects_the_string_wire_form() {
        let start = 0x100;
        let (logical, node) = group_area_format(start);
        let flag = node.offset;
        assert_eq!(
            crate::field_table::strings_format_of(&node, &logical),
            StringFormat::Enhanced
        );
        let counted = read(&node, &logical, Dialect::Contents).expect("0x0088 has a table");
        assert!(counted.exact());

        let mut simple = logical.clone();
        simple[flag] &= !0x10;
        assert_eq!(
            crate::field_table::strings_format_of(&node, &simple),
            StringFormat::Simple
        );
        let scanned = read(&node, &simple, Dialect::Contents).expect("0x0088 has a table");
        // The formula's own byte count opens with a NUL, so the scanned read ends the string
        // inside it: every field after it lands four bytes early and the record no longer accounts
        // for itself.
        assert!(counted.exact() && scanned.unread == 4);
        let field = |r: &RecordFields, name: &str| {
            r.fields
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.value.clone())
        };
        assert_eq!(
            field(&counted, "new_page_after_formula"),
            Some(FieldValue::FieldRef {
                name: String::new(),
                kind: 0,
                index: None,
            })
        );
        assert_eq!(
            field(&scanned, "new_page_after_formula"),
            Some(FieldValue::FieldRef {
                name: String::new(),
                kind: 0,
                index: Some(1),
            })
        );
    }

    /// An undeclared child blocks the walk: the reading names where it stopped rather than reading
    /// bytes from the far side of it.
    #[test]
    fn an_undeclared_child_is_reported_not_stepped_over() {
        let (logical, node) = group_area_format(0);
        let blocked = RecordNode {
            rtype: 0x00be, // ObjectPosition: two narrowing twips, no child declared
            ..node
        };
        let r = read(&blocked, &logical, Dialect::Contents).expect("0x00be has a table");
        assert!(!r.exact());
        assert!(r.stop.blocked_by_child || r.undeclared_children > 0);
    }
}
