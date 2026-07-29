//! Print options — page geometry and the DEVMODE orientation/size/source.

use super::row_of;
use super::tree_search::nodes_where;
use crate::codec::RecordNode;
use crate::field_table::cursor::{Piece, RecordContent, StringFormat};
use crate::field_table::table::{read_strings, Row, Table};
use crate::field_table::tables as ft;
use crate::field_table::tables::tabled_schema;
use crate::model::{PrintOptions, Twips};

/// The first record of `table`'s type in the tree, read through it.
///
/// A record type the stream reuses for a second, unrelated record is matched on its schema word as
/// well, so the reading never lands on the other one.
fn read_first(tree: &[RecordNode], logical: &[u8], table: &Table) -> Option<Row> {
    let want = tabled_schema(table.rtype, crate::codec::Dialect::Contents);
    nodes_where(tree, |n| {
        n.rtype == table.rtype && want.is_none_or(|s| s == n.schema)
    })
    .first()
    .map(|n| row_of(n, logical, table))
}

/// SDK `PrintOptions`: the page margins (`0x66`), the printable content size (`0x18e` paper
/// rectangle less the margins) and the DEVMODE orientation / paper size / source (`0x07`). The
/// printer driver / name / port come from the printer record (`0x03`); the printer name is emitted
/// empty. PrinterDuplex sits in the DEVMODE's variable-offset tail and is left at Default.
pub(super) fn build_print_options(tree: &[RecordNode], logical: &[u8]) -> PrintOptions {
    // Portrait / FormSource are the defaults when the page-setup DEVMODE record is absent.
    let mut opts = PrintOptions {
        paper_orientation: crate::model::PaperOrientation::Portrait,
        paper_source: crate::model::PaperSource::FormSource,
        ..Default::default()
    };

    // A plausible page dimension (positive twips). The upper bound is generous — custom/driver
    // "paper" can be very wide (e.g. a 150-inch data-export page) — and only guards against a
    // mis-read field; the dedicated `0x18e` record makes a false positive unlikely.
    let sane = |v: Option<i32>| v.filter(|&x| (1..=1_000_000).contains(&x));

    // The page-setup record (0x66): the four margins in twips. A margin stored as i32::MIN
    // (`0x80000000`) is the engine's "use default" sentinel and resolves to 360 twips (¼ inch).
    if let Some(row) = read_first(tree, logical, &ft::PAGE_SETUP) {
        let margin = |name: &str| {
            let v = row.i(name);
            Twips(if v == i32::MIN { 360 } else { v })
        };
        opts.margins = crate::model::PageMargins {
            left: margin("margin_left"),
            right: margin("margin_right"),
            top: margin("margin_top"),
            bottom: margin("margin_bottom"),
        };
    }

    // The page DEVMODE (0x07): the printer's orientation, paper size, source and duplex.
    if let Some(row) = read_first(tree, logical, &ft::PAGE_DEVMODE) {
        let dm = devmode_of(&row);
        if let Some(o) = dm.orientation {
            opts.paper_orientation = o;
        }
        if let Some(sz) = dm.paper_size {
            opts.paper_size = sz;
        }
        if let Some(src) = dm.source {
            opts.paper_source = src;
        }
        if let Some(d) = dm.duplex {
            opts.printer_duplex = d;
        }
    }

    // The page rectangle (0x18e): the paper width then height as big-endian u32 twips.
    // PageContentWidth/Height are the printable area — the paper dimensions less the margins. The
    // engine reports these directly from the stored rect's edge order and does NOT re-orient it to
    // `PaperOrientation`: a landscape report whose rect is stored portrait-first (A4 11906×16838)
    // reports portrait content dims (width < height) with the orientation carried only by the flag.
    if let Some(row) = read_first(tree, logical, &ft::PAPER_RECT) {
        let dim = |name: &str| sane(Some(row.i(name)));
        if let (Some(paper_w), Some(paper_h)) = (dim("paper_width"), dim("paper_height")) {
            let cw = paper_w - opts.margins.left.0 - opts.margins.right.0;
            let ch = paper_h - opts.margins.top.0 - opts.margins.bottom.0;
            if cw > 0 && ch > 0 {
                opts.content_width = Twips(cw);
                opts.content_height = Twips(ch);
            }
        }
    }

    // No page rectangle stored: a standard paper size implies the sheet, so the printable area is
    // those dimensions (oriented by orientation) less the margins.
    if opts.content_width.0 == 0 && opts.content_height.0 == 0 {
        if let Some((short, long)) = opts.paper_size.std_dims() {
            let landscape = opts.paper_orientation == crate::model::PaperOrientation::Landscape;
            let (paper_w, paper_h) = if landscape {
                (long, short)
            } else {
                (short, long)
            };
            let cw = paper_w - opts.margins.left.0 - opts.margins.right.0;
            let ch = paper_h - opts.margins.top.0 - opts.margins.bottom.0;
            if cw > 0 && ch > 0 {
                opts.content_width = Twips(cw);
                opts.content_height = Twips(ch);
            }
        }
    }
    // Multi-column detail layout (0x6c): the "Format with Multiple Columns" label grid, a
    // report-level singleton. The column width is 0 unless multi-column is enabled, so it also
    // serves as the on/off signal (a separate enable flag is not needed to detect it). The engine
    // stores **no** column count — it fits as many column-width columns as span the printable width
    // — so we derive it as `content_width / (width + gap)`.
    if let Some(row) = read_first(tree, logical, &ft::MULTI_COLUMN) {
        let col_w = row.i("column_width");
        if col_w > 0 {
            let gap_h = row.i("gap_h");
            let gap_v = row.i("gap_v");
            /// The direction that fills a column top-to-bottom first; anything else is
            /// across-then-down.
            const DOWN_THEN_ACROSS: u32 = 1;

            let down_then_across = row.u("direction") == DOWN_THEN_ACROSS;
            let pitch = col_w + gap_h;
            let columns = if pitch > 0 && opts.content_width.0 > 0 {
                (opts.content_width.0 + gap_h) / pitch
            } else {
                1
            };
            if columns >= 2 {
                opts.multi_column = Some(crate::model::MultiColumn {
                    columns: columns as u16,
                    column_width: Twips(col_w),
                    gap_h: Twips(gap_h),
                    gap_v: Twips(gap_v),
                    across_then_down: !down_then_across,
                });
            }
        }
    }

    // The printer record (0x03): driver ("winspool"), the saved device name, and the port, mapping
    // to the SDK `SavedDriverName` / `SavedPrinterName` / `SavedPortName`. The live `printer_name`
    // (SDK `PrinterName`) is reported empty by the engine, so it is kept empty here. A report saved
    // with no printer writes a different record under the same type number, which `read_first`
    // filters out by its schema word.
    if let Some(row) = read_first(tree, logical, &ft::PRINTER) {
        opts.driver_name = Some(row.text("driver_name").to_owned());
        opts.printer_name = String::new();
        opts.saved_printer_name = row.text("saved_printer_name").to_owned();
        opts.port_name = Some(row.text("port_name").to_owned());
    }
    opts
}

