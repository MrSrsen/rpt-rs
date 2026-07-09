//! `tree` — a structural tree of the decoded record DOM, grouped by source stream.

use std::fmt::Write as _;

use rpt::model::{Node, Value};
use rpt::Rpt;
use serde::Serialize;

use crate::util::{paint, truncate, BOLD, BOLD_GREEN, BRIGHT_MAGENTA, CYAN, DIM, YELLOW};

pub(crate) const HELP: &str = "\
rpt tree — a structural tree of the decoded record DOM

Grouped by source stream. Each node is tagged by kind: CfbStream(<name>) is the first tier (the
main report's Contents, then every subreport's Subdocument N/Contents); Branch(<type>) is a node
with nested children; Leaf(<type>) is a node with none (field data only). <type> is the record's
registry name (or its raw 0xNNNN word), followed by a truncated preview of its content.

The tree highlights by prominence — recognized record types and field/text content stand out,
picture / OLE object records and large embedded data blobs are flagged in pink/purple, and
scaffolding (unknown types, small byte runs, connectors) is dimmed. Color is on by default at a
terminal and off when piped; --color / --no-color (and NO_COLOR / CLICOLOR_FORCE) override that.

USAGE:
    rpt tree <file.rpt> [--json] [--depth N] [--color | --no-color]

OPTIONS:
    --json         emit the node tree as JSON
    --depth N      limit the tree to N levels deep; deeper nodes are collapsed
    --color        force coloring on even when piped (e.g. `rpt tree f.rpt --color | less -R`)
    --no-color     force coloring off
";

/// Max characters of a leaf-value preview shown per node.
const PREVIEW_MAX: usize = 60;
/// Max characters of a single text leaf value within a preview.
const TEXT_MAX: usize = 40;
/// A raw byte run at least this large is treated as an embedded data blob (image / picture /
/// saved-data / property bag) and highlighted rather than dimmed, so attachments stand out.
const BLOB_BYTES: usize = 512;

/// A node's type name (registry name, or `Type_0xNNNN` for unmodelled types) and raw type word.
fn node_type(node: &Node) -> (String, u16) {
    match node {
        // The only modelled DOM node; every other record surfaces as `Unknown`.
        Node::FieldDef(_) => ("FieldDef".to_string(), 0x0073),
        Node::Unknown(u) => (u.type_name(), u.rtype),
    }
}

/// This node's child records (a modelled leaf has none).
fn node_children(node: &Node) -> &[Node] {
    match node {
        Node::Unknown(u) => &u.children,
        Node::FieldDef(_) => &[],
    }
}

/// The node's kind tag in the tree, in project-agnostic tree vocabulary: `Leaf` when the node has
/// no children, `Branch` when it does. The first tier of the tree is `CfbStream` (the CFB/OLE2
/// spec's term for a stream).
fn node_kind(node: &Node) -> &'static str {
    if node_children(node).is_empty() {
        "Leaf"
    } else {
        "Branch"
    }
}

/// True if this record type is identified in the registry (has a symbolic name).
fn node_is_known(node: &Node) -> bool {
    match node {
        Node::FieldDef(_) => true,
        Node::Unknown(u) => u.tag().name().is_some(),
    }
}

/// The type identity shown inside a node's kind tag: the registry name (e.g. `ReportProperty`)
/// when known, else the bare hex type word (e.g. `0x0066`) — so an unknown type isn't printed as
/// the redundant `Type_0x0066`.
fn node_identity(node: &Node) -> String {
    let (name, tag) = node_type(node);
    if node_is_known(node) {
        name
    } else {
        format!("{tag:#06x}")
    }
}

