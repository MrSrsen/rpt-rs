//! The SDK-shaped `ReportDocument` facade — runs on a committed public-demo fixture.

#[test]
fn report_document_facade_loads_and_exports() {
    let path = rpt_test_support::fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
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
    assert!(
        !doc.render().pages.is_empty(),
        "paginated at least one page"
    );
}

// The write is the only fallible step of an export, and the error exists to name the file: a bare
// `std::io::Error` would say "No such file or directory" without saying about what.
#[test]
fn exporting_to_disk_writes_a_pdf_and_names_the_path_it_cannot_write() {
    let path = rpt_test_support::fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let doc = rpt_render::ReportDocument::load(&path).expect("load");

    let out = std::env::temp_dir().join(format!("rpt-render-facade-{}.pdf", std::process::id()));
    doc.export_pdf_to_disk(&out).expect("write the export");
    assert!(std::fs::read(&out)
        .expect("read back")
        .starts_with(b"%PDF-"));
    let _ = std::fs::remove_file(&out);

    let err = doc
        .export_pdf_to_disk(out.join("no-such-directory/out.pdf"))
        .expect_err("a path under a missing directory cannot be written");
    assert!(err.to_string().contains("no-such-directory"), "{err}");
    assert!(
        std::error::Error::source(&err).is_some(),
        "the io::Error is kept as the source"
    );
}

/// Assert every face `pdf` embeds is one of the compiled-in Liberation/DejaVu set. krilla writes each
/// one's name in plaintext (`/BaseFont/SHYXUL+LiberationSans`), so which face the writer actually
/// resolved is directly observable in the bytes. The whitespace between a key and its name value is
/// optional in PDF, so the name may be butted straight against the key.
fn assert_only_bundled_faces(pdf: &[u8], what: &str) {
    let text = String::from_utf8_lossy(pdf);
    let faces: Vec<&str> = text
        .split("/BaseFont")
        .skip(1)
        .filter_map(|frag| frag.trim_start().strip_prefix('/'))
        .map(|name| {
            let end = name
                .find(|c: char| c.is_whitespace() || "/<>[]()".contains(c))
                .unwrap_or(name.len());
            &name[..end]
        })
        .collect();
    assert!(!faces.is_empty(), "{what} must embed a face");
    assert!(
        faces
            .iter()
            .all(|f| f.contains("Liberation") || f.contains("DejaVu")),
        "{what} must embed only bundled faces, got {faces:?}"
    );
}

/// The zero-config render is reproducible: neither half of the font stack reads the host's installed
/// library unless asked. The host library stays reachable — the same two fields set the other way —
/// which is what makes this a default rather than a removed capability.
#[test]
fn the_default_render_reads_no_host_fonts() {
    use rpt_render::{FontSource, PdfBackend, PdfOptions, RenderOptions};

    assert_eq!(RenderOptions::default().fonts, FontSource::Bundled);
    assert_eq!(PdfOptions::default().fonts, FontSource::Bundled);

    let path = rpt_test_support::fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let doc = rpt_render::ReportDocument::load(&path).expect("load");
    assert_only_bundled_faces(&doc.to_pdf(), "the default render");

    // Both halves opt back in from the facade: layout metrics via `RenderOptions::fonts`, embedded
    // faces via `PdfOptions::fonts`. Which faces this host has is not asserted — that the opt-in is
    // reachable and renders is.
    let paged = rpt_render::render_with(
        doc.report(),
        RenderOptions {
            fonts: FontSource::System,
            ..Default::default()
        },
    );
    let pdf = rpt_render::render_backend(
        &paged,
        &PdfBackend,
        &PdfOptions {
            fonts: FontSource::System,
            ..Default::default()
        },
    );
    assert!(pdf.starts_with(b"%PDF-"));
}

/// A PDF whose bytes are a property of the report alone — not of the host's font library — needs both
/// halves of the font stack injected: the layout's metrics (a bundled `CosmicLayout` via
/// `render_dataset_with`) and the faces the backend embeds (`PdfOptions::fonts`). Both are reachable
/// from the facade, which is what lets a committed PDF baseline be blessed on one machine and checked
/// on another.
#[test]
fn a_hermetic_pdf_is_reachable_through_the_facade() {
    use rpt_render::{
        CosmicLayout, DateTimeSpecials, FontProvider, FontSource, Locale, PdfBackend, PdfOptions,
    };

    let path = rpt_test_support::fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open");
    let report = rpt.report();
    let dataset = rpt_data::build_dataset(&rpt_data::EmptySource, &report.data_definition);
    let paged = rpt_render::render_dataset_with(
        report,
        &dataset,
        Box::new(CosmicLayout::new(FontProvider::bundled())),
        Locale::default(),
        None,
        Some(DateTimeSpecials::from_unix_seconds(0)),
    );
    let bundled_opts = PdfOptions {
        fonts: FontSource::Bundled,
        ..Default::default()
    };
    let pdf = rpt_render::render_backend(&paged, &PdfBackend, &bundled_opts);
    assert!(pdf.starts_with(b"%PDF-"));
    // Every embedded face must be one of the bundled ones: the host's own Arial/Times reaching the
    // document would make the bytes a property of this machine.
    assert_only_bundled_faces(&pdf, "a hermetic render");
    assert_eq!(
        pdf,
        rpt_render::render_backend(&paged, &PdfBackend, &bundled_opts),
        "the hermetic render must be reproducible"
    );
}

/// The `PageBackend` seam must produce byte-identical output to the concrete free functions it
/// delegates to — it is an additive dispatch layer, not a new code path.
#[test]
fn render_backend_seam_matches_free_functions() {
    use rpt_pages::{
        DrawOp, FontSpec, ObjectKind, ObjectRef, Page, PageSize, PagedDocument, TextAlign, TextRun,
    };
    use rpt_reader::model::{Color, Rect, Twips};

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
        character_spacing: Twips(0),
        source: Some(ObjectRef::new("Details", ObjectKind::Field).named("f")),
    }));
    let doc = PagedDocument {
        pages: vec![page],
        ..Default::default()
    };

    // The backend seam and the free function must produce the same bytes.
    assert_eq!(
        rpt_render::render_backend(
            &doc,
            &rpt_render::PdfBackend,
            &rpt_render::PdfOptions::default()
        ),
        rpt_render_pdf::render_pages(&doc.pages),
    );
    // …and so must the fallible entry point the CLI uses, which is the same render without the
    // failure-page wrapper.
    assert_eq!(
        rpt_render::try_render_pages_with_assets(&doc.pages, &doc.assets)
            .expect("this document must serialize"),
        rpt_render_pdf::render_pages(&doc.pages),
    );
}
