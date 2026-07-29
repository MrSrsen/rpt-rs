//! One currency symbol per page (SDK `OneCurrencySymbolPerPage`).
//!
//! A field with the flag set prints its currency symbol on the **first printed value of that field
//! on each page** and blanks it on every later value on the same page. Which value is a page's first
//! is not a property of the value, so it cannot be decided in the value formatter: a band's text is
//! resolved *before* the page-break decision that follows it, so at format time the value's page is
//! not yet settled and the band may still move. This pass therefore runs after pagination, over the
//! recorded fixups, when page membership is final.
//!
//! It is page-local by construction — the rule reads only the ops of the page it is on — so a single
//! page re-formatted on its own reproduces the same result.

use crate::CurrencyFixup;
use rpt_format_value::{format_currency, CurrencyFormat};
use rpt_model::Twips;
use rpt_pages::{DrawOp, Page, TextLayout};
use std::collections::HashSet;

/// Blank the repeated currency symbols on every page.
///
/// `fixups` are in emission order, which is print order (ops are only ever appended, and a page's
/// emission order is the order the engine formats its bands in), so the first fixup of an object on
/// a page is that object's first printed value there.
pub(crate) fn apply(pages: &mut [Page], fixups: &[CurrencyFixup], text_layout: &dyn TextLayout) {
    if fixups.is_empty() {
        return;
    }
    let mut current_page = usize::MAX;
    let mut symbol_used: HashSet<&str> = HashSet::new();
    for fx in fixups {
        if fx.page_index != current_page {
            current_page = fx.page_index;
            symbol_used.clear();
        }
        let Some(DrawOp::Text(run)) = pages
            .get_mut(fx.page_index)
            .and_then(|p| p.ops.get_mut(fx.op_index))
        else {
            continue;
        };
        let blanked = format_currency(
            fx.mark.value,
            &CurrencyFormat {
                symbol: symbol_blank(&fx.mark.spec.symbol),
                ..fx.mark.spec.clone()
            },
        );
        // A value that printed no symbol in the first place — a suppressed zero, or one replaced by a
        // zero literal, both of which replace the whole rendering — neither claims the page's symbol
        // nor needs rewriting.
        if blanked == run.text {
            continue;
        }
        // The page's first printed value of this object keeps its symbol.
        if symbol_used.insert(fx.object.as_str()) {
            continue;
        }
        if let Some(m) = run.metrics.as_mut() {
            m.advance = Twips(crate::text::spaced_width_twips(
                text_layout,
                &blanked,
                &run.font,
                run.character_spacing,
            ) as i32);
        }
        run.text = blanked;
    }
}

/// The blank the engine leaves where the symbol was: the symbol is replaced by **one space per
/// character plus one**, not deleted, so the amount keeps a leading gap. The count follows the stored
/// symbol's length rather than its drawn width — a one-character symbol is blanked by two spaces
/// whether it is a narrow `$` or a wide `W` — so it is independent of the injected text stack.
fn symbol_blank(symbol: &str) -> String {
    " ".repeat(symbol.chars().count() + 1)
}
