//! Conditional-format formula slots: collecting formula bodies and resolving slot refs.

use crate::build_model::data_def::named_value;
use crate::build_model::row_of;
use crate::build_model::tree_search::nodes_where;
use crate::codec::RecordNode;
use crate::field_table::table::{Cell, Row};
use crate::field_table::tables as ft;
use crate::records::rtype::*;
use std::collections::BTreeMap;

/// The conditional-format formula bodies, keyed by the **0-based global index** of their `0x76`
/// record in the Contents stream — the same index a condition slot stores. An owner's slot names
/// the formula by `@<name>` *and* this index, which picks the exact body (disambiguating repeated
/// names without any ordering assumption).
pub(in crate::build_model) fn condition_formula_bodies(
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
        let name = named_value(n, logical).name;
        if is_modeled_condition(&name) {
            map.insert(idx, (name, body));
        }
    }
    map
}

/// The formula text of a `0x76` record for use as a conditional-format formula.
pub(super) fn cond_formula_body(node: &RecordNode, logical: &[u8]) -> String {
    row_of(node, logical, &ft::FORMULA).text("text").to_owned()
}

/// The reserved conditional-format formula names this reader carries onto the model — the full set
/// the engine exposes as an editable per-property condition on an object, section, font or border.
/// Each corresponds to a member of one of the SDK `Cr…ConditionFormulaTypeEnum` vocabularies (object
/// visibility/display-string/graphic-location; section visibility/new-page-before/after/keep-together/
/// suppress-if-blank/reset-page-number/underlay/print-at-bottom/background; font color/style; border
/// colors). A slot naming a string outside this reserved set is not carried (it is not a condition
/// slot). These are the stored formula names as they appear in the bytes; how (and whether) each maps
/// to an output surface is the consumer's concern.
pub(crate) fn is_modeled_condition(name: &str) -> bool {
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
            // font-color conditions
            | "Font_Color"
            | "Font_Style"
            // border conditions
            | "Fore_Color"
    )
}

/// The conditional-format formula references a condition wrapper's slots name, in slot order: a
/// slot's `@`-name with the `@` stripped, paired with the index that picks the formula's body.
///
/// A wrapper is a run of field references, one per property of the record it wraps, and an empty
/// slot names no field — so the occupied ones are exactly the references that carry a name, and
/// only the reserved names are conditions this reader models.
pub(crate) fn condition_slots(row: &Row) -> Vec<(String, usize)> {
    row.iter()
        .filter_map(|(_, v)| match v {
            Cell::Ref { text, index, .. } => {
                let name = text.strip_prefix('@')?;
                let index = usize::from((*index)?);
                is_modeled_condition(name).then(|| (name.to_owned(), index))
            }
            _ => None,
        })
        .collect()
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
