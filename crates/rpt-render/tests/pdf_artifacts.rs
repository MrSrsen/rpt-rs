//! L4b — **PDF artifact** checks: properties of the finished file, not a diff of it.
//!
//! Where this sits. L4a (`pdf_baselines.rs`) pins the writer's operator stream against a committed
//! golden, so "did the output change" is already answered. What a golden cannot answer is whether the
//! bytes around those operators form a document: a page dropped from the page tree, a font whose width
//! table disagrees with the advances the text was laid out at, an image re-encoded on the way in, a
//! broken xref. Every operator can be individually correct and the file still be wrong. So nothing
//! here compares against a baseline — each test asserts a *relationship*, either between the artifact
//! and the Page IR it came from, or between the artifact and a real PDF reader.
//!
//! Three assertion classes, in three tests kept separate on purpose:
//!
//! 1. [`pdf_structure_matches_the_page_ir`] — page count equals the Page IR's, one font object per
//!    resolved face carrying a width table and an embedded font program, every shown glyph actually
//!    priced, and one image XObject per placed asset with the filter that asset's format implies.
//! 2. [`declared_widths_agree_with_the_laid_out_advances`] — the highest-value check. The engine's text
//!    geometry is `hmtx` advances scaled to the 1000-unit em, written into the font's width table, and
//!    a reader positions every following glyph from it. So the pen advance a reader computes
//!    (declared widths, less the `TJ` adjustments the writer emitted) must equal the advance the
//!    layout engine measured and placed the run by. When it does not, the symptom is a page of
//!    displaced glyphs; this names the cause.
//! 3. [`qpdf_parses_every_rendered_artifact`] — it opens. A real reader must accept the file. Kept in
//!    its own test so a machine without the tool cannot mask the two in-Rust classes above.
//!
//! **PDF/A and PDF/UA are deliberately not asserted here.** These fixtures render with no validator
//! set, so a conformance assertion over them would be a test of nothing. The conformance levels are
//! opt-in (`PdfOptions::conformance`) and are covered where they are requested — `rpt-render-pdf`'s
//! own tests and the semantics suite. Those read back the claim our own writer emitted (the
//! `pdfaid` keys in the XMP packet, the tag tree, the output intent), so they are a
//! **self-declaration** check, not conformance. No validator runs in this tree; veraPDF sweeps are
//! run by hand and their results recorded in `docs/rendering/08-cli.md`.
//!
//! **Hermetic by construction.** Every fixture renders from its own embedded saved data, so this
//! harness needs no database and runs on every checkout. Layout metrics and embedding are both pinned
//! to the bundled faces — a check that the declared widths match the laid-out advances is meaningless
//! if the two halves are free to resolve different faces on different hosts.

// Font-accurate advances only mean something against the cosmic-text stack.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rpt_data::RowSource;
use rpt_pages::{
    DrawOp, ImageAsset, ImageFit, ImageOp, Page, PageSize, PagedDocument, TextAlign, TextRun,
    TWIPS_PER_PT,
};
use rpt_reader::model::{Rect, Twips};
use rpt_render::{
    render_backend, render_dataset_with, CosmicLayout, DateTimeSpecials, FontProvider, FontSource,
    Locale, PdfBackend, PdfOptions,
};
use rpt_test_support::{pdf::structure, workspace_root};

/// The frozen "now" every fixture renders against — the same instant the L2/L3/L4a harnesses use.
const AS_OF_UNIX: i64 = 1_700_000_000;

/// The reports this layer covers, and the artifact property each one buys.
///
/// Three, deliberately: this layer is the most expensive to maintain and the least specific about what
/// broke, and it exists only to catch the class of defect the lower layers cannot see. A fourth report
/// would re-pay for coverage these three already have.
const FIXTURES: &[&str] = &[
    // Twelve requested families over one page, several of which fall back — so the PDF carries more
    // than one embedded face and "one font object per resolved face" is a real claim rather than a
    // tautology. It is also where a width table belonging to the wrong face would show up, since the
    // faces here have visibly different advances.
    "typography/font_faces",
    // An image XObject placed from a real embedded picture, plus saved rows formatted into text: the
    // only fixture here that exercises the asset path end to end (Page IR asset → XObject → filter).
    "worrall/USStatesWithAbbreviations",
    // Four pages of grouped data. The page-count cross-check is the point: a writer that dropped or
    // duplicated a page still emits a well-formed content stream for the pages it did write, so L4a's
    // per-page listing would compare clean against a baseline blessed from the same mistake.
    "benbrahim777/USA Orders, Percentages",
];