/// A compact, human-readable preview of a node's own leaf content — the field name/type for a
/// modelled field, else the decoded leaf values (strings quoted, ints inline, raw runs sized).
/// `None` when the node carries no previewable content of its own. With `color`, text content is
/// highlighted, large embedded data blobs are called out in magenta, and small raw byte runs are
/// dimmed; the overall visible width is still capped.
fn node_preview(node: &Node, color: bool) -> Option<String> {
    match node {
        Node::FieldDef(f) => {
            let name = paint(color, YELLOW, &format!("{:?}", f.name));
            let ty = paint(color, CYAN, &format!("{:?}", f.value_type));
            Some(format!("{name} {ty}"))
        }
        Node::Unknown(u) => {
            if u.values.is_empty() {
                return None;
            }
            // Accumulate by *visible* width so embedded ANSI codes never count toward the cap.
            let mut visible = 0usize;
            let mut parts: Vec<String> = Vec::new();
            for v in &u.values {
                let (plain, code) = match v {
                    Value::Text(s) => (format!("{:?}", truncate(s, TEXT_MAX)), YELLOW),
                    Value::Int(i) => (i.to_string(), ""),
                    // Large byte runs are embedded data blobs (images / saved data / property
                    // bags) — call them out in magenta and label them, rather than dimming.
                    Value::Bytes(b) if b.len() >= BLOB_BYTES => {
                        (format!("[{}B blob]", b.len()), BRIGHT_MAGENTA)
                    }
                    Value::Bytes(b) => (format!("[{}B]", b.len()), DIM),
                };
                if visible + plain.chars().count() > PREVIEW_MAX {
                    parts.push(paint(color, DIM, "…"));
                    break;
                }
                visible += plain.chars().count() + 1; // +1 for the joining space
                parts.push(if code.is_empty() {
                    plain
                } else {
                    paint(color, code, &plain)
                });
            }
            Some(parts.join(" "))
        }
    }
}

/// The picture / image / OLE object record types, so the tree can flag embedded images in
/// pink/purple: `0xae` PictureObject (base opener), `0xaf` PictureWrapper (static/OLE image),
/// `0xb1` BlobFieldWrapper (DB-blob picture), `0xbd` OleObjectItem (embedded OLE item detail).
fn is_image_record(tag: u16) -> bool {
    matches!(tag, 0x00ae | 0x00af | 0x00b1 | 0x00bd)
}

