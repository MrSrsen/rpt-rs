//! The declarative field table: what a record's content is, as a sequence of named typed fields.
//!
//! One table describes both directions — the reader walks it to decode, the encoder walks it to
//! emit — so a record's layout is transcribed once instead of once per direction.
//!
//! A [`Kind::Skip`] is deliberately not a field: it names no meaning and keeps its bytes verbatim.
//! It is the honest form for a run whose contents are still unknown, and it stays visible as such.
//!
//! Reading a row is checked against the declaration it was read from, in both halves at once: a
//! [`Row`] accessor fails on a name the table does not declare *and* on a value whose shape the
//! accessor does not read, because a table's field set and each field's wire type are equally
//! constants of the record type and neither can be made to differ by any file. The one absence a
//! lookup answers quietly is a field the record ended before — that is the layout working, and it
//! is what every trailing field relies on. Inspecting a [`Cell`] already in hand is the opposite
//! question and keeps the opposite answer: [`Cell::u`] and its siblings report the shape they find
//! with an `Option` rather than deciding anything about a caller.

use super::cursor::{ChildRef, ContentCursor, Encoder, Piece, RecordContent, Stop, StringFormat};
use crate::codec::Dialect;

/// The index a field reference stores when it names no field.
pub(crate) const UNSET_FIELD_INDEX: u16 = 0xffff;

/// One field's decoded value: the cell a [`Row`] holds under a field's name.
///
/// Signedness is part of the value, not just of the field that produced it: an `I` never masquerades
/// as a `U`, so a table that declares the wrong signedness fails its accessor rather than reporting
/// a negative as a value near the type's ceiling.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cell {
    /// An unsigned scalar (every unsigned fixed-width and every narrowing integer read).
    U(u32),
    /// A signed scalar, widened from whatever width it was stored at.
    I(i32),
    /// A double.
    F64(f64),
    /// A single-precision float, kept at its own width so re-emitting reproduces its bytes.
    F32(f32),
    /// A string: the text before the first NUL, and the exact block as stored.
    Str { text: String, block: Vec<u8> },
    /// A field reference: the referenced field's name (block as stored), and the handle that
    /// resolves it — the pool the field lives in, and its index within that pool, `None` when the
    /// reference names no field.
    Ref {
        text: String,
        block: Vec<u8>,
        kind: u32,
        index: Option<u16>,
    },
    /// An undecoded run, kept verbatim.
    Bytes(Vec<u8>),
    /// A nested record's identity.
    Child(ChildRef),
    /// The rows of a `repeat`.
    Seq(Vec<Row>),
}

impl Cell {
    pub(crate) fn u(&self) -> Option<u32> {
        match self {
            Cell::U(v) => Some(*v),
            _ => None,
        }
    }

    pub(crate) fn i(&self) -> Option<i32> {
        match self {
            Cell::I(v) => Some(*v),
            _ => None,
        }
    }

    /// Any integer value, whatever its signedness — for the places that want the number and not the
    /// wire type, such as a repeat's count.
    pub(crate) fn num(&self) -> Option<i64> {
        match self {
            Cell::U(v) => Some(i64::from(*v)),
            Cell::I(v) => Some(i64::from(*v)),
            _ => None,
        }
    }

    /// Any floating-point value. A single-precision field widens losslessly.
    pub(crate) fn f(&self) -> Option<f64> {
        match self {
            Cell::F64(v) => Some(*v),
            Cell::F32(v) => Some(f64::from(*v)),
            _ => None,
        }
    }

    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Cell::Str { text, .. } | Cell::Ref { text, .. } => Some(text),
            _ => None,
        }
    }

    /// A field reference's handle: the pool it names, and its index within that pool.
    pub(crate) fn handle(&self) -> Option<(u32, Option<u16>)> {
        match self {
            Cell::Ref { kind, index, .. } => Some((*kind, *index)),
            _ => None,
        }
    }

    /// The rows of a `repeat`.
    pub(crate) fn seq(&self) -> Option<&[Row]> {
        match self {
            Cell::Seq(rows) => Some(rows),
            _ => None,
        }
    }

    /// What this value is, in the words the accessors use — so a lookup that does not read it can
    /// say what it found. A shape names the value and not its width: the width is the table's,
    /// and every accessor reads every width its shape has.
    fn shape(&self) -> &'static str {
        match self {
            Cell::U(_) => "an unsigned scalar",
            Cell::I(_) => "a signed scalar",
            Cell::F64(_) | Cell::F32(_) => "a float",
            Cell::Str { .. } => "a string",
            Cell::Ref { .. } => "a field reference",
            Cell::Bytes(_) => "a byte run",
            Cell::Child(_) => "a nested record",
            Cell::Seq(_) => "a repeat's rows",
        }
    }
}

/// Where a field's bytes sit in the record's **joined runs** — the cursor's own coordinate, with
/// every nested record spliced out. A field that occupies no field bytes is empty at the position
/// it was read at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    /// The span of a row assembled by hand: it names the bytes of no record.
    pub(crate) const NONE: Span = Span { start: 0, end: 0 };
}

/// One field a row carries: what it is called, what it was read as, what it says, and where it
/// came from.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub name: &'static str,
    /// The kind the field was **read at**, which for a schema-widened field is the half its
    /// record's version selected rather than the declaration offering both.
    pub kind: Kind,
    pub value: Cell,
    pub span: Span,
}

/// The values a record's field table produced, in table order. Fields the record was too short to
/// reach are simply absent.
///
/// A row also carries the declaration it was read from, so it can tell those absent fields apart
/// from a name that is not a field of this record at all — see [`Row::get`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Row {
    fields: Vec<Entry>,
    /// The fields declared at this level of the table: a table's own for a record's row, a
    /// `repeat` body's for one of its rows. Empty for a row built by hand rather than read.
    declared: &'static [Field],
}

/// Two rows are equal when they carry the same values under the same names; neither the
/// declaration they were read from nor the bytes they came from is part of the reading.
impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.fields.len() == other.fields.len()
            && std::iter::zip(&self.fields, &other.fields)
                .all(|(a, b)| a.name == b.name && a.value == b.value)
    }
}

impl Row {
    /// An empty row that will be filled from `declared`.
    pub(crate) fn declaring(declared: &'static [Field]) -> Self {
        Self {
            fields: Vec::new(),
            declared,
        }
    }

    /// Record a field the table read: its value, and the bytes it came from.
    pub(crate) fn push(&mut self, name: &'static str, kind: Kind, value: Cell, span: Span) {
        self.fields.push(Entry {
            name,
            kind,
            value,
            span,
        });
    }

    /// A field's value, or `None` when the record ended before it.
    ///
    /// A name the row's declaration does not have is **not** an absent value: a record type's field
    /// set is a constant, so no record can make such a name appear, and reporting `None` would read
    /// as a field that decoded empty. Every accessor funnels through here, so every lookup by a
    /// name a table no longer declares fails at the lookup instead of at whatever the default
    /// happens to look like downstream. A name that is present cost nothing to check.
    pub(crate) fn get(&self, name: &str) -> Option<&Cell> {
        match self.fields.iter().rev().find(|e| e.name == name) {
            Some(e) => Some(&e.value),
            None => {
                self.assert_declared(name);
                None
            }
        }
    }

    fn assert_declared(&self, name: &str) {
        // A hand-built row declares nothing and is read back as it was written.
        if self.declared.is_empty() || self.declared.iter().any(|f| f.name == name) {
            return;
        }
        let declared: Vec<&str> = self.declared.iter().map(|f| f.name).collect();
        panic!(
            "no field named `{name}` is declared here; the declaration has: {}",
            declared.join(", ")
        );
    }