/// The PDF stream filter an asset of `media_type` must arrive as.
///
/// JPEG is passed through: its own DCT data becomes the stream, so the PDF carries the original
/// entropy-coded bytes rather than a re-encode. Everything else is decoded to samples and deflated,
/// which is the only faithful option for a format PDF has no native filter for.
fn expected_filter(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "DCTDecode",
        _ => "FlateDecode",
    }
}

/// Render `path` hermetically: its own saved data (or an empty source), the bundled faces on both the
/// layout and the embedding side, and the frozen as-of instant. Returns the Page IR alongside the PDF
/// so the two can be cross-checked.
fn render_fixture(path: &Path) -> (PagedDocument, Vec<u8>) {
    let rpt = rpt_reader::Rpt::open(path).expect("open fixture report");
    let report = rpt.report();

    let saved_holder;
    let source: &dyn RowSource = match &report.saved_data {
        Some(saved) => {
            saved_holder = rpt_data::SavedDataSource::from_report(saved, report);
            &saved_holder
        }
        None => &rpt_data::EmptySource,
    };
    let as_of = DateTimeSpecials::from_unix_seconds(AS_OF_UNIX);
    let dataset = rpt_data::build_dataset_opts(
        source,
        &report.data_definition,
        rpt_data::DatasetOptions {
            datetime: Some(as_of),
            ..Default::default()
        },
    );
    let doc = render_dataset_with(
        report,
        &dataset,
        Box::new(CosmicLayout::new(FontProvider::bundled())),
        Locale::default(),
        None,
        Some(as_of),
    );
    let pdf = render_backend(
        &doc,
        &PdfBackend,
        &PdfOptions {
            fonts: FontSource::Bundled,
            ..Default::default()
        },
    );
    assert!(pdf.starts_with(b"%PDF"), "{}: not a PDF", path.display());
    (doc, pdf)
}

/// Resolve a fixture's `<group>/<name>` key to the committed report, or `None` when this checkout does
/// not carry it.
fn report_path(root: &Path, rel: &str) -> Option<PathBuf> {
    let path = root
        .join("tests/fixtures/reports")
        .join(format!("{rel}.rpt"));
    path.is_file().then_some(path)
}

/// One rendered fixture: its key, the Page IR, and the PDF bytes produced from it.
struct Rendered {
    rel: &'static str,
    doc: PagedDocument,
    pdf: Vec<u8>,
}

/// Every present fixture, rendered once for the whole harness. The three assertion classes are
/// separate tests but share one render — a report costs the same to render three times as once, and
/// nothing here mutates it.
fn rendered() -> &'static [Rendered] {
    static RENDERED: OnceLock<Vec<Rendered>> = OnceLock::new();
    RENDERED.get_or_init(|| {
        let root = workspace_root();
        let mut out = Vec::new();
        for rel in FIXTURES {
            let Some(path) = report_path(&root, rel) else {
                eprintln!("SKIP {rel}: not present on this checkout");
                continue;
            };
            let (doc, pdf) = render_fixture(&path);
            out.push(Rendered { rel, doc, pdf });
        }
        out
    })
}

