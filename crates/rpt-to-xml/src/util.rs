//! XML serialisation helpers shared across the writer modules.

pub(crate) fn b(v: bool) -> &'static str {
    if v {
        "True"
    } else {
        "False"
    }
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
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
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