/// Paint a node's type label by prominence: a field definition is brightest, a picture/OLE object
/// record is called out in pink/purple, any other recognized (named) record type is highlighted,
/// and an unmodelled `Type_0xNNNN` is dimmed.
fn paint_label(color: bool, node: &Node, name: &str) -> String {
    match node {
        Node::FieldDef(_) => paint(color, BOLD_GREEN, name),
        Node::Unknown(u) if is_image_record(u.tag().0) => paint(color, BRIGHT_MAGENTA, name),
        Node::Unknown(u) if u.tag().name().is_some() => paint(color, CYAN, name),
        _ => paint(color, DIM, name),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TreeNodeJson {
    /// `record` (has nested children) or `leaf` (terminal, field data only).
    kind: &'static str,
    #[serde(rename = "type")]
    type_name: String,
    tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<TreeNodeJson>,
}

#[derive(Serialize)]
struct TreeSubreportJson {
    name: String,
    roots: Vec<TreeNodeJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TreeReport<'a> {
    file: &'a str,
    node_count: usize,
    roots: Vec<TreeNodeJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    subreports: Vec<TreeSubreportJson>,
}

/// Build the JSON node tree for one record, capping recursion at `max_depth` levels.
fn tree_node_json(node: &Node, depth: usize, max_depth: usize) -> TreeNodeJson {
    let (type_name, tag) = node_type(node);
    let kids = node_children(node);
    let (children, truncated) = if depth + 1 >= max_depth && !kids.is_empty() {
        (Vec::new(), true)
    } else {
        (
            kids.iter()
                .map(|c| tree_node_json(c, depth + 1, max_depth))
                .collect(),
            false,
        )
    };
    TreeNodeJson {
        kind: if node_kind(node) == "Leaf" {
            "leaf"
        } else {
            "branch"
        },
        type_name,
        tag: format!("{tag:#06x}"),
        preview: node_preview(node, false),
        truncated,
        children,
    }
}

/// Render one record and its subtree as indented box-drawing lines, capping at `max_depth`.
/// The `prefix` passed down stays uncolored so widths line up; color is applied per printed line.
fn render_node(
    out: &mut String,
    node: &Node,
    prefix: &str,
    is_last: bool,
    depth: usize,
    max_depth: usize,
    color: bool,
) {
    let (_, tag) = node_type(node);
    let branch = if is_last { "└─ " } else { "├─ " };
    let scaffold = paint(color, DIM, &format!("{prefix}{branch}"));
    // `Kind(Identity)` tag — e.g. `Record(Area)` or `Leaf(0x0066)`. The identity keeps its
    // prominence color; the kind word and parens are dim scaffolding.
    let label = format!(
        "{}{}{}",
        paint(color, DIM, &format!("{}(", node_kind(node))),
        paint_label(color, node, &node_identity(node)),
        paint(color, DIM, ")"),
    );
    // Append the raw hex type word for known types (for unknowns the identity already is the hex).
    let hex = if node_is_known(node) {
        format!(" {}", paint(color, DIM, &format!("{tag:#06x}")))
    } else {
        String::new()
    };
    let preview = node_preview(node, color)
        .map(|p| format!("  {p}"))
        .unwrap_or_default();
    let _ = writeln!(out, "{scaffold}{label}{hex}{preview}");

    let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
    let kids = node_children(node);
    if depth + 1 >= max_depth && !kids.is_empty() {
        let more = paint(color, DIM, &format!("└─ … {} more", kids.len()));
        let bars = paint(color, DIM, &child_prefix);
        let _ = writeln!(out, "{bars}{more}");
        return;
    }
    let last = kids.len().saturating_sub(1);
    for (i, child) in kids.iter().enumerate() {
        render_node(
            out,
            child,
            &child_prefix,
            i == last,
            depth + 1,
            max_depth,
            color,
        );
    }
}

/// Render a record forest under `prefix`, starting at `depth`. `prefix` is the (uncolored)
/// scaffolding inherited from any enclosing tier (e.g. a stream group above these roots).
fn render_roots(
    out: &mut String,
    roots: &[Node],
    prefix: &str,
    depth: usize,
    max_depth: usize,
    color: bool,
) {
    let last = roots.len().saturating_sub(1);
    for (i, node) in roots.iter().enumerate() {
        render_node(out, node, prefix, i == last, depth, max_depth, color);
    }
}

/// Total number of nodes across a report's record forest.
fn node_count(roots: &[Node]) -> usize {
    roots.iter().map(Node::count).sum()
}

pub(crate) fn tree(file: &str, json: bool, depth: Option<usize>, color: bool) -> rpt::Result<()> {
    let rpt = Rpt::open(file)?;
    let report = rpt.report();
    let max_depth = depth.unwrap_or(usize::MAX);

    if json {
        let subreports = report
            .subreports
            .iter()
            .map(|s| TreeSubreportJson {
                name: s.name.clone(),
                roots: s
                    .report
                    .records
                    .iter()
                    .map(|n| tree_node_json(n, 0, max_depth))
                    .collect(),
            })
            .collect();
        crate::util::print_json(&TreeReport {
            file,
            node_count: node_count(&report.records),
            roots: report
                .records
                .iter()
                .map(|n| tree_node_json(n, 0, max_depth))
                .collect(),
            subreports,
        });
        return Ok(());
    }

    // First tier of the tree = the source CFB streams. The main report is the `Contents` stream;
    // every subreport is its own `Subdocument N/Contents` stream. Grouping by stream makes it
    // explicit which part of the file each record forest comes from. Each entry is
    // (stream name, optional subreport name, record roots).
    let mut groups: Vec<(&str, Option<&str>, &[Node])> = vec![("Contents", None, &report.records)];
    for sub in &report.subreports {
        groups.push((
            "Subdocument/Contents",
            Some(sub.name.as_str()),
            &sub.report.records,
        ));
    }
    let total_nodes: usize = groups.iter().map(|(_, _, roots)| node_count(roots)).sum();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{file}: {total_nodes} nodes across {} stream(s), {} distinct record types",
        groups.len(),
        report.distinct_record_types(),
    );
    let last = groups.len().saturating_sub(1);
    for (i, (stream, sub_name, roots)) in groups.iter().enumerate() {
        let is_last = i == last;
        let branch = if is_last { "└─ " } else { "├─ " };
        // `CfbStream(<name>)` kind tag; a subreport's name follows as (yellow) content.
        let label = paint(color, BOLD, &format!("CfbStream({stream})"));
        let sub_label = sub_name
            .map(|n| format!("  {}", paint(color, YELLOW, &format!("{n:?}"))))
            .unwrap_or_default();
        let count = paint(color, DIM, &format!("[{} records]", node_count(roots)));
        let _ = writeln!(
            out,
            "{}{label}{sub_label}  {count}",
            paint(color, DIM, branch)
        );
        // The record forest hangs under the stream tier. `--depth` still counts record levels
        // (the stream tier is free), so record roots start at depth 0.
        let child_prefix = if is_last { "   " } else { "│  " };
        render_roots(&mut out, roots, child_prefix, 0, max_depth, color);
    }
    print!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_record_tags_are_flagged() {
        // Picture / image / OLE object record types.
        for tag in [0x00ae, 0x00af, 0x00b1, 0x00bd] {
            assert!(is_image_record(tag), "{tag:#06x} should be an image record");
        }
        // Neighbours and unrelated records are not.
        for tag in [0x00a9, 0x00b0, 0x00be, 0x0064, 0x00ca] {
            assert!(
                !is_image_record(tag),
                "{tag:#06x} should not be an image record"
            );
        }
    }

    #[test]
    fn image_label_uses_pink_purple_when_colored() {
        // The pink/purple flag is the bright-magenta SGR code.
        assert!(BRIGHT_MAGENTA.contains("95"));
        // With color off, no escape codes leak into the label.
        assert_eq!(
            paint(false, BRIGHT_MAGENTA, "PictureObject"),
            "PictureObject"
        );
    }

    use rpt::model::Unknown;

    /// Strip ANSI SGR escape sequences (`\x1b[…m`) so a colored preview can be compared by its
    /// visible content.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn unknown(rtype: u16, values: Vec<Value>) -> Node {
        Node::Unknown(Unknown {
            rtype,
            values,
            ..Default::default()
        })
    }

    #[test]
    fn preview_is_none_without_own_content() {
        assert_eq!(node_preview(&unknown(0x0064, vec![]), false), None);
        assert_eq!(node_preview(&unknown(0x0064, vec![]), true), None);
    }

    #[test]
    fn preview_color_does_not_change_visible_content() {
        // A mix of the previewable value kinds (quoted text, inline int, large blob, small byte run).
        let node = unknown(
            0x0064,
            vec![
                Value::Text("hello".into()),
                Value::Int(42),
                Value::Bytes(vec![0u8; 600]),
                Value::Bytes(vec![0u8; 10]),
            ],
        );
        let plain = node_preview(&node, false).unwrap();
        let colored = node_preview(&node, true).unwrap();
        // Coloring wraps parts in SGR codes but must not alter the visible characters — so the width
        // cap (which counts visible width only) sees the same content either way.
        assert_ne!(plain, colored, "colored output should carry SGR codes");
        assert_eq!(strip_ansi(&colored), plain);
        // A large byte run is labelled as a blob; a small one is sized inline.
        assert!(plain.contains("[600B blob]"), "{plain}");
        assert!(plain.contains("[10B]"), "{plain}");
    }

    #[test]
    fn preview_caps_visible_width_and_marks_overflow() {
        // Far more content than PREVIEW_MAX: the preview is clipped with a trailing ellipsis and the
        // ANSI codes never inflate the width (stripped colored == plain).
        let values: Vec<Value> = (0..40).map(|i| Value::Int(1_000_000 + i)).collect();
        let node = unknown(0x0064, values);
        let plain = node_preview(&node, false).unwrap();
        assert!(
            plain.contains('…'),
            "expected an overflow ellipsis: {plain}"
        );
        // The last value is well past the cap, so it must not appear.
        assert!(!plain.contains("1000039"), "{plain}");
        // Visible width stays near the cap (a couple of chars of slack for the join space + ellipsis).
        assert!(
            plain.chars().count() <= PREVIEW_MAX + 2,
            "visible width {} exceeded cap: {plain}",
            plain.chars().count()
        );
        assert_eq!(strip_ansi(&node_preview(&node, true).unwrap()), plain);
    }

    #[test]
    fn json_leaf_shape_omits_empty_fields() {
        // A leaf with its own content: `kind`/`type`/`tag`/`preview` present; `truncated` and
        // `children` are omitted (default false / empty).
        let node = unknown(0x0064, vec![Value::Int(7)]);
        let v = serde_json::to_value(tree_node_json(&node, 0, usize::MAX)).unwrap();
        assert_eq!(v["kind"], "leaf");
        assert_eq!(v["tag"], "0x0064");
        assert!(v["type"].is_string());
        assert!(v["preview"].is_string());
        assert!(v.get("truncated").is_none(), "false truncated is skipped");
        assert!(v.get("children").is_none(), "empty children is skipped");
    }

    #[test]
    fn json_preview_omitted_when_absent() {
        let node = unknown(0x0064, vec![]);
        let v = serde_json::to_value(tree_node_json(&node, 0, usize::MAX)).unwrap();
        assert!(v.get("preview").is_none());
    }

    #[test]
    fn json_branch_truncates_children_at_max_depth() {
        let child = unknown(0x0001, vec![]);
        let parent = Node::Unknown(Unknown {
            rtype: 0x0002,
            children: vec![child],
            ..Default::default()
        });
        // max_depth 1 collapses the children below the root.
        let v = serde_json::to_value(tree_node_json(&parent, 0, 1)).unwrap();
        assert_eq!(v["kind"], "branch");
        assert_eq!(v["truncated"], true);
        assert!(v.get("children").is_none(), "collapsed children are empty");
    }
}