    /// A field's value in the shape the accessor reads, or `None` when the record ended before it.
    ///
    /// A value that *is* there in another shape is the caller reading a field at a wire type its
    /// table does not declare — a mistake of exactly the kind [`Row::get`] refuses a name for, and
    /// one no record can produce, so it fails here rather than at whatever this accessor's default
    /// happens to look like downstream. Every accessor funnels through this, so a table that
    /// changes a field's wire type is answered at the first record read by a caller it left behind.
    fn read<'a, T>(
        &'a self,
        name: &str,
        reads: &str,
        as_shape: impl Fn(&'a Cell) -> Option<T>,
    ) -> Option<T> {
        let v = self.get(name)?;
        match as_shape(v) {
            Some(x) => Some(x),
            None => panic!("field `{name}` holds {}, which is not {reads}", v.shape()),
        }
    }

    /// A field's unsigned scalar value, or `0` when the record ended before it.
    pub(crate) fn u(&self, name: &str) -> u32 {
        self.read(name, "an unsigned scalar", Cell::u).unwrap_or(0)
    }

    /// A field's signed scalar value, or `0` when the record ended before it.
    pub(crate) fn i(&self, name: &str) -> i32 {
        self.read(name, "a signed scalar", Cell::i).unwrap_or(0)
    }

    /// A field's integer value regardless of signedness, or `0` when the record ended before it.
    pub(crate) fn num(&self, name: &str) -> i64 {
        self.read(name, "an integer", Cell::num).unwrap_or(0)
    }

    pub(crate) fn text(&self, name: &str) -> &str {
        self.read(name, "a string", Cell::text).unwrap_or("")
    }

    /// A `repeat`'s rows, or nothing when the record ended before the field.
    pub(crate) fn seq(&self, name: &str) -> &[Row] {
        self.read(name, "a repeat's rows", Cell::seq).unwrap_or(&[])
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'static str, &Cell)> {
        self.fields.iter().map(|e| (e.name, &e.value))
    }

    /// The fields as the table read them, with the wire type and the bytes behind each — what a
    /// caller needs to say *where* a value came from as well as what it is.
    pub(crate) fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.fields.iter()
    }
}

/// What a `when` clause and a `repeat` count may branch on: the record's identity, the fields read
/// so far, the enclosing row when inside a repeat, and the repeat index.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub rtype: u16,
    pub schema: u16,
    pub row: &'a Row,
    pub outer: Option<&'a Row>,
    pub index: usize,
}

/// A `when` clause.
pub(crate) type Pred = fn(&Ctx<'_>) -> bool;

/// Everything a walk over one field list carries but the row it is filling: the record's identity,
/// and where in an enclosing repeat the list sits.
///
/// It is what a [`Ctx`] is made of, minus the one part that changes with every field read, so both
/// directions carry one value down the walk instead of four in step.
#[derive(Debug, Clone, Copy)]
struct Frame<'a> {
    rtype: u16,
    schema: u16,
    outer: Option<&'a Row>,
    index: usize,
}

impl<'a> Frame<'a> {
    /// The frame a record's own fields are walked in.
    const fn top(rtype: u16, schema: u16) -> Self {
        Self {
            rtype,
            schema,
            outer: None,
            index: 0,
        }
    }

    /// The frame one row of a repeat body is walked in: the row enclosing it, and which row it is.
    const fn nested(self, outer: &'a Row, index: usize) -> Self {
        Self {
            outer: Some(outer),
            index,
            ..self
        }
    }

    /// What a predicate sees: this frame, with the row read so far.
    fn ctx<'b>(&self, row: &'b Row) -> Ctx<'b>
    where
        'a: 'b,
    {
        Ctx {
            rtype: self.rtype,
            schema: self.schema,
            row,
            outer: self.outer,
            index: self.index,
        }
    }
}

/// When a field is present in a record.
///
/// Not every field is written by every record of its type. A record may simply stop early, and a
/// reader that keeps going reads the *next* record's bytes as this one's fields; a record may also
/// carry a field only from some version of its layout on. Both are part of the layout, so both are
/// stated in the table rather than inferred from a length.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum Presence {
    /// Every record of this type carries it.
    #[default]
    Always,
    /// Only while the record still has content left.
    ///
    /// The trailing fields a writer appends are guarded this way: a record written before the
    /// field existed simply ends, and its absence is the layout working rather than a short read.
    IfMoreContent,
    /// From this schema version on.
    ///
    /// A schema word is a version with a numeric ordering, so a field added in one version is
    /// present at that version and every later one.
    FromSchema(u16),
    /// At this schema version only — an **alternative** layout, not an addition: one version reads
    /// these fields and every other version reads the ones beside them.
    OnlyAtSchema(u16),
    /// While a predicate over the record read so far holds.
    When(Pred),
}

/// How many times a `repeat` body runs.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Count {
    Fixed(usize),
    /// Taken from an earlier field's value.
    FromField(&'static str),
}

/// A field's wire type.
///
/// Every multi-byte kind names its byte order, because the format has both: a bare width would mean
/// big-endian by convention alone, and a field written at the wrong order would read as a plausible
/// number rather than as a mistake. A single byte has no order and so carries no suffix.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    /// Fixed-width unsigned integers.
    U8,
    U16Be,
    U32Be,
    /// Fixed-width two's-complement integers. The record layer reads each width in both
    /// signednesses, and the two are not interchangeable: a negative read as unsigned lands near the
    /// type's ceiling instead.
    I8,
    I16Be,
    I32Be,
    /// A single-precision float, little-endian.
    F32Le,
    /// A single-precision float, big-endian.
    F32Be,
    /// A double, little-endian.
    F64Le,
    /// A double, big-endian — the order the chart definition's axis min/max values are stored in,
    /// and the one that matches every other scalar in the format.
    F64Be,
    /// The **narrowing** integers, whose width follows their magnitude: [`Kind::VarU16`] is 1 byte,
    /// or 2 with `0x80` set, and [`Kind::VarU32`] is 2 bytes, or 4. A twip is a [`Kind::VarU32`].
    ///
    /// The wide form carries its marker in the top bit, so the value is unsigned by construction and
    /// the encoding is not LEB128 — only the variable width is shared with one.
    VarU16,
    VarU32,
    /// A string, in whichever of the two wire forms the record's own header declared. The choice is
    /// the record's, not the table's, which is why this kind names no encoding.
    Str,
    /// A **boolean**, two bytes big-endian — the width the query engine's archive routes a boolean
    /// through, so a table that reads one as a byte takes the next field's first byte with it.
    Bool,
    /// A **blob**: a big-endian `u32` byte count and that many raw bytes.
    ///
    /// It is a string's framing without a string's terminator, and the difference is the count:
    /// a string's includes its trailing NUL, a blob's is the bytes themselves. Nothing in a blob is
    /// text, so nothing here reads it as any.
    Blob,
    /// A **field reference**: a string naming a field, then the handle that resolves it — the pool
    /// the field lives in (a narrowing enum) and its index within that pool, [`UNSET_FIELD_INDEX`]
    /// when it names none.
    ///
    /// It is one composite rather than three entries because it is one thing the record layer
    /// writes and reads as a unit, and because its unset form is eight bytes of `00`s and `ff`s
    /// that a fixed-width run will happily swallow — a reference that ever names a field then moves
    /// every field after it.
    FieldRef,
    /// A run of bytes with no known meaning, kept verbatim.
    Skip(usize),
    /// A nested record, expected here in the sequence.
    Child(u16),
    /// A field whose **width follows the record's schema**: `narrow` below `at`, `wide` from it.
    ///
    /// A record's schema is a version, and a version can widen a field rather than add one — the
    /// writer picks the schema per record instance from the value it is storing, narrow while the
    /// value fits and wide otherwise. Declaring both halves as one entry keeps the field a single
    /// name, which is what every consumer reads it by.
    WidensAt {
        at: u16,
        narrow: Width,
        wide: Width,
    },
    /// A repeated body.
    Repeat {
        count: Count,
        body: &'static [Field],
    },
}

