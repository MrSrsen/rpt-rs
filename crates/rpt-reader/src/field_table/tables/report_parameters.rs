//! The `ReportParametersStream` — a fourth vocabulary, framed like the report definition's and
//! numbered like nothing else.

use super::field_definitions::value_entry;
use super::*;

/// The strings that accompany one stored value. Only the first is a description; the rest are read
/// and dropped. The count is read while the record still has content, so a record that carries no
/// strings at all ends at the value it last stated rather than short of a list it never wrote.
const CURRENT_VALUE_STRINGS: &[Field] = &[
    Field::optional("count", Kind::U16Be),
    Field::new(
        "strings",
        Kind::Repeat {
            count: Count::FromField("count"),
            body: &[Field::new("text", Kind::Str)],
        },
    ),
];

/// The value entry as a saved current value frames it: the byte count widens at `0x0702`.
const CURRENT_VALUE_ENTRY: [Field; 11] = value_entry(0x0702);

/// `0x0031 CurrentValueRecord` — one parameter's saved current value.
///
/// The record names its parameter by the index the whole report shares, then states the value type
/// once and carries every value under it. Version `0x0700` carries a single value and no count at
/// all; every later version carries the count, the values, the marker bytes and the per-value
/// string lists, and widens each value's byte count from a word to a long at `0x0702`.
///
/// Three of the trailing fields are carried only while the record still has content — two words and
/// the flag that admits the prompting block. That block is a CRMetaObjects document stating the same
/// values in their prompting form, and for a range it is the only place the bound kinds are stated,
/// the values themselves being the two bounds.
pub(crate) const CURRENT_VALUE_RECORD: Table = Table {
    dialect: Dialect::ReportParameters,
    rtype: 0x0031,
    name: "CurrentValueRecord",
    fields: &[
        Field::new("parameter_index", Kind::U32Be),
        Field::new("value_type", Kind::VarU16),
        Field::only_at_schema(
            "value",
            Kind::Repeat {
                count: Count::Fixed(1),
                body: &CURRENT_VALUE_ENTRY,
            },
            0x0700,
        ),
        Field::from_schema("_u0", Kind::I16Be, 0x0701),
        Field::from_schema("value_count", Kind::U16Be, 0x0701),
        Field::from_schema(
            "values",
            Kind::Repeat {
                count: Count::FromField("value_count"),
                body: &CURRENT_VALUE_ENTRY,
            },
            0x0701,
        ),
        Field::from_schema("marker_count", Kind::U16Be, 0x0701),
        Field::from_schema(
            "markers",
            Kind::Repeat {
                count: Count::FromField("marker_count"),
                body: &[Field::new("_u0", Kind::U8)],
            },
            0x0701,
        ),
        Field::optional("_u1", Kind::I16Be),
        Field::optional("_u2", Kind::I16Be),
        Field::optional("has_prompting_info", Kind::I16Be),
        Field::when("prompting_info", Kind::Blob, |c| {
            c.row.i("has_prompting_info") != 0
        }),
        Field::from_schema(
            "value_strings",
            Kind::Repeat {
                count: Count::FromField("value_count"),
                body: CURRENT_VALUE_STRINGS,
            },
            0x0701,
        ),
    ],
};
