//! The basic PDF writer (fallback): dependency-free minimal PDF 1.4, Helvetica base-14, uncompressed
//! streams. Always available; used when the `krilla-backend` feature is disabled (or if krilla fails
//! to serialize a document). Raw PDF is y-up (origin bottom-left), so this writer flips y.

use crate::common::{
    aligned_x, approx_text_width, baseline_offset_twips, chan, pt, solid_of, KAPPA, TWIPS_PER_PT,
};
use rpt_model::Color;
use rpt_pages::{DrawOp, EllipseOp, Fill, LineOp, Page, PolygonOp, RectOp, TextRun};
use std::fmt::Write as _;

/// Render pages with the self-contained, dependency-free writer (no embedded fonts, no compression).
///
/// This is the zero-dependency fallback used when the `krilla-backend` feature is disabled;
/// [`crate::render_pages`] prefers the krilla backend for real font embedding.
pub fn render_pages_basic(pages: &[Page]) -> Vec<u8> {
    let mut pdf = BasicWriter::new();
    pdf.write(pages)
}

struct BasicWriter {
    out: Vec<u8>,
    offsets: Vec<usize>,
}

impl BasicWriter {
    fn new() -> BasicWriter {
        BasicWriter {
            out: Vec::new(),
            offsets: Vec::new(),
        }
    }

    /// Reserve object number `n` (1-based) and record the current byte offset for the xref table.
    fn begin_obj(&mut self, n: usize) {
        while self.offsets.len() < n {
            self.offsets.push(0);
        }
        self.offsets[n - 1] = self.out.len();
        self.push(&format!("{n} 0 obj\n"));
    }

    fn push(&mut self, s: &str) {
        self.out.extend_from_slice(s.as_bytes());
    }

    fn write(&mut self, pages: &[Page]) -> Vec<u8> {
        self.push("%PDF-1.4\n%\u{00e2}\u{00e3}\u{00cf}\u{00d3}\n");

        // Object layout: 1=Catalog, 2=Pages, 3=Font, then per page: a Page obj + a Contents obj.
        let n_pages = pages.len().max(1);
        let page_obj = |i: usize| 4 + i * 2; // Page object number for page i
        let content_obj = |i: usize| 5 + i * 2; // its Contents stream

        // 1: Catalog
        self.begin_obj(1);
        self.push("<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        // 2: Pages
        self.begin_obj(2);
        let kids: String = (0..n_pages)
            .map(|i| format!("{} 0 R", page_obj(i)))
            .collect::<Vec<_>>()
            .join(" ");
        self.push(&format!(
            "<< /Type /Pages /Count {n_pages} /Kids [{kids}] >>\nendobj\n"
        ));

        // 3: Font (Helvetica, base-14 — no embedding needed).
        self.begin_obj(3);
        self.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n");

        for (i, page) in pages.iter().enumerate() {
            let w = pt(page.size.width.0);
            let h = pt(page.size.height.0);
            let content = build_content(page, h);

            // Page object.
            self.begin_obj(page_obj(i));
            self.push(&format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] \
                 /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>\nendobj\n",
                content_obj(i)
            ));

            // Contents stream.
            self.begin_obj(content_obj(i));
            self.push(&format!(
                "<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n",
                content.len() + 1
            ));
        }

        // xref + trailer.
        let xref_pos = self.out.len();
        let offsets = self.offsets.clone();
        let n_objs = offsets.len();
        self.push(&format!("xref\n0 {}\n", n_objs + 1));
        self.push("0000000000 65535 f \n");
        for off in &offsets {
            self.push(&format!("{off:010} 00000 n \n"));
        }
        self.push(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            n_objs + 1
        ));

        std::mem::take(&mut self.out)
    }
}