/// One of the fixed-width scalars, as the two halves of a [`Kind::WidensAt`] name them.
///
/// A version widens a field between two widths, so each half is a width and nothing else: the
/// variable-length kinds are absent, and so is [`Kind::WidensAt`] itself — a half that widened
/// again would be a shape this vocabulary does not offer, and one no reader could resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Width {
    U8,
    U16Be,
    U32Be,
    I8,
    I16Be,
    I32Be,
    F32Le,
    F32Be,
    F64Le,
    F64Be,
}

impl Width {
    /// The wire type this width is read and written at.
    pub(crate) const fn kind(self) -> Kind {
        match self {
            Width::U8 => Kind::U8,
            Width::U16Be => Kind::U16Be,
            Width::U32Be => Kind::U32Be,
            Width::I8 => Kind::I8,
            Width::I16Be => Kind::I16Be,
            Width::I32Be => Kind::I32Be,
            Width::F32Le => Kind::F32Le,
            Width::F32Be => Kind::F32Be,
            Width::F64Le => Kind::F64Le,
            Width::F64Be => Kind::F64Be,
        }
    }
}

impl Kind {
    /// The wire type a record of version `schema` carries this field at.
    ///
    /// Only a schema-widened field has anything to resolve; every other kind is itself. Both
    /// directions resolve here, so a record is written back at the width it was read at.
    pub(crate) const fn resolve(self, schema: u16) -> Kind {
        match self {
            Kind::WidensAt { at, narrow, wide } => {
                if schema < at {
                    narrow.kind()
                } else {
                    wide.kind()
                }
            }
            k => k,
        }
    }
}

/// One entry of a field table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Field {
    pub name: &'static str,
    pub kind: Kind,
    /// When the record carries it at all.
    pub presence: Presence,
}

impl Field {
    /// A field every record of the type carries.
    pub(crate) const fn new(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            kind,
            presence: Presence::Always,
        }
    }

    /// A field read only while the predicate holds.
    pub(crate) const fn when(name: &'static str, kind: Kind, p: Pred) -> Self {
        Self {
            name,
            kind,
            presence: Presence::When(p),
        }
    }

    /// A field read only while the record still has content left.
    pub(crate) const fn optional(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            kind,
            presence: Presence::IfMoreContent,
        }
    }

    /// A field carried from `schema` on.
    pub(crate) const fn from_schema(name: &'static str, kind: Kind, schema: u16) -> Self {
        Self {
            name,
            kind,
            presence: Presence::FromSchema(schema),
        }
    }

    /// A field carried at `schema` alone, as the alternative to the fields beside it.
    pub(crate) const fn only_at_schema(name: &'static str, kind: Kind, schema: u16) -> Self {
        Self {
            name,
            kind,
            presence: Presence::OnlyAtSchema(schema),
        }
    }
}

/// A record type's content, declared as a sequence of fields.
///
/// A type number alone does not identify a record: each stream numbers its own vocabulary, so
/// `0x0008` is a font in the report definition and an index in the query engine's schema. The
/// dialect is therefore part of the table's identity, and everything keyed by record type — the
/// registry lookup, the version ceiling — is keyed by the pair.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Table {
    pub dialect: Dialect,
    pub rtype: u16,
    pub name: &'static str,
    pub fields: &'static [Field],
}

/// The result of walking a table over a record.
#[derive(Debug, Clone)]
pub(crate) struct Reading {
    pub row: Row,
    /// Field bytes the table did not account for. Anything but `0` means the table is wrong about
    /// this record, and is a loud failure rather than a tolerance.
    pub unread: usize,
    /// Undeclared children left in the content.
    pub undeclared_children: usize,
    pub stop: Stop,
    /// Every field in the table was reached.
    pub complete: bool,
    /// The record's schema is newer than the newest layout this reader knows for its type, so the
    /// table was not walked at all and `row` is empty.
    ///
    /// A schema is a version: a newer one may have widened a field or added one mid-sequence, and
    /// every field after the change would then be read at the wrong offset. Refusing is the only
    /// safe answer, and it is what the format's own reader does.
    pub schema_too_new: bool,
}

impl Reading {
    /// The table describes this record exactly: no bytes left over, no undeclared child, and no
    /// read blocked by one.
    pub(crate) fn exact(&self) -> bool {
        !self.schema_too_new
            && self.unread == 0
            && self.undeclared_children == 0
            && !self.stop.blocked_by_child
            && !self.stop.child_mismatch
    }
}

/// Walk `table` over `content`, decoding every field it declares, framing strings as the record's
/// own header declared.
pub(crate) fn read_strings(
    table: &Table,
    content: &RecordContent,
    strings: StringFormat,
) -> Reading {
    // A record newer than the newest layout known for its type is refused rather than decoded: the
    // fields may have been widened or inserted since, and every field after the change would be
    // read at the wrong offset. A type with no established maximum is read as before. The ceiling
    // belongs to the table's own dialect: the record another stream writes under the same number
    // has its own history and its own versions.
    if super::tables::max_supported_schema(content.rtype, table.dialect)
        .is_some_and(|max| content.schema > max)
    {
        return Reading {
            row: Row::declaring(table.fields),
            unread: content.field_byte_len(),
            undeclared_children: 0,
            stop: Stop::default(),
            complete: false,
            schema_too_new: true,
        };
    }
    let mut cur = ContentCursor::with_strings(content, strings);
    let mut row = Row::declaring(table.fields);
    let frame = Frame::top(content.rtype, content.schema);
    let complete = read_fields(table.fields, &mut cur, &mut row, frame);
    let (unread, undeclared_children) = cur.leftover();
    Reading {
        unread,
        undeclared_children,
        stop: cur.stop(),
        complete,
        row,
        schema_too_new: false,
    }
}

/// A string block's text: everything before its terminating NUL.
///
/// The table names the block, so whatever is in it is the stored value and is read as one: an empty
/// string is an answer rather than a failure, and bytes that are not valid UTF-8 are rendered
/// lossily rather than rejected. The plausibility rules in [`crate::bytes`] are the opposite trade,
/// and belong to a search, which has no declaration to read against.
pub(crate) fn text_of(block: &[u8]) -> String {
    String::from_utf8_lossy(block.split(|&c| c == 0).next().unwrap_or(block)).into_owned()
}

/// Read one field reference: the name, the pool, and the index that resolves it.
pub(crate) fn read_field_ref(cur: &mut ContentCursor<'_>) -> Option<Cell> {
    let block = cur.string()?.to_vec();
    let kind = cur.narrowing(1)?;
    let index = cur.u16_be()?;
    Some(Cell::Ref {
        text: text_of(&block),
        block,
        kind,
        index: (index != UNSET_FIELD_INDEX).then_some(index),
    })
}

