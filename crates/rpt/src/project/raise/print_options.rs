//! Print options — page geometry and the DEVMODE orientation/size/source.

use super::*;

/// SDK `PrintOptions`: the page margins (`0x66`), the printable content size (`0x18e` paper
/// rectangle less the margins) and the DEVMODE orientation / paper size / source (`0x07`). The
/// printer driver / name / port come from the printer record (`0x03`); the printer name is emitted
/// empty. PrinterDuplex sits in the DEVMODE's variable-offset tail and is left at Default.
pub(super) fn raise_print_options(tree: &[RecordNode], logical: &[u8]) -> PrintOptions {
    // Portrait / FormSource are the defaults when the page-setup DEVMODE record is absent.
    let mut opts = PrintOptions {
        paper_orientation: crate::model::PaperOrientation::Portrait,
        paper_source: crate::model::PaperSource::FormSource,
        ..Default::default()
    };

    // A plausible page dimension (positive twips). The upper bound is generous — custom/driver
    // "paper" can be very wide (e.g. a 150-inch data-export page) — and only guards against a
    // mis-read leaf; the dedicated `0x18e` record makes a false positive unlikely.
    let sane = |v: Option<i32>| v.filter(|&x| (1..=1_000_000).contains(&x));

    // The page-setup record (0x66): the four margins as big-endian u32 twips — Left, Right, Top,
    // Bottom, after a 3-byte header. A margin stored as i32::MIN (`0x80000000`) is the engine's
    // "use default" sentinel and resolves to 360 twips (¼ inch, Crystal's default).
    if let Some(b) = leaves_of(tree, logical, PAGE_SETUP).into_iter().next() {
        let margin =
            |off: usize| i32_be(&b, off).map(|v| Twips(if v == i32::MIN { 360 } else { v }));
        if let (Some(l), Some(r), Some(t), Some(bm)) =
            (margin(3), margin(7), margin(11), margin(15))
        {
            opts.margins = crate::model::PageMargins {
                left: l,
                right: r,
                top: t,
                bottom: bm,
            };
        }
    }

    // The page DEVMODE (0x07): a Crystal-compacted big-endian variant of the Win32 `DEVMODEW`
    // printer struct. Rather than the full struct, it stores an 8-byte header
    // (sub-type[0..2], the low word of `dmFields`[2..4], `dmOrientation`[4..6], `dmPaperSize`[6..8])
    // then one big-endian u16 for every *further* printer-union member whose `dmFields` bit is set,
    // in DEVMODE struct order. `DM_ORIENTATION`/`DM_PAPERSIZE` are always set (so those two members
    // occupy the fixed header slots); the remaining members are walked in order, advancing two bytes
    // per present member — so `dmDefaultSource` (paper source) and `dmDuplex` sit at offsets that
    // shift with which earlier members are present. Members past `dmCollate` and any trailing bytes
    // are ignored (only orientation / paper size / source / duplex are exposed by the SDK).
    if let Some(b) = leaves_of(tree, logical, PAPER_DEVMODE).into_iter().next() {
        let dm = decode_devmode(&b);
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
    if let Some(b) = leaves_of(tree, logical, PAPER_RECT).into_iter().next() {
        if let (Some(paper_w), Some(paper_h)) = (sane(i32_be(&b, 0)), sane(i32_be(&b, 4))) {
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
    // report-level singleton. Field order (big-endian): leftMargin, topMargin, **labelWidth**,
    // labelHeight, horizontalGap, verticalGap (u16 twips), then downThenAcross (u32 bool). The
    // label (column) width sits at leaf offset 0x0c; it is 0 unless multi-column is enabled, so it
    // also serves as the on/off signal (a separate multi-column-enable flag is not needed to detect
    // it). The engine stores **no** column count — it fits as many label-width columns as span the
    // printable width — so we derive it as `content_width / (width + gap)`.
    if let Some(b) = leaves_of(tree, logical, MULTI_COLUMN).into_iter().next() {
        let col_w = u16_be(&b, 0x0c).map(i32::from).unwrap_or(0);
        if col_w > 0 {
            let gap_h = u16_be(&b, 0x10).map(i32::from).unwrap_or(0);
            let gap_v = u16_be(&b, 0x12).map(i32::from).unwrap_or(0);
            // downThenAcross == 1 means fill a column top-to-bottom first; anything else (0, or the
            // -1 "unset" default) is the usual across-then-down flow.
            let down_then_across = u32_be(&b, 0x14) == Some(1);
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

    if let Some(node) = tree.iter().find(|n| n.rtype == PRINTER) {
        let strings = all_strings(node, logical);
        // Order: driver ("winspool"), saved printer/device name, port. `driver_name`/`port_name` map
        // to the SDK `SavedDriverName`/`SavedPortName`; string[1] is the `SavedPrinterName` device
        // string (e.g. a network printer path). The live `printer_name` (SDK `PrinterName`) is
        // reported empty by the engine, so it is kept empty here.
        opts.driver_name = strings.first().cloned();
        opts.printer_name = String::new();
        opts.saved_printer_name = strings.get(1).cloned().unwrap_or_default();
        opts.port_name = strings.get(2).cloned();
    }
    opts
}

/// The page DEVMODE members this project surfaces, each `None` when the leaf does not carry it.
pub(crate) struct Devmode {
    pub(crate) orientation: Option<crate::model::PaperOrientation>,
    pub(crate) paper_size: Option<crate::model::PaperSize>,
    pub(crate) source: Option<crate::model::PaperSource>,
    pub(crate) duplex: Option<crate::model::PrinterDuplex>,
}

/// Decode a page-setup DEVMODE (`0x07`) leaf: a Crystal-compacted big-endian variant of the Win32
/// `DEVMODEW` printer struct. Rather than the full struct, it stores an 8-byte header
/// (sub-type `[0..2]`, the low word of `dmFields` `[2..4]`, `dmOrientation` `[4..6]`, `dmPaperSize`
/// `[6..8]`) then one big-endian `u16` for every *further* printer-union member whose `dmFields` bit
/// is set, in DEVMODE struct order. `DM_ORIENTATION`/`DM_PAPERSIZE` are always set (so those two
/// members occupy the fixed header slots); the remaining members are walked in order, advancing two
/// bytes per present member — so `dmDefaultSource` (paper source) and `dmDuplex` sit at offsets that
/// shift with which earlier members are present. Members past `dmCollate` and any trailing bytes are
/// ignored (only orientation / paper size / source / duplex are exposed by the SDK).
pub(crate) fn decode_devmode(b: &[u8]) -> Devmode {
    let u16be = |off: usize| u16_be(b, off).map(i32::from);
    let mut dm = Devmode {
        orientation: u16be(4).map(crate::model::PaperOrientation::from_code),
        paper_size: u16be(6).map(crate::model::PaperSize::from_code),
        source: None,
        duplex: None,
    };

    let dm_fields = u16_be(b, 2).unwrap_or(0);
    // The printer-union members that follow `dmPaperSize` in DEVMODE struct order, each with its
    // `DM_*` `dmFields` bit. Every member whose bit is set contributes one big-endian u16.
    const DM_PAPERLENGTH: u16 = 0x0004;
    const DM_PAPERWIDTH: u16 = 0x0008;
    const DM_SCALE: u16 = 0x0010;
    const DM_COPIES: u16 = 0x0100;
    const DM_DEFAULTSOURCE: u16 = 0x0200;
    const DM_PRINTQUALITY: u16 = 0x0400;
    const DM_COLOR: u16 = 0x0800;
    const DM_DUPLEX: u16 = 0x1000;
    const DM_YRESOLUTION: u16 = 0x2000;
    const DM_TTOPTION: u16 = 0x4000;
    const DM_COLLATE: u16 = 0x8000;
    const TAIL_MEMBERS: [u16; 11] = [
        DM_PAPERLENGTH,
        DM_PAPERWIDTH,
        DM_SCALE,
        DM_COPIES,
        DM_DEFAULTSOURCE,
        DM_PRINTQUALITY,
        DM_COLOR,
        DM_DUPLEX,
        DM_YRESOLUTION,
        DM_TTOPTION,
        DM_COLLATE,
    ];
    let mut off = 8;
    for bit in TAIL_MEMBERS {
        if dm_fields & bit == 0 {
            continue;
        }
        match bit {
            DM_DEFAULTSOURCE => dm.source = u16be(off).map(crate::model::PaperSource::from_code),
            DM_DUPLEX => dm.duplex = u16be(off).map(crate::model::PrinterDuplex::from_code),
            _ => {}
        }
        off += 2;
    }
    dm
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