/// Build one page's content stream (PDF operators).
fn build_content(page: &Page, page_h_pt: f64) -> String {
    let mut c = String::new();
    // Draw-op coordinates are printable-relative (0-based); translate the coordinate space by the
    // page origin so content sits inside the physical margins. PDF y is up, so a downward shift of
    // `oy` is `ty = -oy`.
    let (ox, oy) = (pt(page.origin.x.0), pt(page.origin.y.0));
    let shifted = ox != 0.0 || oy != 0.0;
    if shifted {
        let _ = writeln!(c, "q 1 0 0 1 {ox:.2} {:.2} cm", -oy);
    }
    for op in &page.ops {
        match op {
            DrawOp::Rect(r) => rect_ops(&mut c, r, page_h_pt),
            DrawOp::Ellipse(e) => ellipse_ops(&mut c, e, page_h_pt),
            DrawOp::Line(l) => line_ops(&mut c, l, page_h_pt),
            DrawOp::Polygon(p) => polygon_ops(&mut c, p, page_h_pt),
            DrawOp::Text(t) => text_ops(&mut c, t, page_h_pt),
            DrawOp::Image(_) => {} // images out of scope for this backend
        }
    }
    if shifted {
        c.push_str("Q\n");
    }
    c
}

fn rect_ops(c: &mut String, r: &RectOp, h: f64) {
    let x = pt(r.bounds.left.0);
    let y = h - pt(r.bounds.top.0 + r.bounds.height.0);
    let w = pt(r.bounds.width.0);
    let ht = pt(r.bounds.height.0);
    let _ = writeln!(c, "{x:.2} {y:.2} {w:.2} {ht:.2} re");
    paint_ops(c, r.fill.as_ref(), r.stroke.as_ref());
}

/// Emit the paint operator (`B`/`f`/`S`/`n`) after a path, given its optional fill and stroke.
/// A gradient/hatch fill is painted as its representative solid colour (see [`solid_of`]).
fn paint_ops(c: &mut String, fill: Option<&Fill>, stroke: Option<&rpt_pages::Stroke>) {
    match (fill, stroke) {
        (Some(fill), Some(s)) => {
            set_fill(c, solid_of(fill));
            set_stroke_color(c, s.color);
            set_line_width(c, pt(s.width.0));
            c.push_str("B\n"); // fill + stroke
        }
        (Some(fill), None) => {
            set_fill(c, solid_of(fill));
            c.push_str("f\n");
        }
        (None, Some(s)) => {
            set_stroke_color(c, s.color);
            set_line_width(c, pt(s.width.0));
            c.push_str("S\n");
        }
        (None, None) => c.push_str("n\n"),
    }
}

/// An ellipse inscribed in the op's bounds, built from four cubic-Bézier quarter arcs (`c`), then
/// painted. y is flipped for PDF's bottom-left origin.
fn ellipse_ops(c: &mut String, e: &EllipseOp, h: f64) {
    if e.bounds.width.0 <= 0 || e.bounds.height.0 <= 0 {
        return;
    }
    let cx = pt(e.bounds.left.0) + pt(e.bounds.width.0) / 2.0;
    let cy = h - (pt(e.bounds.top.0) + pt(e.bounds.height.0) / 2.0);
    let rx = pt(e.bounds.width.0) / 2.0;
    let ry = pt(e.bounds.height.0) / 2.0;
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let _ = writeln!(c, "{:.2} {:.2} m", cx + rx, cy);
    let _ = writeln!(
        c,
        "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c",
        cx + rx,
        cy + ky,
        cx + kx,
        cy + ry,
        cx,
        cy + ry
    );
    let _ = writeln!(
        c,
        "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c",
        cx - kx,
        cy + ry,
        cx - rx,
        cy + ky,
        cx - rx,
        cy
    );
    let _ = writeln!(
        c,
        "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c",
        cx - rx,
        cy - ky,
        cx - kx,
        cy - ry,
        cx,
        cy - ry
    );
    let _ = writeln!(
        c,
        "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c",
        cx + kx,
        cy - ry,
        cx + rx,
        cy - ky,
        cx + rx,
        cy
    );
    c.push_str("h\n");
    paint_ops(c, e.fill.as_ref(), e.stroke.as_ref());
}

