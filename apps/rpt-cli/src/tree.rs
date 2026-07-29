//! `tree` — a structural tree of the decoded records, grouped by source stream.

use std::fmt::Write as _;

use rpt_reader::annotate::{summarize, RecordSummary};
use rpt_reader::raw::{Dialect, Node, Value};
use rpt_reader::Rpt;
use serde::Serialize;

use crate::util::{paint, truncate, CliError, BOLD, BOLD_GREEN, BRIGHT_MAGENTA, CYAN, DIM, YELLOW};

pub(crate) const HELP: &str = "\
rpt tree — a structural tree of the decoded records

Grouped by source stream. Each node is tagged by kind: CfbStream(<name>) is the first tier (the
main report's Contents, then every subreport's Subdocument N/Contents); Branch(<type>) is a node
with nested children; Leaf(<type>) is a node with none (its own field bytes only). <type> is the record's
registry name (or its raw 0xNNNN word). For record types the decoder understands (format records,
group-area options, summary definitions) the node line shows a concise decoded summary of the
record's values (e.g. `DecimalPlaces=2 Negative=…`); every other record shows a truncated preview
of its raw content instead.

The tree highlights by prominence — recognized record types and field/text content stand out,
picture / OLE object records and large embedded data blobs are flagged in pink/purple, and
scaffolding (unknown types, small byte runs, connectors) is dimmed. Color is on by default at a
terminal and off when piped; --color / --no-color (and NO_COLOR / CLICOLOR_FORCE) override that.

USAGE:
    rpt tree <file.rpt> [--json] [--depth N] [--color | --no-color]

    --json node output carries the decoded values as a `decoded` object (in addition to `preview`).

OPTIONS:
    --json         emit the node tree as JSON
    --depth N      limit the tree to N levels deep; deeper nodes are collapsed
    --color        force coloring on even when piped (e.g. `rpt tree f.rpt --color | less -R`)
    --no-color     force coloring off
";

/// Max characters of the preview of a node's own field bytes.
const PREVIEW_MAX: usize = 60;
/// Max characters of a single text value within a preview.
const TEXT_MAX: usize = 40;
/// A raw byte run at least this large is treated as an embedded data blob (image / picture /
/// saved-data / property bag) and highlighted rather than dimmed, so attachments stand out.
const BLOB_BYTES: usize = 512;

/// How one record forest is rendered: the vocabulary its records are written in, the depth cap and
/// whether to color. The dialect travels with the tree because a record carries a type number and
/// not the stream it came from — the same number is an unrelated record in each vocabulary, so
/// every lookup on it needs both.
#[derive(Clone, Copy)]
struct Render {
    dialect: Dialect,
    max_depth: usize,
    color: bool,
}

/// A node's type name (registry name, or `Type_0xNNNN` for unmodelled types) and raw type word.
fn node_type(node: &Node, dialect: Dialect) -> (String, u16) {
    match node {
        // The only modelled node; every other record surfaces as `Unknown`.
        Node::FieldDef(_) => ("FieldDef".to_string(), 0x0073),
        Node::Unknown(u) => (u.type_name(dialect), u.rtype),
    }
}

/// This node's child records, in wire order (a modelled node has none).
fn node_children(node: &Node) -> Vec<&Node> {
    match node {
        Node::Unknown(u) => u.children().collect(),
        Node::FieldDef(_) => Vec::new(),
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

/// True if this record type is identified in the registry (has a symbolic name in `dialect`).
fn node_is_known(node: &Node, dialect: Dialect) -> bool {
    match node {
        Node::FieldDef(_) => true,
        Node::Unknown(u) => u.tag().name(dialect).is_some(),
    }
}

/// The type identity shown inside a node's kind tag: the registry name (e.g. `ReportProperty`)
/// when known, else the bare hex type word (e.g. `0x0066`) — so an unknown type isn't printed as
/// the redundant `Type_0x0066`.
fn node_identity(node: &Node, dialect: Dialect) -> String {
    let (name, tag) = node_type(node, dialect);
    if node_is_known(node, dialect) {
        name
    } else {
        format!("{tag:#06x}")
    }
}

/// A compact, human-readable preview of a node's own field bytes — the field name/type for a
/// modelled field, else the values decoded from its runs (strings quoted, raw runs sized).
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
            let values = u.values();
            if values.is_empty() {
                return None;
            }
            // Accumulate by *visible* width so embedded ANSI codes never count toward the cap.
            let mut visible = 0usize;
            let mut parts: Vec<String> = Vec::new();
            for v in &values {
                let (plain, code) = match v {
                    Value::Text(s) => (format!("{:?}", truncate(s, TEXT_MAX)), YELLOW),
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
                parts.push(paint(color, code, &plain));
            }
            Some(parts.join(" "))
        }
    }
}