#[test]
fn pdf_structure_matches_the_page_ir() {
    let rendered = rendered();
    let mut failures = Vec::new();

    for Rendered { rel, doc, pdf } in rendered {
        let pdf = structure(pdf);
        // The page tree, against the pages the layout engine actually produced. A dropped or
        // duplicated page is invisible in a per-page operator diff.
        if doc.pages.len() != pdf.pages.len() {
            failures.push(format!(
                "{rel}: the PDF has {} page(s), the Page IR {}",
                pdf.pages.len(),
                doc.pages.len()
            ));
        }

        // One font object per resolved face. Two objects under one `/BaseFont` means the same subset
        // was embedded twice — every page of that face pays for it, and a reader loads it twice.
        for (base_font, objects) in pdf.faces() {
            if objects.len() > 1 {
                failures.push(format!(
                    "{rel}: face {base_font} is embedded as {} font objects ({objects:?})",
                    objects.len()
                ));
            }
        }

        for (page_no, page) in pdf.pages.iter().enumerate() {
            for (name, font) in &page.fonts {
                let at = format!("{rel} page {} /{name} ({})", page_no + 1, font.face);
                if font.widths.is_empty() {
                    failures.push(format!("{at}: declares no glyph widths"));
                }
                if !font.has_descriptor {
                    failures.push(format!("{at}: has no /FontDescriptor"));
                }
                // Without an embedded program the face has to exist on the reader's machine, which
                // is exactly the reproducibility the bundled font stack is there to remove.
                if !font.has_font_program {
                    failures.push(format!("{at}: embeds no font program"));
                }
            }
            // Every glyph the page shows must be priced by the font that shows it. A missing entry
            // is not a parse error — the reader silently falls back to `/DW`, so the run draws with
            // the wrong advances and nothing complains.
            for show in &page.shows {
                let Some(font) = page.fonts.get(&show.font) else {
                    failures.push(format!(
                        "{rel} page {}: shows text with unbound font /{}",
                        page_no + 1,
                        show.font
                    ));
                    continue;
                };
                let unpriced: Vec<u32> = show
                    .glyphs
                    .iter()
                    .copied()
                    .filter(|g| !font.widths.contains_key(g))
                    .collect();
                if !unpriced.is_empty() {
                    failures.push(format!(
                        "{rel} page {} /{}: no width entry for glyph(s) {unpriced:?} in {:?}",
                        page_no + 1,
                        show.font,
                        show.text
                    ));
                }
            }
        }

        // Images: one XObject per distinct asset placed on the page, with the filter its source
        // format implies. An asset the layout engine never resolved draws a placeholder instead, so
        // only ops with an asset are expected to produce an XObject.
        for (page_no, (ir, page)) in doc.pages.iter().zip(&pdf.pages).enumerate() {
            let placed: BTreeSet<&String> = ir
                .ops
                .iter()
                .filter_map(|op| match op {
                    DrawOp::Image(i) if doc.assets.contains_key(&i.image_id) => Some(&i.image_id),
                    _ => None,
                })
                .collect();
            let at = format!("{rel} page {}", page_no + 1);
            if placed.len() != page.images.len() {
                failures.push(format!(
                    "{at}: {} placed image(s) but {} image XObject(s)",
                    placed.len(),
                    page.images.len()
                ));
            }
            let want: BTreeSet<&str> = placed
                .iter()
                .map(|id| expected_filter(&doc.assets[*id].media_type))
                .collect();
            for (name, image) in &page.images {
                if image.subtype != "Image" || image.width <= 0 || image.height <= 0 {
                    failures.push(format!("{at} /{name}: not a sized image XObject ({image})"));
                }
                if !want.contains(image.filter.as_str()) {
                    failures.push(format!(
                        "{at} /{name}: filter /{} is not one of the expected {want:?}",
                        image.filter
                    ));
                }
            }
        }
    }

    eprintln!(
        "pdf artifacts: {} of {} fixture(s) checked",
        rendered.len(),
        FIXTURES.len()
    );
    assert_eq!(
        rendered.len(),
        FIXTURES.len(),
        "{} of the {} committed reports listed in FIXTURES are missing, so their structure was never \
         checked",
        FIXTURES.len() - rendered.len(),
        FIXTURES.len()
    );
    assert!(
        failures.is_empty(),
        "{} PDF structure problem(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A layout-produced text run whose drawn advance must equal its measured one. Justified runs are
/// excluded: the writer deliberately spreads the box's slack across their inter-word gaps, so the
/// drawn advance is wider than the shaped one by design.
fn measured_runs(page: &Page) -> Vec<&TextRun> {
    page.ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(t)
                if t.metrics.is_some() && !t.text.is_empty() && t.align != TextAlign::Justified =>
            {
                Some(t)
            }
            _ => None,
        })
        .collect()
}

