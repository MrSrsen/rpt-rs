//! Export the rpt-rs semantic report model ([`rpt_model::Report`]) to a [KDL](https://kdl.dev)
//! document — the human-readable, sparsely-defaulted authoring surface for a report.
//!
//! This is the semantic half of the report-as-source design: a hand-written node mapping (not a
//! generic serde derive) whose readability *is* the format — construct kinds as node names, the
//! identifying name as the first argument, scalars as `key=value` properties, nested structs and
//! `Vec`s as child nodes in model order, and only non-default values emitted. Geometry stays in raw
//! twips, colours render as `#rrggbb`, enums as kebab-case tokens (an unmapped engine code falls
//! back to its raw integer), and formula bodies render as KDL multi-line strings.
//!
//! Binary payloads never enter the KDL: an embedded picture emits a `source="…"` reference to a
//! sidecar file, and [`assets`] returns the matching bytes in memory for a caller to write out (the
//! crate itself does no I/O). OLE embeds, which the model keeps only as a digest, are listed under an
//! `embeds` node by reference.
//!
//! The crate depends only on [`rpt_model`] and [`kdl`]: no decoder, no I/O, WASM-safe. The lossless
//! carrier layer (undecoded residue/raw/stream nodes) is a later addition tracked separately.
//!
//! ```no_run
//! # fn demo(report: &rpt_model::Report) {
//! let kdl = rpt_kdl::to_kdl_string(report);
//! println!("{kdl}");
//! # }
//! ```

#![forbid(unsafe_code)]

mod assets;
mod build;
mod document;
mod enums;
mod format;
mod objects;

pub use assets::{assets, Asset};

use kdl::{KdlDocument, KdlEntryFormat, KdlValue};
use rpt_model::Report;

/// Build a [`KdlDocument`] for `report`: a single top-level `report` node with its full child tree.
///
/// The document is autoformatted (indented, one node per line) and its multi-line string values —
/// formula bodies, text content — are rendered as KDL triple-quoted blocks. [`to_kdl_string`] is
/// this document's textual form.
pub fn to_document(report: &Report) -> KdlDocument {
    let mut doc = KdlDocument::new();
    doc.nodes_mut().push(document::report_node(report).build());
    doc.autoformat();
    patch_multiline(&mut doc, 0);
    doc
}

/// Export `report` to formatted KDL document text.
pub fn to_kdl_string(report: &Report) -> String {
    to_document(report).to_string()
}

/// Rewrite every multi-line string value in the (already autoformatted) document as a KDL
/// triple-quoted block indented one level below its owning node, so formula bodies and text content
/// read as literal blocks rather than `\n`-escaped one-liners.
///
/// `depth` is the nesting depth of `doc`'s nodes (0 for the top level); content is indented at
/// `depth + 1` levels of four spaces, matching the autoformatter. Values containing a backslash or an
/// existing `"""` are left in the escaped one-line form the autoformatter produced, so the output is
/// always valid KDL that round-trips to the same string.
fn patch_multiline(doc: &mut KdlDocument, depth: usize) {
    for node in doc.nodes_mut() {
        for entry in node.entries_mut() {
            let value = match entry.value() {
                KdlValue::String(s) => s.clone(),
                _ => continue,
            };
            if value.contains('\n') && !value.contains('\\') && !value.contains("\"\"\"") {
                // The autoformatter cleared each entry's format, so the node stringifier now inserts
                // the inter-entry space itself. Setting a format opts this entry out of that, so the
                // leading space must be restored here alongside the multi-line value representation.
                entry.set_format(KdlEntryFormat {
                    value_repr: multiline_repr(&value, (depth + 1) * 4),
                    leading: " ".into(),
                    ..Default::default()
                });
            }
        }
        if let Some(children) = node.children_mut() {
            patch_multiline(children, depth + 1);
        }
    }
}

/// The KDL triple-quoted representation of `content`, with each line (and the closing `"""`) indented
/// by `indent` spaces — the whitespace prefix KDL strips back off on parse, so the round-tripped value
/// equals `content`.
fn multiline_repr(content: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::from("\"\"\"\n");
    for line in content.split('\n') {
        out.push_str(&pad);
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&pad);
    out.push_str("\"\"\"");
    out
}
