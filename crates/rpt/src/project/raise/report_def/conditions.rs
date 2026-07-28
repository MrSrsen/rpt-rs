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
            if is_modeled_condition(&name) {
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

/// The reserved conditional-format formula names this reader carries onto the model — the full set
/// the engine exposes as an editable per-property condition on an object, section, font or border.
/// Each corresponds to a member of one of the SDK `Cr…ConditionFormulaTypeEnum` vocabularies (object
/// visibility/display-string/graphic-location; section visibility/new-page-before/after/keep-together/
/// suppress-if-blank/reset-page-number/underlay/print-at-bottom/background; font colour/style; border
/// colours). A slot naming a string outside this reserved set is not carried (it is not a condition
/// slot). These are the stored formula names as they appear in the bytes; how (and whether) each maps
/// to an output surface is the consumer's concern.
pub(super) fn is_modeled_condition(name: &str) -> bool {
    matches!(
        name,
        // object-format conditions
        "Object_Visibility"
            | "Display_String"
            | "Graphic_Location"
            // section-area conditions
            | "Section_Visibility"
            | "New_Page_After"
            | "New_Page_Before"
            | "Reset_Page_Number_After"
            | "Keep_Together"
            | "Suppress_if_Blank"
            | "Underlay_Following_Sections"
            | "Print_at_Bottom_of_Page"
            | "Hide_for_Drilldown"
            | "Section_Back_Color"
            | "Background_Color"
            | "Back_Color"
            // font-colour conditions
            | "Font_Color"
            | "Font_Style"
            // border conditions
            | "Fore_Color"
    )
}

/// The conditional-format formula references (`@<name>`, with the `@` stripped) carried by an
/// object/section condition-slot record (`0xfd`/`0xff`/`0x0101`): an occupied slot inlines a
/// length-prefixed `@`-name, an empty one is a fixed sentinel. Only references to modeled reserved
/// names are returned, in record order.
pub(super) fn condition_refs(node: &RecordNode, logical: &[u8]) -> Vec<(String, usize)> {
    let bytes = node.leaf_bytes(logical);
    let mut refs = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if let Some((s, consumed)) = read_lp_string(&bytes[i..]) {
            if let Some(name) = s.strip_prefix('@') {
                if is_modeled_condition(name) {
                    // Occupied slot layout: [u32 BE nameLen]['@'+name+NUL][u16 LE syntax = 0x0001]
                    // [index], so the formula index's low byte sits at `string_end + 2`. The index's
                    // byte width is the one owner-dependent detail: `0xfd` object slots store a
                    // 2-byte (u16 LE) index; `0xed` border slots store a 1-byte index abutting the
                    // next slot's u32-BE length prefix — but that prefix's leading byte is `0x00`, so
                    // the value read at `+3` is `0x00` either way. Reading the index as a u16 LE at
                    // `+2`/`+3` is therefore exact for both; advance only past the string and let the
                    // byte-scan re-anchor on the next `@`-name rather than assume a trailer width
                    // (missing bytes read as 0, since a 1-byte index at the leaf's end has no `+3`).
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

/// Resolve each condition reference on an owner record to its `(reserved name, formula text)` pair,
/// picking the exact formula body by the slot's global formula index. An empty body means the slot
/// referenced a placeholder, so it is skipped (carrying an empty formula would be a wrong value).
/// The key is the stored reserved formula name (`refs` is already filtered to modeled names).
pub(super) fn resolve_conditions(
    refs: &[(String, usize)],
    bodies: &BTreeMap<usize, (String, String)>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, fml_idx) in refs {
        if let Some((_, body)) = bodies.get(fml_idx) {
            if !body.is_empty() {
                out.push((name.clone(), body.clone()));
            }
        }
    }
    out
}
