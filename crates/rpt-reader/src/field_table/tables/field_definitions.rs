//! Each kind of field is a wrapper around the definition record below it, with its own few fields
//! after: a database field wraps a `0x0072`, which wraps the `0x0071` base; a formula field wraps its
//! `0x0076` body; a summary wraps its `0x007e`. The nested record always comes **first**, so a
//! wrapper's own bytes start where its child ends.

use super::*;

/// `0x0029 RecordSortField` — one sort entry: the sorted field's reference, a `(kind, index)` handle
/// into the report's own field pools, and the sort direction.
///
/// The handle's kind is what tells a plain field sort from a group's summary sort: `0` a database
/// field, `1` a formula, `2` a summary — whose reference is the summary's display form and whose
/// index is not a pool entry. For a database or formula sort the index names the entry: in a report
/// sorted on three customer columns the three records carry that report's pool indices 1, 2 and 3.
pub(crate) const RECORD_SORT_FIELD: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0029,
    name: "RecordSortField",
    fields: &[
        Field::new("field", Kind::Str),
        Field::new("field_kind", Kind::VarU16),
        Field::new("field_index", Kind::U16Be),
        Field::new("direction", Kind::VarU16),
    ],
};

/// `0x006e FieldManagerEntry` — the field-pool census the engine writes once per report.
///
/// A leading word, then **nine** pool sizes, each of which the engine hands straight to one field
/// array before reading its records; the ninth is read only if the record has bytes left. The first
/// two pools are the database fields and the formula bodies (the latter short of the three built-in
/// formulas). The leading word is not part of the first count — it is zero, which is the only reason
/// reading the two together as one `u32` ever gave the right number.
pub(crate) const FIELD_MANAGER_ENTRY: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x006e,
    name: "FieldManagerEntry",
    fields: &[
        Field::new("_u0", Kind::I16Be),
        Field::new("database_fields", Kind::U16Be),
        Field::new("formula_bodies", Kind::U16Be),
        Field::new("_u1", Kind::U16Be),
        Field::new("_u2", Kind::U16Be),
        Field::new("_u3", Kind::U16Be),
        Field::new("_u4", Kind::U16Be),
        Field::new("_u5", Kind::U16Be),
        Field::new("_u6", Kind::U16Be),
        Field::new("_u7", Kind::U16Be),
    ],
};

/// `0x010c GuidelineEntry` — one design-surface snap guideline: where it sits, and how many
/// objects are attached to it.
///
/// It is the head of the guideline it belongs to rather than a member of a list: exactly one is
/// written inside each `0x010d` guideline list and each `0x010f` guideline collection, which are
/// the horizontal and vertical guides of a section's ruler. The position is a twip coordinate on
/// the canvas, stored at a fixed four bytes rather than in the narrowing form most twips take, and
/// it is signed.
///
/// The word after it is a count and not a flag word: it states how many `0x0112` object-connection
/// collections follow this record inside the same guideline, each holding the one `0x0111` edge
/// that binds a report object to this guide. A guide nothing is snapped to writes zero.
pub(crate) const GUIDELINE_ENTRY: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x010c,
    name: "GuidelineEntry",
    fields: &[
        Field::new("position", Kind::I32Be),
        Field::new("connection_count", Kind::U16Be),
    ],
};

/// `0x0118 FormulaVariable` — one persisted `Global`/`Shared` formula-language variable: its name,
/// its declared result kind, and its scope.
pub(crate) const FORMULA_VARIABLE: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0118,
    name: "FormulaVariable",
    fields: &[
        Field::new("name", Kind::Str),
        Field::new("value_type", Kind::VarU16),
        Field::new("scope", Kind::VarU16),
    ],
};

/// `0x0071 NamedValue` — the field-definition base every kind of field is built on: its name, the
/// type of the value it produces, and how long that value is.
///
/// One shape, not two. A definition with no name of its own — a summary — stores an *empty* string
/// rather than omitting the field, so its type byte is simply the first byte after a five-byte
/// empty string block instead of the first byte after a name.
///
/// The length is stored twice. The narrow form counts **characters** for a string-typed value and
/// bytes for everything else, and it saturates at 255; the wide form that follows the second name
/// is the byte count outright and supersedes it. Only the wide form is a byte count for every type,
/// so it is the one a consumer wants. It is signed: a value whose width is not fixed — a blob —
/// stores `-1`.
pub(crate) const NAMED_VALUE: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0071,
    name: "NamedValue",
    fields: &[
        Field::new("name", Kind::Str),
        Field::new("value_type", Kind::VarU16),
        Field::new("narrow_length", Kind::U16Be),
        Field::new("_u0", Kind::Str),
        Field::optional("length", Kind::I32Be),
    ],
};