fn line_ops(c: &mut String, l: &LineOp, h: f64) {
    set_stroke_color(c, l.stroke.color);
    set_line_width(c, pt(l.stroke.width.0));
    let _ = writeln!(
        c,
        "{:.2} {:.2} m {:.2} {:.2} l S",
        pt(l.from.x.0),
        h - pt(l.from.y.0),
        pt(l.to.x.0),
        h - pt(l.to.y.0),
    );
}

fn polygon_ops(c: &mut String, p: &PolygonOp, h: f64) {
    if p.points.len() < 2 {
        return;
    }
    // Build the path (y-flipped for PDF's bottom-left origin).
    let _ = writeln!(
        c,
        "{:.2} {:.2} m",
        pt(p.points[0].x.0),
        h - pt(p.points[0].y.0)
    );
    for pt_ in &p.points[1..] {
        let _ = writeln!(c, "{:.2} {:.2} l", pt(pt_.x.0), h - pt(pt_.y.0));
    }
    if p.closed {
        c.push_str("h\n"); // close the subpath
    }
    // Paint operator: fill+stroke / fill / stroke / no-op (open polylines never fill).
    let fill = if p.closed { p.fill.as_ref() } else { None };
    paint_ops(c, fill, p.stroke.as_ref());
}

fn text_ops(c: &mut String, t: &TextRun, h: f64) {
    if t.text.is_empty() {
        return;
    }
    let size = t.font.size_pt as f64;
    // Baseline: top + ascent, then flip to PDF's y-up. Use the run's resolved ascent when present,
    // else the ~0.8-em heuristic.
    let baseline_twips = t.bounds.top.0 as f64 + baseline_offset_twips(t);
    let y = h - baseline_twips / TWIPS_PER_PT;
    // Horizontal alignment: shift x by the run's stored advance (else the approximate width) for
    // center/right.
    let text_w = match &t.metrics {
        Some(m) => pt(m.advance.0),
        None => approx_text_width(&t.text, size),
    };
    let x = aligned_x(t.align, pt(t.bounds.left.0), pt(t.bounds.width.0), text_w);
    set_fill(c, t.color);
    // Rotation is CCW degrees about the run's origin (top-left of bounds). PDF is y-up, so a positive
    // angle rotates CCW visually; wrap the text in a `cm` that rotates about the pivot. `0.0` emits the
    // plain single-line output unchanged.
    let rotated = t.rotation != 0.0;
    if rotated {
        let (pvx, pvy) = (pt(t.bounds.left.0), h - pt(t.bounds.top.0));
        let rad = (t.rotation as f64).to_radians();
        let (cos, sin) = (rad.cos(), rad.sin());
        let e = pvx - pvx * cos + pvy * sin;
        let f = pvy - pvx * sin - pvy * cos;
        let _ = writeln!(
            c,
            "q {cos:.5} {sin:.5} {:.5} {cos:.5} {e:.2} {f:.2} cm",
            -sin
        );
    }
    let _ = writeln!(
        c,
        "BT /F1 {size:.2} Tf {x:.2} {y:.2} Td ({}) Tj ET",
        escape_pdf_text(&t.text)
    );
    if rotated {
        c.push_str("Q\n");
    }
}

fn set_fill(c: &mut String, color: Color) {
    let _ = writeln!(
        c,
        "{:.3} {:.3} {:.3} rg",
        chan(color.r),
        chan(color.g),
        chan(color.b)
    );
}

fn set_stroke_color(c: &mut String, color: Color) {
    let _ = writeln!(
        c,
        "{:.3} {:.3} {:.3} RG",
        chan(color.r),
        chan(color.g),
        chan(color.b)
    );
}

fn set_line_width(c: &mut String, w: f64) {
    let _ = writeln!(c, "{:.2} w", w.max(0.25));
}

/// Escape a string for a PDF literal `(...)`. Non-ASCII collapses to `?` (WinAnsi/embedding is a
/// follow-up); `(`, `)`, `\` are backslash-escaped.
fn escape_pdf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            c if c.is_ascii() && !c.is_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}
