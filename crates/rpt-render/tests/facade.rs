//! The SDK-shaped `ReportDocument` facade — runs on a committed public-demo fixture.
use std::path::PathBuf;

#[test]
fn report_document_facade_loads_and_exports() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    // SDK-familiar shape: one object loads + holds model + exports.
    let doc = rpt_render::ReportDocument::load(&path).expect("load");
    assert!(!doc.report().data_definition.field_definitions.is_empty());

    // Field accessor views work through the facade's model.
    let n_formulas = doc.report().data_definition.formula_fields().count();
    assert!(n_formulas >= 1, "worrall has @CCTLD_formatted");

    // Export surface delegates to the render crates.
    let pdf = doc.to_pdf();
    // Any PDF version header (the krilla backend emits a newer version than the basic writer's 1.4).
    assert!(pdf.starts_with(b"%PDF-"), "PDF header");
    let html = doc.to_html();
    // The HTML backend emits the engine's RAS-direct frame (XHTML 1.0 Transitional doctype +
    // `crystalstyle` containers), not a bare `<!doctype html>`.
    assert!(html.contains("<!DOCTYPE html"), "XHTML doctype");
    assert!(
        html.contains("crystalstyle"),
        "reportrenderer page container"
    );
    assert_eq!(doc.export_svg_pages().len(), doc.render().pages.len());
}

/// The `PageBackend` seam must produce byte-identical output to the concrete free functions it
/// delegates to — it is an additive dispatch layer, not a new code path.
#[test]
fn render_backend_seam_matches_free_functions() {
    use rpt::model::{Color, Rect, Twips};
    use rpt_pages::{
        DrawOp, FontSpec, ObjectKind, ObjectRef, Page, PageSize, PagedDocument, TextAlign, TextRun,
    };

    let mut page = Page::new(
        1,
        PageSize {
            width: Twips(12240),
            height: Twips(15840),
        },
    );
    page.push(DrawOp::Text(TextRun {
        bounds: Rect {
            left: Twips(200),
            top: Twips(200),
            width: Twips(2000),
            height: Twips(300),
        },
        text: "Backend seam".into(),
        font: FontSpec::default(),
        color: Color::default(),
        align: TextAlign::Left,
        rotation: 0.0,
        metrics: None,
        source: Some(ObjectRef::new("Details", ObjectKind::Field).named("f")),
    }));
    let doc = PagedDocument {
        pages: vec![page],
        ..Default::default()
    };

    // HTML
    assert_eq!(
        rpt_render::render_backend(&doc, &rpt_render::HtmlBackend, &rpt_render::HtmlOptions),
        rpt_render_html::render_pages_with_assets(&doc.pages, &doc.assets),
    );
    // SVG (one string per page)
    assert_eq!(
        rpt_render::render_backend(&doc, &rpt_render::SvgBackend, &()),
        doc.pages
            .iter()
            .map(rpt_render_svg::render_page)
            .collect::<Vec<_>>(),
    );
    // PDF: Auto == render_pages, Basic == render_pages_basic.
    assert_eq!(
        rpt_render::render_backend(
            &doc,
            &rpt_render::PdfBackend,
            &rpt_render::PdfOptions::default()
        ),
        rpt_render_pdf::render_pages(&doc.pages),
    );
    assert_eq!(
        rpt_render::render_backend(
            &doc,
            &rpt_render::PdfBackend,
            &rpt_render::PdfOptions {
                writer: rpt_render::PdfWriter::Basic,
            },
        ),
        rpt_render_pdf::render_pages_basic(&doc.pages),
    );
    // Raster (one PNG per page at default DPI)
    assert_eq!(
        rpt_render::render_backend(
            &doc,
            &rpt_render::RasterBackend,
            &rpt_render::RasterOptions::default(),
        ),
        rpt_render_raster::render_pages(&doc.pages),
    );
}