/// `0x0072 NamedValueWrapper` — a `0x0071` and nothing else.
pub(crate) const NAMED_VALUE_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0072,
    name: "NamedValueWrapper",
    fields: &[Field::new("value", Kind::Child(0x0071))],
};

/// `0x0073 FieldDef` — a database field: the wrapped definition, then the handle that resolves the
/// column it reads.
pub(crate) const FIELD_DEFINITION: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0073,
    name: "FieldDef",
    fields: &[
        Field::new("value", Kind::Child(0x0072)),
        Field::new("field_id", Kind::U32Be),
        Field::new("_u0", Kind::Skip(6)),
    ],
};

/// One field a formula body refers to. The reference resolves to the pool the field lives in and
/// its index there, so a formula's dependencies survive a rename of the field it names.
const FORMULA_REFERENCE: &[Field] = &[Field::new("field", Kind::FieldRef)];

/// `0x0076 Formula` — one stored formula: the definition that names it, the fields its body refers
/// to, the body text, and what the formula is for.
///
/// The `0x0071` definition is the record's **first** content, not a sibling that follows it: it
/// occupies the head of the content span and the body's own bytes start where it ends. It carries
/// the formula's name, the value type its result has, and that result's stored width.
///
/// The dependency list precedes the text: a count, then one reference per field the body names.
/// Each reference is a name, the pool it resolves in, and its index there — so a wide reference
/// moves the body text, which no fixed offset can follow.
///
/// The text is followed by what the formula is for — which kind of formula it is, and which format
/// property it conditions — then seven words. The three fields closing the record are each written
/// only while the record still has content, which is what lets a writer end it before them: the
/// authoring dialect, `1` for the Basic-syntax editor and `0` for Crystal's own; how the body
/// treats a null field value, `1` for substituting the value type's default and `0` for raising;
/// and one more word.
pub(crate) const FORMULA: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0076,
    name: "Formula",
    fields: &[
        Field::new("value", Kind::Child(0x0071)),
        Field::new("reference_count", Kind::U16Be),
        Field::new(
            "references",
            Kind::Repeat {
                count: Count::FromField("reference_count"),
                body: FORMULA_REFERENCE,
            },
        ),
        Field::new("text", Kind::Str),
        Field::new("formula_kind", Kind::VarU16),
        Field::new("format_formula_kind", Kind::VarU16),
        Field::new("_u0", Kind::I16Be),
        Field::new("_u1", Kind::I16Be),
        Field::new("_u2", Kind::I16Be),
        Field::new("_u3", Kind::I16Be),
        Field::new("_u4", Kind::I16Be),
        Field::new("_u5", Kind::I16Be),
        Field::new("_u6", Kind::I16Be),
        Field::optional("syntax", Kind::VarU16),
        Field::optional("null_treatment", Kind::VarU16),
        Field::optional("_u7", Kind::I16Be),
    ],
};

/// `0x0077 FormulaFieldWrapper` — a formula field: its body, then two words the engine loads into a
/// runtime cache slot. Both are written only while the record still has content.
pub(crate) const FORMULA_FIELD_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0077,
    name: "FormulaFieldWrapper",
    fields: &[
        Field::new("formula", Kind::Child(0x0076)),
        Field::optional("_u0", Kind::I16Be),
        Field::optional("_u1", Kind::I16Be),
    ],
};

/// `0x0078 ReportProperty` — a special-variable field (page number, print date, record number …):
/// the wrapped definition, then which variable it is.
pub(crate) const REPORT_PROPERTY: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0078,
    name: "ReportProperty",
    fields: &[
        Field::new("value", Kind::Child(0x0071)),
        Field::new("special_var_type", Kind::VarU16),
    ],
};

/// `0x0079 FieldDefinition2` — a group-name field: the wrapped definition, which kind of group it
/// names, and a string the writer leaves empty.
pub(crate) const FIELD_DEFINITION2: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0079,
    name: "FieldDefinition2",
    fields: &[
        Field::new("value", Kind::Child(0x0071)),
        Field::new("group_type", Kind::VarU16),
        Field::new("_u0", Kind::U16Be),
        Field::new("_u1", Kind::Str),
    ],
};

/// The value type an entry of [`value_entry`] is written under, read off the record the repeat
/// belongs to. These are the codes of
/// [`FieldValueType::from_code`](crate::model::FieldValueType::from_code).
fn entry_value_type(c: &Ctx<'_>) -> u32 {
    c.outer.map_or(0, |record| record.u("value_type"))
}

