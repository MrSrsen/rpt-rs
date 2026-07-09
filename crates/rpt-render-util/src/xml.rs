//! XML/HTML escaping shared by the markup backends (SVG, HTML).

/// Escape text for XML/HTML content (`&`, `<`, `>`) — the escaping the SVG and HTML backends share.
/// (The HTML backend additionally turns spaces into `&nbsp;` for its no-wrap runs; that quirk stays
/// in the backend.)
pub fn escape_xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape a string for an XML/HTML attribute value (text escaping plus `"`), shared by the SVG and
/// HTML backends.
pub fn escape_xml_attr(s: &str) -> String {
    escape_xml_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escaping_matches_backend_expectations() {
        assert_eq!(escape_xml_text("a & b <c>"), "a &amp; b &lt;c&gt;");
        assert_eq!(escape_xml_attr("x\"&"), "x&quot;&amp;");
    }
}