/// Runs whose declared advance is known to sit outside [`residual_band`], because layout and the PDF
/// writer shape at different granularities: cosmic-text shapes word by word, so a kern pair that
/// straddles a word boundary never fires when the run is measured, while the writer shapes the whole
/// run in one buffer and applies it. Both shape with harfrust — the shaper is not the difference.
///
/// Listed by text rather than tolerated by a wider band, deliberately: a band loose enough to admit
/// these would admit a real regression on every other run too. This way the check still fails on a
/// divergence that is new, and the list shrinking is a signal worth seeing rather than a test break.
const KNOWN_WORD_BOUNDARY_KERNS: &[&str] = &[
    "Order Amount",
    "USA Orders, Grouped by Province or Municipality",
    "New York",
    "West Virginia",
];

/// The bound on `declared − measured`, a quantization identity rather than a tolerance: the band a
/// run's declared advance may land in, relative to the advance it was laid out at. It is derived from
/// two roundings, not fitted to the data — widening either term would be widening a statement about
/// how the geometry is quantized:
///
/// * [`rpt_pages::TextMetrics::advance`] is a whole-twip integer built by *truncating* the measured
///   width, which alone puts the declared width at or above the stored value and under the next one.
/// * a `/Widths` entry is itself quantized to the font's 1000-unit em, so each glyph contributes up
///   to half an em unit of error in either direction, and over an `n`-glyph run that accumulates.
///
/// So the band is `-n·u/2 ..= 1 + n·u/2`, where `u` is one em unit in twips at the run's size. A flat
/// epsilon cannot express this: the same slack that is generous for a two-glyph abbreviation is far
/// too tight for a sixty-glyph line.
fn residual_band(glyphs: usize, em_unit: f64) -> std::ops::RangeInclusive<f64> {
    let slack = glyphs as f64 * em_unit / 2.0;
    -slack..=(1.0 + slack)
}