/// Emit one field reference, reproducing the unset index it was read from.
pub(crate) fn write_field_ref(v: &Cell, enc: &mut Encoder) {
    if let Cell::Ref {
        block, kind, index, ..
    } = v
    {
        enc.string(block);
        enc.narrowing(1, *kind);
        enc.u16_be(index.unwrap_or(UNSET_FIELD_INDEX));
    }
}

/// Whether a record of version `schema` carries a field declared with this presence.
///
/// Only the presences that follow the record's **version** are decided here; a presence that
/// follows the record's *content* admits the field and is settled by the caller, which is the half
/// that differs between reading and writing. Sharing this keeps the two directions from disagreeing
/// about which version a field belongs to.
fn version_admits(presence: Presence, schema: u16) -> bool {
    match presence {
        Presence::FromSchema(v) => schema >= v,
        Presence::OnlyAtSchema(v) => schema == v,
        _ => true,
    }
}

/// The field bytes between `at` and where the cursor now stands — one field's run.
fn span(at: usize, cur: &ContentCursor<'_>) -> Span {
    Span {
        start: at,
        end: cur.pos(),
    }
}

fn read_fields(
    fields: &'static [Field],
    cur: &mut ContentCursor<'_>,
    row: &mut Row,
    frame: Frame<'_>,
) -> bool {
    for f in fields {
        let present = version_admits(f.presence, frame.schema)
            && match f.presence {
                // Matches the engine's own field-presence rule: a nested record counts as content too.
                Presence::IfMoreContent => !cur.at_end(),
                Presence::When(p) => p(&frame.ctx(row)),
                _ => true,
            };
        if !present {
            continue;
        }
        // A schema-widened field resolves to one of its two halves before anything is read.
        let kind = f.kind.resolve(frame.schema);
        // Where this field's bytes start, so the row records the run each value was read from and
        // no second walk has to reconstruct it.
        let at = cur.pos();
        let v = match kind {
            Kind::U8 => cur.u8().map(|v| Cell::U(u32::from(v))),
            Kind::U16Be => cur.u16_be().map(|v| Cell::U(u32::from(v))),
            Kind::U32Be => cur.u32_be().map(Cell::U),
            Kind::I8 => cur.i8().map(|v| Cell::I(i32::from(v))),
            Kind::I16Be => cur.i16_be().map(|v| Cell::I(i32::from(v))),
            Kind::I32Be => cur.i32_be().map(Cell::I),
            Kind::F32Le => cur.f32_le().map(Cell::F32),
            Kind::F32Be => cur.f32_be().map(Cell::F32),
            Kind::F64Le => cur.f64_le().map(Cell::F64),
            Kind::F64Be => cur.f64_be().map(Cell::F64),
            Kind::VarU16 => cur.narrowing(1).map(Cell::U),
            Kind::VarU32 => cur.narrowing(2).map(Cell::U),
            Kind::Str => cur.string().map(|b| Cell::Str {
                text: text_of(b),
                block: b.to_vec(),
            }),
            Kind::Bool => cur.u16_be().map(|v| Cell::U(u32::from(v))),
            Kind::Blob => cur.blob().map(|b| Cell::Bytes(b.to_vec())),
            Kind::FieldRef => read_field_ref(cur),
            Kind::Skip(n) => cur.take(n).map(|b| Cell::Bytes(b.to_vec())),
            Kind::Child(rt) => cur.child_expect(rt).map(|c| Cell::Child(c.clone())),
            Kind::Repeat { count, body } => {
                let n = match count {
                    Count::Fixed(n) => n,
                    // A count field's signedness is the table's business, not the repeat's; a
                    // negative count is no rows.
                    Count::FromField(name) => row.num(name).max(0) as usize,
                };
                let mut rows = Vec::with_capacity(n);
                let mut ok = true;
                for i in 0..n {
                    let mut inner = Row::declaring(body);
                    ok &= read_fields(body, cur, &mut inner, frame.nested(row, i));
                    rows.push(inner);
                    if !ok {
                        break;
                    }
                }
                if ok {
                    Some(Cell::Seq(rows))
                } else {
                    row.push(f.name, kind, Cell::Seq(rows), span(at, cur));
                    return false;
                }
            }
            // Resolved to one of its halves above; a nested one is not a shape this vocabulary
            // offers.
            Kind::WidensAt { .. } => None,
        };
        match v {
            Some(v) => row.push(f.name, kind, v, span(at, cur)),
            // The record ran out (or a child blocks the way): stop here and leave the rest at
            // their defaults, exactly as a straight-line reader does.
            None => return false,
        }
    }
    true
}

/// Emit the pieces `row` describes under `table` — the write direction of the same declaration —
/// as a record of version `schema` whose strings are framed in `strings`.
///
/// The version is what every schema-selected field is resolved against — both which fields the
/// record carries and how wide they are — so emitting a record at a version other than the one it
/// was read at reproduces that version's layout, not its own.
pub(crate) fn write_as(table: &Table, row: &Row, schema: u16, strings: StringFormat) -> Vec<Piece> {
    let mut enc = Encoder::with_strings(strings);
    write_fields(table.fields, row, Frame::top(table.rtype, schema), &mut enc);
    enc.finish()
}

fn write_fields(fields: &'static [Field], row: &Row, frame: Frame<'_>, enc: &mut Encoder) {
    for f in fields {
        // A version decides presence here exactly as it does when reading, so emitting a row at a
        // version yields that version's layout rather than the one the row was read at. A
        // data-driven condition is re-evaluated; a presence the record's own *length* decided is
        // read back off the row, which is where the reader's decision was recorded.
        if !version_admits(f.presence, frame.schema) {
            continue;
        }
        if let Presence::When(p) = f.presence {
            if !p(&frame.ctx(row)) {
                continue;
            }
        }
        let v = match row.get(f.name) {
            Some(v) => v,
            // A field the record was too short to reach is not emitted; the record ends where the
            // values do. A field the record simply does not carry is passed over instead, since
            // the fields after it may still be present.
            None if matches!(f.presence, Presence::Always) => return,
            None => continue,
        };
        let kind = f.kind.resolve(frame.schema);
        match (kind, v) {
            (Kind::U8, Cell::U(x)) => enc.u8(*x as u8),
            (Kind::U16Be, Cell::U(x)) => enc.u16_be(*x as u16),
            (Kind::U32Be, Cell::U(x)) => enc.u32_be(*x),
            (Kind::I8, Cell::I(x)) => enc.i8(*x as i8),
            (Kind::I16Be, Cell::I(x)) => enc.i16_be(*x as i16),
            (Kind::I32Be, Cell::I(x)) => enc.i32_be(*x),
            (Kind::F32Le, Cell::F32(x)) => enc.f32_le(*x),
            (Kind::F32Be, Cell::F32(x)) => enc.f32_be(*x),
            (Kind::F64Le, Cell::F64(x)) => enc.f64_le(*x),
            (Kind::F64Be, Cell::F64(x)) => enc.f64_be(*x),
            (Kind::VarU16, Cell::U(x)) => enc.narrowing(1, *x),
            (Kind::VarU32, Cell::U(x)) => enc.narrowing(2, *x),
            (Kind::Str, Cell::Str { block, .. }) => enc.string(block),
            (Kind::Bool, Cell::U(x)) => enc.u16_be(*x as u16),
            (Kind::Blob, Cell::Bytes(b)) => enc.blob(b),
            (Kind::FieldRef, v @ Cell::Ref { .. }) => write_field_ref(v, enc),
            (Kind::Skip(_), Cell::Bytes(b)) => enc.bytes(b),
            (Kind::Child(_), Cell::Child(c)) => enc.child(c.clone()),
            (Kind::Repeat { body, .. }, Cell::Seq(rows)) => {
                for (i, inner) in rows.iter().enumerate() {
                    write_fields(body, inner, frame.nested(row, i), enc);
                }
            }
            // A value of the wrong shape for its declared kind: emit nothing rather than
            // guessing, so a round-trip check fails loudly.
            _ => return,
        }
    }
}

