//! XML serialisation helpers shared across the writer modules.
//!
//! Many oracle-facing attribute strings are the `Debug` form (`{:?}`) of an `rpt` model enum — both
//! directly (see [`value_type_name`]/[`field_type_name`], and the `{:?}` sites across the writers,
//! e.g. `SortDirection`/`JoinType`/`PaperOrientation`) and via [`paper_size_str`]/[`paper_source_str`].
//! The exported text is therefore tied to the enum *variant names* in `rpt`: renaming a variant
//! silently changes the XML, and nothing but the byte-identity (sha256) parity gate catches it.

use std::fmt::Write as _;

pub(crate) fn b(v: bool) -> &'static str {
    if v {
        "True"
    } else {
        "False"
    }
}

/// Write a container element following the ubiquitous empty-vs-populated shape: an empty `items`
/// yields a self-closing `{indent}<{tag} />`; otherwise `{indent}<{tag}>`, each item written by
/// `each`, then `{indent}</{tag}>`. `each` emits an item's own (deeper-indented) lines. This is the
/// single home for the collection-emitting pattern that recurs across the writers.
pub(crate) fn write_collection<T>(
    o: &mut String,
    indent: &str,
    tag: &str,
    items: &[T],
    mut each: impl FnMut(&mut String, &T),
) {
    if items.is_empty() {
        let _ = writeln!(o, "{indent}<{tag} />");
        return;
    }
    let _ = writeln!(o, "{indent}<{tag}>");
    for item in items {
        each(o, item);
    }
    let _ = writeln!(o, "{indent}</{tag}>");
}

/// A field value type as the `@ValueType` attribute (the `CrFieldValueType` enum name, e.g.
/// `NumberField`): the enum variant name + the `Field` suffix.
pub(crate) fn value_type_name(vt: rpt::model::FieldValueType) -> String {
    format!("{vt:?}Field")
}

/// A field value type as the `@Type` attribute on `<Field>` (the fully-qualified constant, e.g.
/// `crFieldValueTypeStringField`).
pub(crate) fn field_type_name(vt: rpt::model::FieldValueType) -> String {
    format!("crFieldValueType{vt:?}Field")
}

/// A paper size as the `@PaperSize` attribute: the SDK name (`PaperLetter`), or the bare driver
/// code for sizes outside the modelled set.
pub(crate) fn paper_size_str(p: rpt::model::PaperSize) -> String {
    match p {
        rpt::model::PaperSize::Code(n) => n.to_string(),
        other => format!("{other:?}"),
    }
}

/// A paper source as the `@PaperSource` attribute: the SDK name (`FormSource`), or the bare code.
pub(crate) fn paper_source_str(p: rpt::model::PaperSource) -> String {
    match p {
        rpt::model::PaperSource::Code(n) => n.to_string(),
        other => format!("{other:?}"),
    }
}

/// XML-escape `s`, matching .NET `XmlWriter`. `&`, `<`, `>`, CR, LF and TAB are always entitized
/// (CR/LF/TAB as `&#xD;`/`&#xA;`/`&#x9;` so they survive XML normalization). Inside a double-quoted
/// attribute value `"` is entitized too; in element text it stays literal. `'` is never escaped (a
/// double-quoted attribute does not require it); other control characters are dropped. The single
/// escaping routine for the whole writer — see [`escape`] (attribute values) and [`escape_text`]
/// (element text).
fn escape_xml(s: &str, in_attribute: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            '\n' => out.push_str("&#xA;"),
            '"' if in_attribute => out.push_str("&quot;"),
            // Tab is significant in element text too (SQL Command bodies indent with tabs), so emit
            // it as a char reference in both contexts rather than dropping it as a control byte.
            '\t' => out.push_str("&#x9;"),
            // Drop anything outside the XML 1.0 `Char` production (C0 controls, U+FFFE/U+FFFF, …) so
            // the writer never emits a not-well-formed document, even from an unusual stored value.
            c if !is_xml_char(c) => {}
            c => out.push(c),
        }
    }
    out
}

/// Whether `c` is a legal XML 1.0 character (`\t`/`\n`/`\r` are handled before this is reached).
fn is_xml_char(c: char) -> bool {
    matches!(c as u32, 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// XML-escape an attribute value (formula text, names, …). See [`escape_xml`].
pub(crate) fn escape(s: &str) -> String {
    escape_xml(s, true)
}

/// XML-escape element text (multi-line formula / Command bodies). Element-text line endings are
/// normalized to LF: a stored `\r\n` formula body is emitted with `&#xA;` only, never `&#xD;` (CR
/// is preserved only in *attribute* values, via [`escape`]). See [`escape_xml`].
pub(crate) fn escape_text(s: &str) -> String {
    let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
    escape_xml(&normalized, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml_metacharacters() {
        assert_eq!(
            escape("a & b < c > \"d\""),
            "a &amp; b &lt; c &gt; &quot;d&quot;"
        );
    }

    #[test]
    fn element_text_normalizes_crlf_to_lf() {
        // Element text drops CR (only &#xA; in bodies); attributes keep it.
        assert_eq!(escape_text("a\r\nb\rc"), "a&#xA;b&#xA;c");
        assert_eq!(escape("a\r\nb"), "a&#xD;&#xA;b");
    }
}