#[test]
fn declared_widths_agree_with_the_laid_out_advances() {
    let mut failures = Vec::new();
    let mut compared = 0usize;
    // The residual distribution, reported so a run that only just fits the identity is visible rather
    // than merely passing: the widest deviation in em units per glyph, and the range of the residual.
    let mut worst_per_glyph = (0.0f64, String::new());
    let mut residuals = (f64::MAX, f64::MIN);

    for Rendered { rel, doc, pdf } in rendered() {
        let pdf = structure(pdf);
        for (page_no, (ir, page)) in doc.pages.iter().zip(&pdf.pages).enumerate() {
            // A run drawn from more than one face (a fallback segment) is shown once per face, so
            // shows are accumulated until their decoded text completes the next run's.
            let mut shows = page.shows.iter();
            for run in measured_runs(ir) {
                let mut text = String::new();
                let mut pen_pt = 0.0f64;
                let mut glyphs = 0usize;
                while text != run.text {
                    let Some(show) = shows.next() else { break };
                    text.push_str(&show.text);
                    // The pen advance, not the raw width sum: `/Widths` declares per-glyph advances
                    // with no kerning, because kerning is applied by the `TJ` position adjustments
                    // rather than by the table. Summing the table alone therefore overstates every
                    // kerned run — `PA`, `VA`, `Te` — by exactly the kern, and comparing that to a
                    // shaped advance would be comparing two different quantities.
                    pen_pt += show.pen_pt;
                    glyphs += show.glyphs.len();
                }
                if text != run.text {
                    failures.push(format!(
                        "{rel} page {}: the content stream shows no text for run {:?}",
                        page_no + 1,
                        run.text
                    ));
                    break;
                }
                let measured = f64::from(
                    run.metrics
                        .expect("measured_runs keeps only runs with metrics")
                        .advance
                        .0,
                );
                let declared = pen_pt * TWIPS_PER_PT;
                let residual = declared - measured;
                compared += 1;
                residuals = (residuals.0.min(residual), residuals.1.max(residual));
                // One em unit at this size, in twips — the granularity a width table works in.
                let em_unit = f64::from(run.font.size_pt) * TWIPS_PER_PT / 1000.0;
                let per_glyph = residual.abs() / (glyphs.max(1) as f64 * em_unit);
                if per_glyph > worst_per_glyph.0 {
                    worst_per_glyph = (
                        per_glyph,
                        format!(
                            "{rel} page {} {:?} ({glyphs} glyphs, {}pt): residual {residual:.3} twips",
                            page_no + 1,
                            run.text,
                            run.font.size_pt,
                        ),
                    );
                }
                let known = KNOWN_WORD_BOUNDARY_KERNS.contains(&run.text.as_str());
                if !residual_band(glyphs, em_unit).contains(&residual) && !known {
                    failures.push(format!(
                        "{rel} page {}: {:?} in {} {}pt advances {declared:.3} twips by its \
                         declared widths and kerns but was laid out at {measured} twips (residual \
                         {residual:.3}, {per_glyph:.4} em units per glyph)",
                        page_no + 1,
                        run.text,
                        run.font.family,
                        run.font.size_pt,
                    ));
                }
            }
        }
    }

    eprintln!(
        "pdf artifacts: {compared} run advance(s) compared; residual range [{:.4}, {:.4}] twips; \
         widest {:.5} em units/glyph — {}",
        residuals.0, residuals.1, worst_per_glyph.0, worst_per_glyph.1
    );
    assert!(
        compared > 0,
        "no run advances were compared — the fixtures produced no measured text runs"
    );
    assert!(
        failures.is_empty(),
        "{} run(s) whose declared widths disagree with the laid-out advance:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A 2×2 baseline JPEG, made for this test. It exists because no committed report embeds a JPEG — the
/// corpus's pictures are OLE bitmaps — and the passthrough claim is worth asserting on its own rather
/// than waiting for a fixture that happens to carry one.
const TINY_JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x1b, 0x12, 0x14, 0x17, 0x14, 0x11, 0x1b,
    0x17, 0x16, 0x17, 0x1e, 0x1c, 0x1b, 0x20, 0x28, 0x42, 0x2b, 0x28, 0x25, 0x25, 0x28, 0x51, 0x3a,
    0x3d, 0x30, 0x42, 0x60, 0x55, 0x65, 0x64, 0x5f, 0x55, 0x5d, 0x5b, 0x6a, 0x78, 0x99, 0x81, 0x6a,
    0x71, 0x90, 0x73, 0x5b, 0x5d, 0x85, 0xb5, 0x86, 0x90, 0x9e, 0xa3, 0xab, 0xad, 0xab, 0x67, 0x80,
    0xbc, 0xc9, 0xba, 0xa6, 0xc7, 0x99, 0xa8, 0xab, 0xa4, 0xff, 0xdb, 0x00, 0x43, 0x01, 0x1c, 0x1e,
    0x1e, 0x28, 0x23, 0x28, 0x4e, 0x2b, 0x2b, 0x4e, 0xa4, 0x6e, 0x5d, 0x6e, 0xa4, 0xa4, 0xa4, 0xa4,
    0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4,
    0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4,
    0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4, 0xa4,
    0xa4, 0xa4, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x02, 0x00, 0x02, 0x03, 0x01, 0x22, 0x00, 0x02,
    0x11, 0x01, 0x03, 0x11, 0x01, 0xff, 0xc4, 0x00, 0x15, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0xff, 0xc4, 0x00, 0x14,
    0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xc4, 0x00, 0x15, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x05, 0xff, 0xc4, 0x00, 0x14, 0x11, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff,
    0xda, 0x00, 0x0c, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00, 0x9c, 0x01, 0x55,
    0x5f, 0xff, 0xd9,
];

#[test]
fn a_jpeg_asset_is_passed_through_as_dctdecode() {
    let mut page = Page::new(
        1,
        PageSize {
            width: Twips(12240),
            height: Twips(15840),
        },
    );
    page.push(DrawOp::Image(ImageOp {
        bounds: Rect {
            left: Twips(720),
            top: Twips(720),
            width: Twips(1440),
            height: Twips(1440),
        },
        image_id: "jpeg".to_string(),
        fit: ImageFit::Fill,
        source: None,
    }));
    let doc = PagedDocument {
        pages: vec![page],
        assets: [(
            "jpeg".to_string(),
            ImageAsset {
                media_type: "image/jpeg".to_string(),
                bytes: TINY_JPEG.to_vec(),
            },
        )]
        .into_iter()
        .collect(),
        ..PagedDocument::default()
    };

    let pdf = render_backend(
        &doc,
        &PdfBackend,
        &PdfOptions {
            fonts: FontSource::Bundled,
            ..Default::default()
        },
    );
    let images = &structure(&pdf).pages[0].images;
    assert_eq!(images.len(), 1, "expected one image XObject: {images:?}");
    let image = images.values().next().expect("one image");
    assert_eq!(
        image.filter,
        expected_filter("image/jpeg"),
        "a JPEG asset must reach the PDF as its own DCT data, not as a re-encode: {image}"
    );
    assert_eq!((image.width, image.height), (2, 2), "{image}");
}

/// The external reader used for the "it opens" check, and why it is the strongest single choice: it
/// validates the xref table and the object structure, and reports objects nothing references — which
/// is the same list a hand-written structural check would have to reimplement.
const READER: &str = "qpdf";

/// Set this in an environment that is supposed to have [`READER`] to turn its absence from a loud
/// skip into a failure. Without it a checkout that simply lacks the tool reports green.
///
/// CI installs [`READER`] and sets this in the `test` job, so a missing reader there is a failure
/// rather than a silent pass. A local checkout without the tool still skips loudly.
const REQUIRE_READER: &str = "RPT_REQUIRE_QPDF";

#[test]
fn qpdf_parses_every_rendered_artifact() {
    let available = std::process::Command::new(READER)
        .arg("--version")
        .output()
        .is_ok();
    if !available {
        let message = format!(
            "SKIP: {READER} is not on PATH, so nothing verified that these PDFs parse.\n  \
             {READER} --check validates the xref table, the object structure and unreferenced \
             objects — the one thing this layer cannot assert from inside Rust.\n  \
             Install it (apt install {READER}) or set {REQUIRE_READER}=1 to make this a failure."
        );
        eprintln!("{message}");
        assert!(
            std::env::var_os(REQUIRE_READER).is_none(),
            "{REQUIRE_READER} is set but {message}"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!("rpt-l4b-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for Rendered { rel, doc, pdf } in rendered() {
        let out = dir.join(format!("{}.pdf", rel.replace('/', "_")));
        std::fs::write(&out, pdf).expect("write rendered PDF");
        let result = std::process::Command::new(READER)
            .arg("--check")
            .arg(&out)
            .output()
            .expect("run qpdf --check");
        checked += 1;
        if !result.status.success() {
            failures.push(format!(
                "{rel}: {READER} --check failed ({})\n{}{}",
                result.status,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr),
            ));
        }
        // qpdf reads the page tree independently of our own parser, so this is a second opinion on
        // the page count the structural test cross-checked against the Page IR.
        let pages = std::process::Command::new(READER)
            .args(["--show-npages", &out.to_string_lossy()])
            .output()
            .expect("run qpdf --show-npages");
        let reported: usize = String::from_utf8_lossy(&pages.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        if reported != doc.pages.len() {
            failures.push(format!(
                "{rel}: {READER} reports {reported} page(s), the Page IR has {}",
                doc.pages.len()
            ));
        }
        std::fs::remove_file(&out).ok();
    }
    std::fs::remove_dir_all(&dir).ok();

    eprintln!("pdf artifacts: {checked} artifact(s) parsed by {READER}");
    assert!(checked > 0, "no artifacts were handed to {READER}");
    assert!(
        failures.is_empty(),
        "{} artifact(s) a real reader rejected:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