/// Whether an entry's payload is one of `types`. A byte count of zero is the null value and carries
/// no payload at all, whatever the type.
fn entry_holds(c: &Ctx<'_>, types: &[u32]) -> bool {
    c.row.num("byte_count") != 0 && types.contains(&entry_value_type(c))
}

/// One stored value: its byte count, then a payload whose shape the **record's** value type
/// decides.
///
/// The count is a prefix, not the payload's framing — the payload is read at the width its type
/// has, and the count states what that width came to. A count of zero is a null value and is the
/// whole entry. A type with no stored form (memo, blob, the wide numerics) carries the count and
/// nothing after it, which is why every payload below is conditional rather than a default arm.
///
/// Several types share a payload: Boolean is a signed word like Int16s, Date and Time are a day
/// number and a second count and are signed longs like Int32s, Currency is a double like Number,
/// and DateTime is a day number and a second count side by side. A string's count is the character
/// count plus its terminator, and the string still carries its own length prefix — so the count
/// does not span the field.
///
/// The count's width follows a version, and the version it widens at belongs to the **record that
/// carries the list**, not to the entry: a parameter's stored list widens one version earlier than a
/// saved current value's. That is the one thing `at` states, and it is why the same entry is
/// declared twice rather than once.
pub(super) const fn value_entry(at: u16) -> [Field; 11] {
    [
        Field::new(
            "byte_count",
            Kind::WidensAt {
                at,
                narrow: Width::U16Be,
                wide: Width::U32Be,
            },
        ),
        Field::when("int8", Kind::I8, |c| entry_holds(c, &[0])),
        Field::when("uint8", Kind::U8, |c| entry_holds(c, &[1])),
        Field::when("int16", Kind::I16Be, |c| entry_holds(c, &[2, 8])),
        Field::when("uint16", Kind::U16Be, |c| entry_holds(c, &[3])),
        Field::when("int32", Kind::I32Be, |c| entry_holds(c, &[4, 9, 10])),
        Field::when("uint32", Kind::U32Be, |c| entry_holds(c, &[5])),
        Field::when("number", Kind::F64Be, |c| entry_holds(c, &[6, 7])),
        Field::when("date_time_days", Kind::I32Be, |c| entry_holds(c, &[15])),
        Field::when("date_time_seconds", Kind::I32Be, |c| entry_holds(c, &[15])),
        Field::when("text", Kind::Str, |c| entry_holds(c, &[11])),
    ]
}

/// The value entry as a parameter's stored list frames it: the byte count widens at `0x0701`.
const PARAMETER_VALUE_ENTRY: [Field; 11] = value_entry(0x0701);

/// One description of a parameter's stored list, one per value. It is read only while the record
/// still has content, so a record that stops after its values keeps the list it stated rather than
/// running past its own end.
const PARAMETER_VALUE_DESCRIPTION: &[Field] = &[Field::optional("text", Kind::Str)];

