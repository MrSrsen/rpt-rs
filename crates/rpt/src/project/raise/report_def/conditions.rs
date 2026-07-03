//! Conditional-format formula slots: collecting formula bodies and resolving slot refs.

use super::*;

/// The conditional-format formula bodies, keyed by the **0-based global index** of their `0x76`
/// record in the Contents stream — the same index a condition slot stores. An owner's slot names
/// the formula by `@<name>` *and* this index, which picks the exact body (disambiguating repeated
/// names without any ordering assumption).
pub(in crate::project::raise) fn condition_formula_bodies(
    tree: &[RecordNode],
    logical: &[u8],
) -> BTreeMap<usize, (String, String)> {
    let nodes = nodes_where(tree, |n| n.rtype == FORMULA || n.rtype == NAMED_VALUE);
    let mut map: BTreeMap<usize, (String, String)> = BTreeMap::new();
    let mut formula_idx = 0usize; // counts every `0x76` body, matching the slot's global index
    let mut pending: Option<(usize, String)> = None;
    for n in nodes {
        if n.rtype == FORMULA {
            pending = Some((formula_idx, cond_formula_body(n, logical)));
            formula_idx += 1;
            continue;
        }
        let Some((idx, body)) = pending.take() else {
            continue;
        };
        if let Some((name, _)) = read_lp_string(&n.leaf_bytes(logical)) {
            if cond_attr(&name).is_some() {
                map.insert(idx, (name, body));
            }
        }
    }
    map
}

/// The formula text of a `0x76` record for use as a conditional-format formula: the longest
/// non-empty length-prefixed string in the leaf (a plain Crystal expression such as
/// `DrillDownGroupLevel > 0` carries none of the markers `formula_body`'s `is_expr` filter wants,
/// so that filter cannot be used here). [`longest_lp`]'s sliding scan matters: a spurious short
/// match near the start would otherwise jump the scan past the real body's length prefix.
pub(super) fn cond_formula_body(node: &RecordNode, logical: &[u8]) -> String {
    longest_lp(&node.leaf_bytes(logical)).unwrap_or_default()
}

/// Map a reserved conditional-format formula name to the XML attribute emitted for it (on the
/// element selected by the owning record type). Only mapped properties carry an emitted attribute;
/// other reserved names carry none.
pub(super) fn cond_attr(name: &str) -> Option<&'static str> {
    Some(match name {
        "Object_Visibility" | "Section_Visibility" => "EnableSuppress",
        "Section_Back_Color" | "Background_Color" | "Back_Color" => "BackgroundColor",
        "New_Page_After" => "EnableNewPageAfter",
        "Font_Color" => "Color",
        "Font_Style" => "Style",
        // Display-string format formula on a field/text object's `0xfd` condition slots.
        "Display_String" => "DisplayString",
        // Dynamic-image location formula on a PictureObject's `0xfd` condition slots.
        "Graphic_Location" => "GraphicLocation",
        // Border foreground colour condition formula (lives on the `0xed` border wrapper, alongside
        // `Back_Color` → BackgroundColor). The border's *foreground* is its line/border colour.
        "Fore_Color" => "BorderColor",
        _ => return None,
    })
}

/// The conditional-format formula references (`@<name>`, with the `@` stripped) carried by an
/// object/section condition-slot record (`0xfd`/`0xff`/`0x0101`): an occupied slot inlines a
/// length-prefixed `@`-name, an empty one is a fixed sentinel. Only references to mapped reserved
/// names are returned, in record order.
pub(super) fn condition_refs(node: &RecordNode, logical: &[u8]) -> Vec<(String, usize)> {
    let bytes = node.leaf_bytes(logical);
    let mut refs = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if let Some((s, consumed)) = read_lp_string(&bytes[i..]) {
            if let Some(name) = s.strip_prefix('@') {
                if cond_attr(name).is_some() {
                    // After the `@name` LP-string: a u16 LE syntax word, then the global formula
                    // index. The index width differs by owner (`0xfd` object slots leave a 2-byte
                    // index; `0xed` border slots leave a 1-byte index immediately abutting the next
                    // slot's length prefix), so the per-slot stride is NOT fixed. Read the index from
                    // the +2..+4 window (the small index value sits in the low byte regardless), then
                    // advance only past the string and let the byte-scan find the next `@`-name —
                    // never assume a trailer width, or a short index would skip the following slot.
                    // The index is a little-endian value at `+2` (low) / `+3` (high). Read it
                    // byte-wise with missing bytes = 0, not as a fixed 2-byte slice: a slot that ends
                    // the record (a 1-byte index as the leaf's last byte) has no `+3` byte, so a
                    // slice read would fail out-of-bounds and default the index to 0.
                    let lo = bytes.get(i + consumed + 2).copied().unwrap_or(0);
                    let hi = bytes.get(i + consumed + 3).copied().unwrap_or(0);
                    let fml_idx = usize::from(u16::from_le_bytes([lo, hi]));
                    refs.push((name.to_string(), fml_idx));
                    i += consumed;
                    continue;
                }
            }
        }
        i += 1;
    }
    refs
}

/// Resolve each condition reference on an owner record to its `(attribute, formula text)` pair,
/// picking the exact formula body by the slot's global formula index. An empty body means the slot
/// referenced a placeholder, so it is skipped (emitting `attr=""` would be a wrong value).
pub(super) fn resolve_conditions(
    refs: &[(String, usize)],
    bodies: &BTreeMap<usize, (String, String)>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, fml_idx) in refs {
        if let (Some(attr), Some((_, body))) = (cond_attr(name), bodies.get(fml_idx)) {
            if !body.is_empty() {
                out.push((attr.to_string(), body.clone()));
            }
        }
    }
    out
}