/// The page DEVMODE members this reader surfaces, each `None` when the record does not carry it.
pub(crate) struct Devmode {
    pub(crate) orientation: Option<crate::model::PaperOrientation>,
    pub(crate) paper_size: Option<crate::model::PaperSize>,
    pub(crate) source: Option<crate::model::PaperSource>,
    pub(crate) duplex: Option<crate::model::PrinterDuplex>,
}

/// The four DEVMODE members the SDK exposes, from a `0x0007` reading. A member the record's
/// `dmFields` mask does not select is absent from the row, and stays `None` here.
fn devmode_of(row: &Row) -> Devmode {
    let member = |name: &str| {
        row.get(name)
            .and_then(super::super::field_table::table::Cell::u)
    };
    Devmode {
        orientation: member("orientation")
            .map(|v| crate::model::PaperOrientation::from_code(v as i32)),
        paper_size: member("paper_size").map(|v| crate::model::PaperSize::from_code(v as i32)),
        source: member("default_source").map(|v| crate::model::PaperSource::from_code(v as i32)),
        duplex: member("duplex").map(|v| crate::model::PrinterDuplex::from_code(v as i32)),
    }
}

/// Decode a page-setup DEVMODE (`0x0007`) run. The record is pure field data, so its bytes are
/// wrapped as a single content run and read through the record's own field table.
///
/// The content is assembled here rather than read from a node, so there is no header to take the
/// string framing from and the reading names the enhanced form — the one the record-tree reader
/// admits a header for.
pub(crate) fn decode_devmode(b: &[u8]) -> Devmode {
    let content = RecordContent {
        rtype: ft::PAGE_DEVMODE.rtype,
        schema: 0x0700,
        pieces: vec![Piece::Run(b.to_vec())],
    };
    devmode_of(&read_strings(&ft::PAGE_DEVMODE, &content, StringFormat::Enhanced).row)
}

#[cfg(test)]
mod tests {
    //! Decoded against the committed public fixture corpus. Only numeric layout values are
    //! asserted; the printer test asserts shape, never the device string.

    /// Open a committed fixture under `tests/fixtures/reports/`.
    fn fixture(rel: &str) -> crate::Rpt {
        let path = rpt_test_support::fixture("tests/fixtures/reports").join(rel);
        crate::Rpt::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
    }

    /// The multi-column report decodes to 3 columns; the geometry reproduces the engine's render
    /// (column pitch ≈ 3810 twips tiling the 11520-twip printable width three across).
    #[test]
    fn multi_column_us_states() {
        let rpt = fixture("worrall/USStatesWithAbbreviations.rpt");
        let mc = rpt
            .report()
            .print_options
            .multi_column
            .expect("US States is a multi-column report");
        assert_eq!(mc.columns, 3);
        assert_eq!(mc.column_width.0, 3816);
        assert_eq!(mc.gap_h.0, 0);
        assert_eq!(mc.gap_v.0, 0);
        assert!(mc.across_then_down);
        // The stored pitch tiles the printable width three across.
        assert!((mc.column_width.0 + mc.gap_h.0 - 3810).abs() <= 20);
    }

    /// A report with a saved printer decodes `SavedPrinterName` (the DEVMODE device string) while
    /// the live `PrinterName` stays empty. Asserts shape only, never the (schema-carrying) value.
    #[test]
    fn saved_printer_name_populated() {
        let rpt = fixture("benbrahim777/Customer List.rpt");
        let po = &rpt.report().print_options;
        assert!(
            !po.saved_printer_name.is_empty(),
            "saved printer device name decoded"
        );
        assert!(po.printer_name.is_empty(), "live printer name stays empty");
    }

    /// The single-column control report has no multi-column layout.
    #[test]
    fn single_column_alpha_isos() {
        let rpt = fixture("worrall/AlphaISOsByCountry.rpt");
        assert!(rpt.report().print_options.multi_column.is_none());
    }
}