#[cfg(test)]
mod tests {
    //! A record built here carries no header to declare a string form, so every reading and
    //! re-emission names the **enhanced** form — the one the record-tree reader admits — rather
    //! than leaving it assumed.

    use super::*;
    use crate::field_table::cursor::Piece;

    /// A synthetic record exercising every construct the vocabulary offers: a narrowing integer
    /// whose width follows its magnitude, a conditional field, a repeat whose length comes from an
    /// earlier field and whose body has an index-conditional entry, and a trailing double.
    const ELEMENT: &[Field] = &[
        Field::new("weight", Kind::U16Be),
        Field::new("italic", Kind::U8),
        Field::new("_pad", Kind::Skip(2)),
        Field::when("_wide_entry", Kind::Skip(1), |c| c.index == 3),
        // A body field may also branch on the enclosing row and on the record's own identity.
        Field::when("_never", Kind::Skip(1), |c| {
            c.rtype == 0 && c.schema == 0 && c.outer.is_some_and(|o| o.u("count") == 0)
        }),
    ];

    fn pie(c: &Ctx<'_>) -> bool {
        matches!(c.row.u("kind"), 3 | 4)
    }

    const SYNTHETIC: Table = Table {
        dialect: Dialect::Contents,
        rtype: 0x7fff,
        name: "Synthetic",
        fields: &[
            Field::new("kind", Kind::VarU16),
            Field::when("detach", Kind::U8, pie),
            Field::new("count", Kind::U8),
            Field::new(
                "elements",
                Kind::Repeat {
                    count: Count::FromField("count"),
                    body: ELEMENT,
                },
            ),
            Field::new("angle", Kind::F64Le),
        ],
    };

    fn content(bytes: &[u8]) -> RecordContent {
        RecordContent {
            rtype: 0x7fff,
            schema: 0x0700,
            pieces: vec![Piece::Run(bytes.to_vec())],
        }
    }

    const F64_1_5: [u8; 8] = [0, 0, 0, 0, 0, 0, 0xf8, 0x3f];