/// Render a decoded [`RecordSummary`] as a `key=value key=value` line: with `color`, each key (and
/// the `=`) is dimmed as scaffolding and the decoded value shown in normal weight, so the meaning
/// stands out against the surrounding tree.
fn paint_summary(color: bool, summary: &RecordSummary) -> String {
    summary
        .fields
        .iter()
        .map(|(k, v)| format!("{}{v}", paint(color, DIM, &format!("{k}="))))
        .collect::<Vec<_>>()
        .join(" ")
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
fn paint_label(color: bool, node: &Node, name: &str, dialect: Dialect) -> String {
    match node {
        Node::FieldDef(_) => paint(color, BOLD_GREEN, name),
        Node::Unknown(u) if is_image_record(u.tag().0) => paint(color, BRIGHT_MAGENTA, name),
        Node::Unknown(u) if u.tag().name(dialect).is_some() => paint(color, CYAN, name),
        _ => paint(color, DIM, name),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TreeNodeJson {
    /// `branch` (has nested children) or `leaf` (its content is its own field bytes and nothing
    /// else) — the same two kinds the text view tags a node with, lowercased.
    kind: &'static str,
    #[serde(rename = "type")]
    type_name: String,
    tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
    /// Decoded values for record types the decoder understands (format records, group-area
    /// options, summary definitions), as an ordered `field -> value` object. Absent for records
    /// with no decoder.
    #[serde(skip_serializing_if = "Option::is_none")]
    decoded: Option<serde_json::Map<String, serde_json::Value>>,
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

/// Build the JSON node tree for one record, capping recursion at the render's depth limit.
fn tree_node_json(node: &Node, depth: usize, render: Render) -> TreeNodeJson {
    let (type_name, tag) = node_type(node, render.dialect);
    let kids = node_children(node);
    let (children, truncated) = if depth + 1 >= render.max_depth && !kids.is_empty() {
        (Vec::new(), true)
    } else {
        (
            kids.iter()
                .map(|c| tree_node_json(c, depth + 1, render))
                .collect(),
            false,
        )
    };
    TreeNodeJson {
        kind: if kids.is_empty() { "leaf" } else { "branch" },
        type_name,
        tag: format!("{tag:#06x}"),
        preview: node_preview(node, false),
        decoded: summarize(node, render.dialect).map(|s| {
            s.fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
                .collect()
        }),
        truncated,
        children,
    }
}

/// Render one record and its subtree as indented box-drawing lines, capping at the render's depth
/// limit. The `prefix` passed down stays uncolored so widths line up; color is applied per printed
/// line.
fn render_node(
    out: &mut String,
    node: &Node,
    prefix: &str,
    is_last: bool,
    depth: usize,
    render: Render,
) {
    let Render { dialect, color, .. } = render;
    let (_, tag) = node_type(node, dialect);
    let branch = if is_last { "└─ " } else { "├─ " };
    let scaffold = paint(color, DIM, &format!("{prefix}{branch}"));
    // `Kind(Identity)` tag — e.g. `Branch(Area)` or `Leaf(0x0066)`. The identity keeps its
    // prominence color; the kind word and parens are dim scaffolding.
    let label = format!(
        "{}{}{}",
        paint(color, DIM, &format!("{}(", node_kind(node))),
        paint_label(color, node, &node_identity(node, dialect), dialect),
        paint(color, DIM, ")"),
    );
    // Append the raw hex type word for known types (for unknowns the identity already is the hex).
    let hex = if node_is_known(node, dialect) {
        format!(" {}", paint(color, DIM, &format!("{tag:#06x}")))
    } else {
        String::new()
    };
    // A recognized record shows its decoded value summary; every other record shows its raw
    // content preview.
    let detail = match summarize(node, dialect) {
        Some(s) => Some(paint_summary(color, &s)),
        None => node_preview(node, color),
    };
    let detail = detail.map(|p| format!("  {p}")).unwrap_or_default();
    let _ = writeln!(out, "{scaffold}{label}{hex}{detail}");

    let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
    let kids = node_children(node);
    if depth + 1 >= render.max_depth && !kids.is_empty() {
        let more = paint(color, DIM, &format!("└─ … {} more", kids.len()));
        let bars = paint(color, DIM, &child_prefix);
        let _ = writeln!(out, "{bars}{more}");
        return;
    }
    let last = kids.len().saturating_sub(1);
    for (i, child) in kids.iter().enumerate() {
        render_node(out, child, &child_prefix, i == last, depth + 1, render);
    }
}

/// Render a record forest under `prefix`, starting at `depth`. `prefix` is the (uncolored)
/// scaffolding inherited from any enclosing tier (e.g. a stream group above these roots).
fn render_roots(out: &mut String, roots: &[Node], prefix: &str, depth: usize, render: Render) {
    let last = roots.len().saturating_sub(1);
    for (i, node) in roots.iter().enumerate() {
        render_node(out, node, prefix, i == last, depth, render);
    }
}

/// Total number of nodes across a report's record forest.
fn node_count(roots: &[Node]) -> usize {
    roots.iter().map(Node::count).sum()
}

pub(crate) fn tree(
    file: &str,
    json: bool,
    depth: Option<usize>,
    color: bool,
) -> Result<(), CliError> {
    let rpt = Rpt::open(file)?;
    let report = rpt.report();
    // The typed record tree is built on demand from the decoded records (not stored on the model).
    // Both trees come from the report definition — the `Contents` stream and each subreport's own —
    // so that is the vocabulary every record below is named and decoded in. It is stated once here,
    // where the trees are obtained, because a node itself does not say which stream it came from.
    let render = Render {
        dialect: Dialect::Contents,
        max_depth: depth.unwrap_or(usize::MAX),
        color,
    };
    let main_dom = rpt.typed_record_tree();
    let sub_doms = rpt.subreport_typed_record_trees();

    if json {
        let subreports = report
            .subreports
            .iter()
            .zip(sub_doms.iter())
            .map(|(s, dom)| TreeSubreportJson {
                name: s.name.clone(),
                roots: dom.iter().map(|n| tree_node_json(n, 0, render)).collect(),
            })
            .collect();
        crate::util::print_json(&TreeReport {
            file,
            node_count: node_count(&main_dom),
            roots: main_dom
                .iter()
                .map(|n| tree_node_json(n, 0, render))
                .collect(),
            subreports,
        });
        return Ok(());
    }

    // First tier of the tree = the source CFB streams. The main report is the `Contents` stream;
    // every subreport is its own `Subdocument N/Contents` stream. Grouping by stream makes it
    // explicit which part of the file each record forest comes from. Each entry is
    // (stream name, optional subreport name, record roots).
    let mut groups: Vec<(&str, Option<&str>, &[Node])> = vec![("Contents", None, &main_dom)];
    for (sub, dom) in report.subreports.iter().zip(sub_doms.iter()) {
        groups.push(("Subdocument/Contents", Some(sub.name.as_str()), dom));
    }
    let total_nodes: usize = groups.iter().map(|(_, _, roots)| node_count(roots)).sum();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{file}: {total_nodes} nodes across {} stream(s), {} distinct record types",
        groups.len(),
        rpt.inventory().len(),
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
        render_roots(&mut out, roots, child_prefix, 0, render);
    }
    print!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_reader::raw::Part;

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

    use rpt_reader::raw::Unknown;

    /// The report definition's vocabulary, which every fixture below is a record of.
    const CONTENTS: Render = Render {
        dialect: Dialect::Contents,
        max_depth: usize::MAX,
        color: false,
    };

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

    /// A record whose content is one run of its own field bytes and no nested record.
    fn unknown(rtype: u16, run: Vec<u8>) -> Node {
        Node::Unknown(Unknown {
            rtype,
            schema: 0x0700,
            parts: vec![Part::Run(run)],
        })
    }

    /// A length-prefixed, NUL-terminated string, as the record layer stores one.
    fn lp(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u32 + 1).to_be_bytes().to_vec();
        out.extend_from_slice(text.as_bytes());
        out.push(0);
        out
    }

    /// A node is labelled in the vocabulary the stream it came from is written in: `0x0008` is the
    /// report definition's font record and the query engine's index record, and neither tree may be
    /// labelled out of the other's table.
    #[test]
    fn a_nodes_identity_follows_the_vocabulary_it_is_read_in() {
        let node = unknown(0x0008, vec![]);
        assert_eq!(node_identity(&node, Dialect::Contents), "Font");
        assert_eq!(node_identity(&node, Dialect::QeSession), "QeIndex");
    }

    #[test]
    fn preview_is_none_without_own_content() {
        assert_eq!(node_preview(&unknown(0x0064, vec![]), false), None);
        assert_eq!(node_preview(&unknown(0x0064, vec![]), true), None);
    }

    #[test]
    fn preview_color_does_not_change_visible_content() {
        // A mix of the previewable value kinds: quoted text, a large blob, a small byte run.
        let mut run = lp("hello");
        run.extend_from_slice(&[0u8; 600]);
        run.extend_from_slice(&lp("x"));
        run.extend_from_slice(&[0u8; 10]);
        let node = unknown(0x0064, run);
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
        let run: Vec<u8> = (0..40).flat_map(|i| lp(&format!("val{i:02}"))).collect();
        let node = unknown(0x0064, run);
        let plain = node_preview(&node, false).unwrap();
        assert!(
            plain.contains('…'),
            "expected an overflow ellipsis: {plain}"
        );
        // The last value is well past the cap, so it must not appear.
        assert!(!plain.contains("val39"), "{plain}");
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
        let node = unknown(0x0064, vec![0, 0, 0, 7]);
        let v = serde_json::to_value(tree_node_json(&node, 0, CONTENTS)).unwrap();
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
        let v = serde_json::to_value(tree_node_json(&node, 0, CONTENTS)).unwrap();
        assert!(v.get("preview").is_none());
    }

    /// A real `0x0088` GroupAreaFormat, from a report authored with the group limit set to 2: four
    /// scalars, a nested `0x0151`, then six more. RepeatGroupHeader = 1 and KeepGroupTogether = 0
    /// are in the first run; VisibleGroupNumberPerPage = 2 is in the second, past the child.
    fn group_area_node() -> Node {
        Node::Unknown(Unknown {
            rtype: 0x0088,
            schema: 0x0700,
            parts: vec![
                Part::Run(vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
                Part::Child {
                    framed_len: 6,
                    node: unknown(0x0151, Vec::new()),
                },
                Part::Run(vec![
                    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xff, 0xff,
                ]),
            ],
        })
    }

    #[test]
    fn recognized_record_line_shows_decoded_summary_not_raw_preview() {
        let mut out = String::new();
        render_node(&mut out, &group_area_node(), "", true, 0, CONTENTS);
        // The decoded summary replaces the raw byte preview.
        assert!(out.contains("VisibleGroupNumberPerPage=2"), "{out}");
        assert!(out.contains("RepeatGroupHeader=true"), "{out}");
        assert!(
            !out.contains("[8B]"),
            "raw preview should be replaced: {out}"
        );
    }

    #[test]
    fn paint_summary_color_does_not_change_visible_content() {
        let s = summarize(&group_area_node(), Dialect::Contents).unwrap();
        let plain = paint_summary(false, &s);
        let colored = paint_summary(true, &s);
        assert_ne!(plain, colored, "colored output should carry SGR codes");
        assert_eq!(strip_ansi(&colored), plain);
        assert_eq!(
            plain,
            "RepeatGroupHeader=true KeepGroupTogether=false VisibleGroupNumberPerPage=2"
        );
    }

    #[test]
    fn json_recognized_record_carries_decoded_object_and_keeps_preview() {
        let v = serde_json::to_value(tree_node_json(&group_area_node(), 0, CONTENTS)).unwrap();
        // Backward-compatible fields are still present.
        assert!(v["preview"].is_string());
        // The decoded values are an object keyed by field name.
        assert_eq!(v["decoded"]["VisibleGroupNumberPerPage"], "2");
        assert_eq!(v["decoded"]["RepeatGroupHeader"], "true");
    }

    #[test]
    fn json_decoded_omitted_for_unrecognized_record() {
        let v =
            serde_json::to_value(tree_node_json(&unknown(0x0064, vec![1]), 0, CONTENTS)).unwrap();
        assert!(v.get("decoded").is_none());
    }

    #[test]
    fn json_branch_truncates_children_at_max_depth() {
        let parent = Node::Unknown(Unknown {
            rtype: 0x0002,
            schema: 0x0700,
            parts: vec![Part::Child {
                framed_len: 6,
                node: unknown(0x0001, Vec::new()),
            }],
        });
        // max_depth 1 collapses the children below the root.
        let v = serde_json::to_value(tree_node_json(
            &parent,
            0,
            Render {
                max_depth: 1,
                ..CONTENTS
            },
        ))
        .unwrap();
        assert_eq!(v["kind"], "branch");
        assert_eq!(v["truncated"], true);
        assert!(v.get("children").is_none(), "collapsed children are empty");
    }
}