/// `0x007a ParameterRecord` — one parameter field: the definition it is built on, how it prompts,
/// what it will accept, and the values offered for it.
///
/// The `0x0071` definition is the record's **first** content — the parameter's name, the type of
/// value it produces, and that value's width — so the record's own bytes start where it ends. The
/// index that follows is the report-wide parameter number, and it is the key a saved current value
/// (`0x0031`) names its parameter by.
///
/// Then the shape of the value: the type once, a count, and that many entries under it. The field
/// reference ahead of the type is the field the parameter is bound to, and `field_name` near the end
/// of the record names the same one; a parameter bound to nothing writes the reference's empty form.
///
/// The bounds come next and are stated per value family rather than per parameter: a flag, two
/// doubles for a numeric one, then — while the record still has content — a long each for a date and
/// a time bound and two for a date-and-time one. An unset bound is not absent: a numeric one is
/// `±f32::MAX`, an empty interval written the widest way round, and the rest are `-1`.
///
/// Everything from `parameter_type` on is written only while the record has content left, which is
/// what lets a writer stop early. It is the record's whole tail: the flags governing what the prompt
/// accepts, the parameter's own name again, how the default list is shown and sorted, one
/// description per stored value, the identity that joins the record to its prompting entry, and the
/// flag that makes the prompt optional.
pub(crate) const PARAMETER_RECORD: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x007a,
    name: "ParameterRecord",
    fields: &[
        Field::new("value", Kind::Child(0x0071)),
        Field::new("parameter_index", Kind::U16Be),
        Field::new("prompt_text", Kind::Str),
        Field::new("_u0", Kind::I16Be),
        Field::new("field", Kind::FieldRef),
        Field::new("value_type", Kind::VarU16),
        Field::new("value_count", Kind::U16Be),
        Field::new(
            "values",
            Kind::Repeat {
                count: Count::FromField("value_count"),
                body: &PARAMETER_VALUE_ENTRY,
            },
        ),
        Field::new("has_bounds", Kind::I16Be),
        Field::new("number_min", Kind::F64Be),
        Field::new("number_max", Kind::F64Be),
        Field::new("edit_mask", Kind::Str),
        Field::optional("date_min", Kind::I32Be),
        Field::optional("date_max", Kind::I32Be),
        Field::optional("time_min", Kind::I32Be),
        Field::optional("time_max", Kind::I32Be),
        Field::optional("date_time_min_days", Kind::I32Be),
        Field::optional("date_time_min_seconds", Kind::I32Be),
        Field::optional("date_time_max_days", Kind::I32Be),
        Field::optional("date_time_max_seconds", Kind::I32Be),
        Field::optional("parameter_type", Kind::VarU16),
        Field::optional("_u1", Kind::Bool),
        Field::optional("dynamic_values", Kind::Bool),
        Field::optional("allow_multiple_values", Kind::Bool),
        Field::optional("_u2", Kind::Bool),
        Field::optional("_u3", Kind::Bool),
        Field::optional("_u4", Kind::Bool),
        Field::optional("_u5", Kind::U16Be),
        Field::optional("_u6", Kind::Bool),
        Field::optional("name", Kind::Str),
        Field::optional("_u7", Kind::Bool),
        Field::optional("_u8", Kind::VarU16),
        Field::optional("default_value_display_type", Kind::Bool),
        Field::optional("default_value_sort_order", Kind::VarU16),
        Field::optional("_u9", Kind::Bool),
        Field::new(
            "descriptions",
            Kind::Repeat {
                count: Count::FromField("value_count"),
                body: PARAMETER_VALUE_DESCRIPTION,
            },
        ),
        Field::optional("_u10", Kind::VarU16),
        Field::optional("field_name", Kind::Str),
        Field::optional("id", Kind::Str),
        Field::optional("_u11", Kind::I16Be),
        Field::optional("optional_prompt", Kind::Bool),
    ],
};

/// `0x007f SummaryFieldWrapper` — a summary field: its definition, then which kind of summary owner
/// it belongs to.
pub(crate) const SUMMARY_FIELD_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x007f,
    name: "SummaryFieldWrapper",
    fields: &[
        Field::new("summary", Kind::Child(0x007e)),
        Field::new("summary_kind", Kind::I16Be),
        Field::new("_u0", Kind::U16Be),
    ],
};

/// `0x009c SectionCodeHeaderFooter` — the wrapper around the area-type record, then one word.
pub(crate) const SECTION_CODE_HEADER_FOOTER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x009c,
    name: "SectionCodeHeaderFooter",
    fields: &[
        Field::new("area_type", Kind::Child(0x009b)),
        Field::new("_u0", Kind::I16Be),
    ],
};

/// `0x0165 ObjectMarker` — a report-part reference: three strings, written only when the record
/// carries any content at all. Ordinarily empty, so the marker is just a header and nothing else.
pub(crate) const OBJECT_MARKER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0165,
    name: "ObjectMarker",
    fields: &[
        Field::optional("_u0", Kind::Str),
        Field::optional("_u1", Kind::Str),
        Field::optional("_u2", Kind::Str),
    ],
};

/// `0x0151 XmlDefinition` — the report's XML-export mapping, written after most records as an
/// **empty** placeholder.
///
/// The record's whole field sequence is behind one gate: a record that carries no content at all is
/// the definition being absent, and nothing after the header is read. An empty record — none
/// carrying a byte or a child — is the type's ordinary form, which is why it is common yet worth
/// nothing to decode.
///
/// A populated one names the definition, then states how many XSLT definitions (`0x0153`, each a
/// name, a kind, two more strings and a trailing enum) follow it, and closes with a `0x0152`
/// terminator. The fields below are the sequence's opening; the count's own trailing run is skipped
/// wholesale by the format, so a populated record would leave bytes unaccounted for here rather
/// than being read at a guess.
pub(crate) const XML_DEFINITION: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0151,
    name: "XmlDefinition",
    fields: &[
        Field::optional("name", Kind::Str),
        Field::optional("_u0", Kind::VarU16),
        Field::optional("_u1", Kind::I16Be),
        Field::optional("xslt_count", Kind::U16Be),
    ],
};

/// `0x0178 SaveMetadata` — one save-time environment fact, as a key and its value.
pub(crate) const SAVE_METADATA: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0178,
    name: "SaveMetadata",
    fields: &[Field::new("key", Kind::Str), Field::new("value", Kind::Str)],
};