    #[test]
    fn a_conditional_field_moves_the_cursor_rather_than_an_offset() {
        // kind 0: no `detach`, two elements.
        let mut plain = vec![0x00, 0x02];
        plain.extend_from_slice(&[0x01, 0x90, 0x01, 0, 0]);
        plain.extend_from_slice(&[0x01, 0x2c, 0x00, 0, 0]);
        plain.extend_from_slice(&F64_1_5);
        let c = content(&plain);
        let r = read_strings(&SYNTHETIC, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!(r.row.u("kind"), 0);
        assert!(r.row.get("detach").is_none());
        assert_eq!(r.row.u("count"), 2);
        assert_eq!(
            write_as(&SYNTHETIC, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );

        // kind 3: the conditional field is present, and everything after it shifts by one byte.
        let mut pie = vec![0x03, 0x07, 0x01];
        pie.extend_from_slice(&[0x02, 0x58, 0x01, 0, 0]);
        pie.extend_from_slice(&F64_1_5);
        let c = content(&pie);
        let r = read_strings(&SYNTHETIC, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!(r.row.u("detach"), 7);
        assert_eq!(
            r.row
                .get("elements")
                .map(|v| matches!(v, Cell::Seq(s) if s.len() == 1)),
            Some(true)
        );
        assert_eq!(
            write_as(&SYNTHETIC, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );
    }

    #[test]
    fn a_repeat_body_may_branch_on_its_index() {
        // Four elements: the fourth carries one extra byte.
        let mut b = vec![0x00, 0x04];
        for i in 0..4u8 {
            b.extend_from_slice(&[0x00, i, 0x00, 0, 0]);
        }
        b.push(0xff); // the index-3 extra
        b.extend_from_slice(&F64_1_5);
        let c = content(&b);
        let r = read_strings(&SYNTHETIC, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete, "{r:?}");
        let Some(Cell::Seq(rows)) = r.row.get("elements") else {
            panic!("elements is a sequence")
        };
        assert_eq!(rows.len(), 4);
        assert!(rows[0].get("_wide_entry").is_none());
        assert!(rows[3].get("_wide_entry").is_some());
        assert_eq!(
            write_as(&SYNTHETIC, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );
    }

    /// The narrowing escape is a width change, not a flag: a wide leading value shifts everything
    /// after it, and the table still lands on it.
    #[test]
    fn a_wide_narrowing_value_shifts_the_rest_of_the_record() {
        let mut b = vec![0x80, 0x82, 0x01]; // kind = 130, one element
        b.extend_from_slice(&[0x01, 0x90, 0x00, 0, 0]);
        b.extend_from_slice(&F64_1_5);
        let c = content(&b);
        let r = read_strings(&SYNTHETIC, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!(r.row.u("kind"), 130);
        assert_eq!(r.row.u("count"), 1);
        assert_eq!(r.row.get("angle"), Some(&Cell::F64(1.5)));
        // And the writer picks the same form back.
        assert_eq!(
            write_as(&SYNTHETIC, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );
    }

    /// A record that ends early leaves its trailing fields at their defaults — the mechanism behind
    /// every optional trailing field, with no special case in the table.
    #[test]
    fn a_short_record_simply_stops() {
        let c = content(&[0x00, 0x00]); // kind, count = 0, and nothing else
        let r = read_strings(&SYNTHETIC, &c, StringFormat::Enhanced);
        assert!(!r.complete);
        assert!(r.stop.ended);
        assert_eq!(r.unread, 0);
        assert_eq!(r.row.u("angle"), 0);
        // Re-emitting stops where the values stop, reproducing the short record.
        assert_eq!(
            write_as(&SYNTHETIC, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );
    }

    /// The escape hatch: a field this vocabulary has no type for — here a bare NUL-terminated
    /// string, which carries no length prefix — handed to hand-written code and counted as such.
    /// A `repeat` may also take a fixed length rather than one read from the record.
    #[test]
    fn a_repeat_may_have_a_fixed_length() {
        const PAIR: &[Field] = &[Field::new("v", Kind::U8)];
        const T: Table = Table {
            dialect: Dialect::Contents,
            rtype: 0x7ffc,
            name: "Fixed",
            fields: &[Field::new(
                "pair",
                Kind::Repeat {
                    count: Count::Fixed(2),
                    body: PAIR,
                },
            )],
        };
        let c = RecordContent {
            rtype: 0x7ffc,
            schema: 0x0700,
            pieces: vec![Piece::Run(vec![7, 9])],
        };
        let r = read_strings(&T, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        let Some(Cell::Seq(rows)) = r.row.get("pair") else {
            panic!("a sequence")
        };
        assert_eq!((rows[0].u("v"), rows[1].u("v")), (7, 9));
        assert_eq!(
            write_as(&T, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );
    }

    /// A table of one field, so a kind can be exercised on its own bytes.
    fn one(fields: &'static [Field]) -> Table {
        Table {
            dialect: Dialect::Contents,
            rtype: 0x7ff0,
            name: "One",
            fields,
        }
    }

    const I8_V: &[Field] = &[Field::new("v", Kind::I8)];
    const I16_V: &[Field] = &[Field::new("v", Kind::I16Be)];
    const I32_V: &[Field] = &[Field::new("v", Kind::I32Be)];
    const U32_V: &[Field] = &[Field::new("v", Kind::U32Be)];
    const F32LE_V: &[Field] = &[Field::new("v", Kind::F32Le)];
    const F32BE_V: &[Field] = &[Field::new("v", Kind::F32Be)];

    /// The whole point of the signed kinds: the same bytes mean different numbers, and the
    /// top-bit-set values that real records carry are the ones that diverge.
    #[test]
    fn a_signed_field_is_not_its_unsigned_reading() {
        const SENTINEL: [u8; 4] = [0x80, 0x00, 0x00, 0x00];
        let unsigned = read_strings(&one(U32_V), &content(&SENTINEL), StringFormat::Enhanced);
        assert_eq!(unsigned.row.get("v"), Some(&Cell::U(0x8000_0000)));
        assert_eq!(unsigned.row.u("v"), 2_147_483_648);

        let signed = read_strings(&one(I32_V), &content(&SENTINEL), StringFormat::Enhanced);
        assert_eq!(signed.row.get("v"), Some(&Cell::I(i32::MIN)));
        assert_eq!(signed.row.i("v"), i32::MIN);

        // A value in hand reports the shape it has: asking a signed one for its unsigned reading is
        // a question, and `None` is its answer.
        assert_eq!(signed.row.get("v").and_then(Cell::u), None);
        // `num` is the accessor that does not care, and it widens both without loss.
        assert_eq!(unsigned.row.num("v"), 2_147_483_648);
        assert_eq!(signed.row.num("v"), i64::from(i32::MIN));
    }

    /// Reading a field at the wrong signedness is a mistake in the caller, not a value: it is
    /// refused where a name the table does not declare is, rather than reading as the accessor's
    /// default on every record for as long as the two disagree.
    #[test]
    #[should_panic(expected = "field `v` holds a signed scalar, which is not an unsigned scalar")]
    fn a_field_read_at_the_wrong_signedness_fails_at_the_accessor() {
        let signed = read_strings(
            &one(I32_V),
            &content(&[0x80, 0x00, 0x00, 0x00]),
            StringFormat::Enhanced,
        );
        let _ = signed.row.u("v");
    }

    /// The same for every other shape: an accessor reads the values its shape covers and refuses
    /// the rest, so no field can be read as a kind of thing it is not.
    #[test]
    #[should_panic(expected = "field `v` holds an unsigned scalar, which is not a string")]
    fn a_scalar_field_read_as_text_fails_at_the_accessor() {
        let unsigned = read_strings(
            &one(U32_V),
            &content(&[0x80, 0x00, 0x00, 0x00]),
            StringFormat::Enhanced,
        );
        let _ = unsigned.row.text("v");
    }

    /// Every signed width round-trips its own extremes byte for byte — the property a table
    /// transcription rests on.
    #[test]
    fn signed_boundaries_round_trip() {
        let cases: &[(&'static [Field], i32, &[u8])] = &[
            (I8_V, -1, &[0xff]),
            (I8_V, i32::from(i8::MIN), &[0x80]),
            (I8_V, i32::from(i8::MAX), &[0x7f]),
            (I16_V, -1, &[0xff, 0xff]),
            (I16_V, i32::from(i16::MIN), &[0x80, 0x00]),
            (I16_V, -720, &[0xfd, 0x30]),
            (I32_V, -1, &[0xff, 0xff, 0xff, 0xff]),
            (I32_V, i32::MIN, &[0x80, 0x00, 0x00, 0x00]),
            (I32_V, i32::MAX, &[0x7f, 0xff, 0xff, 0xff]),
            (I32_V, -1440, &[0xff, 0xff, 0xfa, 0x60]),
        ];
        for &(fields, want, bytes) in cases {
            let t = one(fields);
            let c = content(bytes);
            let r = read_strings(&t, &c, StringFormat::Enhanced);
            assert!(r.exact() && r.complete, "{bytes:02x?}");
            assert_eq!(r.row.i("v"), want, "{bytes:02x?}");
            assert_eq!(
                write_as(&t, &r.row, 0x0700, StringFormat::Enhanced),
                c.pieces,
                "{bytes:02x?}"
            );
        }
    }

    /// A single-precision float keeps its own width, so re-emitting reproduces its four bytes
    /// rather than a rounded double.
    #[test]
    fn a_single_precision_float_round_trips_at_its_own_width() {
        // 0.1 has no exact binary form: a widen-and-narrow that went through `f64` would not land
        // back on the same bits by accident.
        for (fields, bytes) in [
            (F32LE_V, [0xcd, 0xcc, 0xcc, 0x3d]),
            (F32BE_V, [0x3d, 0xcc, 0xcc, 0xcd]),
        ] {
            let t = one(fields);
            let c = content(&bytes);
            let r = read_strings(&t, &c, StringFormat::Enhanced);
            assert!(r.exact() && r.complete, "{bytes:02x?}");
            assert_eq!(r.row.get("v"), Some(&Cell::F32(0.1)), "{bytes:02x?}");
            // The widening accessor reports it as a number, at the value an f32 actually holds.
            assert_eq!(
                r.row.get("v").and_then(Cell::f),
                Some(f64::from(0.1f32)),
                "{bytes:02x?}"
            );
            assert_eq!(
                write_as(&t, &r.row, 0x0700, StringFormat::Enhanced),
                c.pieces,
                "{bytes:02x?}"
            );
        }
    }

    /// A repeat may be counted by a signed field: the count is the number, not its wire type.
    #[test]
    fn a_signed_count_drives_a_repeat() {
        const BODY: &[Field] = &[Field::new("v", Kind::U8)];
        const T: Table = Table {
            dialect: Dialect::Contents,
            rtype: 0x7ff1,
            name: "SignedCount",
            fields: &[
                Field::new("count", Kind::I16Be),
                Field::new(
                    "rows",
                    Kind::Repeat {
                        count: Count::FromField("count"),
                        body: BODY,
                    },
                ),
            ],
        };
        let c = content(&[0x00, 0x02, 7, 9]);
        let r = read_strings(&T, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!(r.row.seq("rows").len(), 2);
        assert_eq!(
            write_as(&T, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );

        // A negative count is no rows rather than a wrap to billions.
        let c = content(&[0xff, 0xff]);
        let r = read_strings(&T, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!(r.row.i("count"), -1);
        assert!(r.row.seq("rows").is_empty());
    }

    /// A record newer than the newest layout known for its type is refused, not decoded at the
    /// wrong offsets. `0x0071` has never changed shape, so `0x0700` is its maximum.
    #[test]
    fn a_schema_newer_than_the_table_knows_is_refused() {
        const NAMED_VALUE: Table = Table {
            dialect: Dialect::Contents,
            rtype: 0x0071,
            name: "NamedValue",
            fields: &[Field::new("_a", Kind::U16Be)],
        };
        let at = |schema| RecordContent {
            rtype: 0x0071,
            schema,
            pieces: vec![Piece::Run(vec![0x12, 0x34])],
        };

        let r = read_strings(&NAMED_VALUE, &at(0x0700), StringFormat::Enhanced);
        assert!(r.exact() && !r.schema_too_new);
        assert_eq!(r.row.u("_a"), 0x1234);

        // One version newer: nothing is decoded, and the reading says why.
        let r = read_strings(&NAMED_VALUE, &at(0x0701), StringFormat::Enhanced);
        assert!(r.schema_too_new);
        assert!(!r.exact());
        assert!(r.row.get("_a").is_none());

        // A type with no established maximum is read as before.
        assert!(super::super::tables::max_supported_schema(0x7fff, Dialect::Contents).is_none());
        assert!(
            !read_strings(
                &SYNTHETIC,
                &content(&[0x00, 0x00, F64_1_5[0]]),
                StringFormat::Enhanced
            )
            .schema_too_new
        );
    }

    /// A maximum belongs to one dialect's record, not to the number it is written under.
    ///
    /// `0x0008` is the report definition's font, whose layout is settled at `0x0700`, and the query
    /// engine's index, written at that stream's own far higher versions. A ceiling read by number
    /// alone would refuse every one of the latter.
    #[test]
    fn a_maximum_belongs_to_the_dialect_that_established_it() {
        use super::super::tables::max_supported_schema;
        assert_eq!(
            max_supported_schema(0x0008, Dialect::Contents),
            Some(0x0700)
        );
        for d in [
            Dialect::QeSession,
            Dialect::Catalog,
            Dialect::ReportParameters,
        ] {
            assert!(max_supported_schema(0x0008, d).is_none());
        }

        let index = RecordContent {
            rtype: 0x0008,
            schema: 0x0900,
            pieces: vec![Piece::Run(vec![0x00, 0x00])],
        };
        assert!(
            !read_strings(
                &super::super::tables::QE_INDEX,
                &index,
                StringFormat::Enhanced
            )
            .schema_too_new
        );
        assert!(
            read_strings(&super::super::tables::FONT, &index, StringFormat::Enhanced)
                .schema_too_new
        );
    }

    /// Every width names its own kind. A swapped arm would read a widened field at another width or
    /// another byte order, and nothing else in the table would be able to tell.
    #[test]
    fn every_width_names_its_own_kind() {
        use std::mem::discriminant as d;
        for (w, k) in [
            (Width::U8, Kind::U8),
            (Width::U16Be, Kind::U16Be),
            (Width::U32Be, Kind::U32Be),
            (Width::I8, Kind::I8),
            (Width::I16Be, Kind::I16Be),
            (Width::I32Be, Kind::I32Be),
            (Width::F32Le, Kind::F32Le),
            (Width::F32Be, Kind::F32Be),
            (Width::F64Le, Kind::F64Le),
            (Width::F64Be, Kind::F64Be),
        ] {
            assert_eq!(d(&w.kind()), d(&k), "{w:?}");
        }
    }

    /// A schema-widened field resolves to the half its record's version selects, and every other
    /// kind resolves to itself whatever the version.
    #[test]
    fn a_kind_resolves_to_the_half_its_version_selects() {
        use std::mem::discriminant as d;
        let widened = Kind::WidensAt {
            at: 0x0702,
            narrow: Width::U16Be,
            wide: Width::U32Be,
        };
        assert_eq!(d(&widened.resolve(0x0701)), d(&Kind::U16Be));
        assert_eq!(d(&widened.resolve(0x0702)), d(&Kind::U32Be));
        assert_eq!(d(&Kind::VarU32.resolve(0x0701)), d(&Kind::VarU32));
        assert_eq!(d(&Kind::VarU32.resolve(0x0702)), d(&Kind::VarU32));
    }

    /// The schema selects a field's *width*, not merely its presence — the shape the engine's
    /// saved-data records use, where the writer picks `0x0701` with `u16` counts or `0x0702` with
    /// `u32` ones according to the values it is storing.
    #[test]
    fn a_field_width_can_follow_the_records_schema() {
        fn narrow(c: &Ctx<'_>) -> bool {
            c.schema < 0x0702
        }
        fn wide(c: &Ctx<'_>) -> bool {
            !narrow(c)
        }
        const WIDENED: Table = Table {
            dialect: Dialect::Contents,
            rtype: 0x7ff0,
            name: "Widened",
            fields: &[
                Field::when("count", Kind::U16Be, narrow),
                Field::when("count", Kind::U32Be, wide),
                Field::new("tail", Kind::U8),
            ],
        };
        let at = |schema, bytes: &[u8]| RecordContent {
            rtype: 0x7ff0,
            schema,
            pieces: vec![Piece::Run(bytes.to_vec())],
        };

        let r = read_strings(
            &WIDENED,
            &at(0x0701, &[0x00, 0x05, 0xaa]),
            StringFormat::Enhanced,
        );
        assert!(r.exact() && r.complete);
        assert_eq!((r.row.u("count"), r.row.u("tail")), (5, 0xaa));

        let r = read_strings(
            &WIDENED,
            &at(0x0702, &[0x00, 0x00, 0x00, 0x05, 0xaa]),
            StringFormat::Enhanced,
        );
        assert!(r.exact() && r.complete);
        assert_eq!((r.row.u("count"), r.row.u("tail")), (5, 0xaa));

        // Reading the wide form under the narrow rule would leave bytes over — the failure the
        // schema check exists to prevent.
        assert!(!read_strings(
            &WIDENED,
            &at(0x0701, &[0x00, 0x00, 0x00, 0x05, 0xaa]),
            StringFormat::Enhanced
        )
        .exact());
    }

    /// A `Str` field says "a string" and nothing about its framing: the same table reads either
    /// wire form, and reads it only when the record declared it. Under the other form the table
    /// stops accounting for the record.
    #[test]
    fn a_string_field_reads_the_form_the_record_declared() {
        const NAMED: &[Field] = &[Field::new("name", Kind::Str), Field::new("tail", Kind::U8)];
        let t = one(NAMED);
        for (form, bytes) in [
            (StringFormat::Enhanced, &b"\x00\x00\x00\x03hi\0\xaa"[..]),
            (StringFormat::Simple, &b"hi\0\xaa"[..]),
        ] {
            let c = content(bytes);
            let r = read_strings(&t, &c, form);
            assert!(r.exact() && r.complete, "{form:?}");
            assert_eq!(r.row.text("name"), "hi", "{form:?}");
            assert_eq!(r.row.u("tail"), 0xaa, "{form:?}");
            assert_eq!(write_as(&t, &r.row, 0x0700, form), c.pieces, "{form:?}");

            // The other form over the same bytes: the framing is wrong, and the table says so.
            let other = match form {
                StringFormat::Enhanced => StringFormat::Simple,
                StringFormat::Simple => StringFormat::Enhanced,
            };
            let wrong = read_strings(&t, &c, other);
            assert!(
                !wrong.exact() || wrong.row.text("name") != "hi",
                "{form:?} read as {other:?} must not agree"
            );
        }
    }

    /// A field reference is one field, and its unset form is a value rather than a hole: the index
    /// `0xffff` reads as "no field" while any other reads as that index, and both re-emit the bytes
    /// they came from.
    #[test]
    fn a_field_reference_distinguishes_unset_from_an_index() {
        const REF: &[Field] = &[
            Field::new("field", Kind::FieldRef),
            Field::new("tail", Kind::U8),
        ];
        let t = one(REF);

        // The empty reference: an empty name, pool 0, index 0xffff.
        let c = content(b"\x00\x00\x00\x01\x00\x00\xff\xff\xaa");
        let r = read_strings(&t, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!(r.row.text("field"), "");
        assert_eq!(r.row.get("field").and_then(Cell::handle), Some((0, None)));
        assert_eq!(r.row.u("tail"), 0xaa);
        assert_eq!(
            write_as(&t, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );

        // A bound one: a name, pool 1, index 3 — and everything after it has moved.
        let c = content(b"\x00\x00\x00\x04sum\x00\x01\x00\x03\xaa");
        let r = read_strings(&t, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!(r.row.text("field"), "sum");
        assert_eq!(
            r.row.get("field").and_then(Cell::handle),
            Some((1, Some(3)))
        );
        assert_eq!(r.row.u("tail"), 0xaa);
        assert_eq!(
            write_as(&t, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );

        // Read as a fixed run of the width the empty form happens to have, a bound reference takes
        // the byte after it with it — the failure the composite exists to prevent.
        const FIXED: &[Field] = &[
            Field::new("field", Kind::Skip(8)),
            Field::new("tail", Kind::U8),
        ];
        assert!(!read_strings(&one(FIXED), &c, StringFormat::Enhanced).exact());
    }

    /// A trailing field the record need not carry: present, the table reads it; absent, the table
    /// still describes the record completely. Declared as unconditional instead, the short record
    /// reads as truncated.
    #[test]
    fn an_optional_field_is_absent_without_shortening_the_record() {
        const OPT: &[Field] = &[
            Field::new("kind", Kind::U8),
            Field::optional("extra", Kind::U16Be),
        ];
        let t = one(OPT);

        let c = content(&[0x07, 0x00, 0x05]);
        let r = read_strings(&t, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!((r.row.u("kind"), r.row.u("extra")), (7, 5));
        assert_eq!(
            write_as(&t, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );

        let c = content(&[0x07]);
        let r = read_strings(&t, &c, StringFormat::Enhanced);
        assert!(r.exact(), "no bytes are left over");
        assert!(
            r.complete,
            "the table reached every field the record carries"
        );
        assert!(r.row.get("extra").is_none());
        assert_eq!(
            write_as(&t, &r.row, 0x0700, StringFormat::Enhanced),
            c.pieces
        );

        // The same two bytes read as unconditional: the record is reported as ending early.
        const REQUIRED: &[Field] = &[
            Field::new("kind", Kind::U8),
            Field::new("extra", Kind::U16Be),
        ];
        let r = read_strings(&one(REQUIRED), &content(&[0x07]), StringFormat::Enhanced);
        assert!(!r.complete && r.stop.ended);
    }

    /// A field's presence can follow the record's schema: one version carries it and an older one
    /// does not, and a version may replace a run of fields rather than extend it.
    #[test]
    fn a_schema_gate_decides_whether_a_field_is_there() {
        const GATED: &[Field] = &[
            Field::new("data_type", Kind::U16Be),
            Field::only_at_schema("options", Kind::U16Be, 0x0900),
            Field::from_schema("attributes", Kind::U32Be, 0x0901),
            Field::from_schema("precision", Kind::U8, 0x0902),
        ];
        const T: Table = Table {
            dialect: Dialect::Contents,
            rtype: 0x7ff2,
            name: "Gated",
            fields: GATED,
        };
        let at = |schema, bytes: &[u8]| RecordContent {
            rtype: 0x7ff2,
            schema,
            pieces: vec![Piece::Run(bytes.to_vec())],
        };

        // The oldest version: the alternative field, and neither addition.
        let c = at(0x0900, &[0x00, 0x03, 0x00, 0x09]);
        let r = read_strings(&T, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!(r.row.u("options"), 9);
        assert!(r.row.get("attributes").is_none());
        assert_eq!(
            write_as(&T, &r.row, 0x0900, StringFormat::Enhanced),
            c.pieces
        );

        // One version on: the alternative is gone and the first addition is there instead.
        let c = at(0x0901, &[0x00, 0x03, 0x00, 0x00, 0x00, 0x11]);
        let r = read_strings(&T, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert!(r.row.get("options").is_none());
        assert_eq!(r.row.u("attributes"), 0x11);
        assert!(r.row.get("precision").is_none());
        assert_eq!(
            write_as(&T, &r.row, 0x0901, StringFormat::Enhanced),
            c.pieces
        );

        // The newest: every addition, and still no alternative.
        let c = at(0x0902, &[0x00, 0x03, 0x00, 0x00, 0x00, 0x11, 0x04]);
        let r = read_strings(&T, &c, StringFormat::Enhanced);
        assert!(r.exact() && r.complete);
        assert_eq!((r.row.u("attributes"), r.row.u("precision")), (0x11, 4));
        assert_eq!(
            write_as(&T, &r.row, 0x0902, StringFormat::Enhanced),
            c.pieces
        );

        // Ungated, the newest record's bytes are read as the oldest layout — the fields shift and
        // the record over-runs, which is what the gates exist to prevent.
        const UNGATED: &[Field] = &[
            Field::new("data_type", Kind::U16Be),
            Field::new("options", Kind::U16Be),
            Field::new("attributes", Kind::U32Be),
        ];
        assert!(!read_strings(
            &one(UNGATED),
            &at(0x0902, &[0x00, 0x03, 0x00, 0x00, 0x00, 0x11, 0x04]),
            StringFormat::Enhanced
        )
        .exact());
    }

    /// A field the record was too short to reach reads as its default — the absence every trailing
    /// field relies on, and the one case a name lookup is allowed to be quiet about.
    #[test]
    fn a_declared_field_the_record_never_reached_reads_as_its_default() {
        let r = read_strings(&SYNTHETIC, &content(&[0x00, 0x00]), StringFormat::Enhanced);
        assert!(r.row.get("angle").is_none());
        assert_eq!(r.row.u("angle"), 0);
        assert_eq!(r.row.text("angle"), "");
    }

    /// A name the table does not declare is a mistake in the caller rather than an absent value:
    /// no record of the type can produce it, so it fails at the lookup instead of reading as a
    /// field that decoded empty.
    #[test]
    #[should_panic(expected = "no field named `angel` is declared here")]
    fn a_lookup_by_an_undeclared_name_fails_at_the_lookup() {
        let r = read_strings(&SYNTHETIC, &content(&[0x00, 0x00]), StringFormat::Enhanced);
        let _ = r.row.text("angel");
    }

    /// The same inside a `repeat`: a body row is checked against the body's own fields, so a name
    /// borrowed from the enclosing record does not quietly read as empty there.
    #[test]
    #[should_panic(expected = "no field named `count` is declared here")]
    fn a_repeat_body_is_checked_against_the_bodys_fields() {
        let mut b = vec![0x00, 0x01];
        b.extend_from_slice(&[0x01, 0x90, 0x01, 0, 0]);
        b.extend_from_slice(&F64_1_5);
        let r = read_strings(&SYNTHETIC, &content(&b), StringFormat::Enhanced);
        assert_eq!(r.row.seq("elements")[0].u("weight"), 400);
        let _ = r.row.seq("elements")[0].u("count");
    }

    /// A row assembled by hand declares nothing, so it is read back exactly as it was written.
    #[test]
    fn a_hand_built_row_admits_the_names_it_was_given() {
        let mut row = Row::default();
        row.push("kind", Kind::VarU16, Cell::U(3), Span::NONE);
        assert_eq!(row.u("kind"), 3);
        assert_eq!(row.u("anything_else"), 0);
    }

    /// A wrong skip length is loud: the table finishes with bytes left over.
    #[test]
    fn an_unaccounted_byte_is_a_failure_not_a_tolerance() {
        const SHORT: Table = Table {
            dialect: Dialect::Contents,
            rtype: 0x7ffd,
            name: "Short",
            fields: &[Field::new("_a", Kind::Skip(3))],
        };
        let c = content(&[1, 2, 3, 4]);
        let r = read_strings(&SHORT, &c, StringFormat::Enhanced);
        assert!(!r.exact());
        assert_eq!(r.unread, 1);
    }
}
