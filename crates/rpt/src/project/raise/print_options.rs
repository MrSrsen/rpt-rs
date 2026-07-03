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

    // Every record of a type, located anywhere in the tree (more than one record can share a type).
    let all = |ty: u16| -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for r in tree {
            r.walk(&mut |n| {
                if n.rtype == ty {
                    out.push(n.leaf_bytes(logical));
                }
            });
        }
        out
    };
    let be32 = |b: &[u8], off: usize| -> Option<i32> {
        b.get(off..off + 4)
            .map(|s| i32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    };
    // A plausible page dimension (positive twips). The upper bound is generous — custom/driver
    // "paper" can be very wide (e.g. a 150-inch data-export page) — and only guards against a
    // mis-read leaf; the dedicated `0x18e` record makes a false positive unlikely.
    let sane = |v: Option<i32>| v.filter(|&x| (1..=1_000_000).contains(&x));

    // The page-setup record (0x66): the four margins as big-endian u32 twips — Left, Right, Top,
    // Bottom, after a 3-byte header. A margin stored as i32::MIN (`0x80000000`) is the engine's
    // "use default" sentinel and resolves to 360 twips (¼ inch, Crystal's default).
    if let Some(b) = all(PAGE_SETUP).into_iter().next() {
        let margin = |off: usize| be32(&b, off).map(|v| Twips(if v == i32::MIN { 360 } else { v }));
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

    // The page DEVMODE (0x07): a Crystal big-endian variant of a Windows DEVMODE. A fixed 8-byte
    // header (sub-type[0..2], `dmFields` bitfield[2..4], orientation[4..6], paper size[6..8]) is
    // followed by a variable section: each Windows `dmFields` bit that is set contributes one
    // big-endian u16, in bit order — so the paper source (bit 9) and duplex (bit 12) sit at offsets
    // that shift with the earlier present fields.
    if let Some(b) = all(PAPER_DEVMODE).into_iter().next() {
        let u16be = |off: usize| {
            b.get(off..off + 2)
                .map(|s| i32::from(u16::from_be_bytes([s[0], s[1]])))
        };
        if let Some(c) = u16be(4) {
            opts.paper_orientation = crate::model::PaperOrientation::from_code(c);
        }
        if let Some(c) = u16be(6) {
            opts.paper_size = crate::model::PaperSize::from_code(c);
        }
        let dm_fields = b.get(2..4).map_or(0, |s| u16::from_be_bytes([s[0], s[1]]));
        let mut off = 8;
        // Conditional fields up to the source (bit 9): paper length/width (2/3), scale (4),
        // copies (8). Each present field is one u16.
        for bit in [0x0004u16, 0x0008, 0x0010, 0x0100] {
            if dm_fields & bit != 0 {
                off += 2;
            }
        }
        if dm_fields & 0x0200 != 0 {
            if let Some(c) = u16be(off) {
                opts.paper_source = crate::model::PaperSource::from_code(c);
            }
            off += 2;
        }
        // Between source and duplex (bit 12): print quality (10) and color (11).
        for bit in [0x0400u16, 0x0800] {
            if dm_fields & bit != 0 {
                off += 2;
            }
        }
        if dm_fields & 0x1000 != 0 {
            if let Some(c) = u16be(off) {
                opts.printer_duplex = crate::model::PrinterDuplex::from_code(c);
            }
        }
    }

    // The page rectangle (0x18e): the paper width then height as big-endian u32 twips.
    // PageContentWidth/Height are the printable area — the paper dimensions less the margins. The
    // rect is stored in either edge order; when it is a *standard* sheet (its sorted edges match the
    // `PaperSize`), the dimensions are oriented to `PaperOrientation` — so a standard sheet saved in
    // the wrong order (e.g. a Legal sheet stored landscape but flagged Portrait) is re-oriented here.
    // A rect that is NOT a standard sheet is a genuine custom page and is kept exactly as stored.
    if let Some(b) = all(PAPER_RECT).into_iter().next() {
        if let (Some(pw), Some(ph)) = (sane(be32(&b, 0)), sane(be32(&b, 4))) {
            let (mut paper_w, mut paper_h) = (pw, ph);
            if let Some((short, long)) = opts.paper_size.std_dims() {
                // Recognise the stored rect as this standard sheet (edge order ignored, small
                // tolerance for mm rounding), then lay it out per the orientation.
                let (lo, hi) = (pw.min(ph), pw.max(ph));
                let matches = (lo - short).abs() <= 30 && (hi - long).abs() <= 30;
                if matches {
                    let landscape =
                        opts.paper_orientation == crate::model::PaperOrientation::Landscape;
                    (paper_w, paper_h) = if landscape { (hi, lo) } else { (lo, hi) };
                }
            }
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
    if let Some(node) = tree.iter().find(|n| n.rtype == PRINTER) {
        let strings = all_strings(node, logical);
        // Order: driver ("winspool"), printer name, port. The printer name is emitted empty (the
        // saved printer is not resolved in the reader's environment), and neither driver nor port is
        // emitted, so these decoded values are kept out of the export.
        opts.driver_name = strings.first().cloned();
        opts.printer_name = String::new();
        opts.port_name = strings.get(2).cloned();
    }
    opts
}
