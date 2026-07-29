//! The krilla PDF writer: real embedded fonts, subset `/Widths`, `/FlateDecode` streams.
//!
//! Drives [krilla] so text is drawn with real TrueType/CFF font subset embedding and content streams
//! are compressed. Its surface is y-down with a top-left origin — the same convention as the Page IR —
//! so there is no y-flip here.
//!
//! [krilla]: https://docs.rs/krilla

use crate::common::{
    aligned_x, approx_text_width, baseline_offset_twips, pt, KAPPA, MIN_STROKE_PT, TWIPS_PER_PT,
};
use crate::fill::{fill_of, rgb_alpha, solid, Bounds};
use crate::tagging::{self, ArtifactKind, UnitKind};
use crate::{ArtifactRole, Conformance, PdfError, PdfOptions, Timestamp};
use rpt_model::{Color, Rect, Twips};
use rpt_pages::{
    DrawOp, EllipseOp, FontSpec, ImageAsset, ImageFit, ImageOp, LineOp, Page, PageSize, PolygonOp,
    RectOp, SectionInfo, TextAlign, TextRun,
};
use rpt_text::{FontDb, FontSource};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use krilla::configure::{Accessibility, Archival, ConfigurationBuilder, ValidationError};
use krilla::error::KrillaError;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::metadata::{DateTime, Metadata};
use krilla::outline::Outline;
use krilla::page::PageSettings;
use krilla::paint::Stroke;
use krilla::surface::Surface;
use krilla::tagging::{
    Artifact, ArtifactType, ContentTag, Node, SpanTag, Tag, TagGroup, TagKind, TagTree,
};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Document, SerializeSettings};

/// The smallest page dimension, in points, a page is clamped to. A degenerate size must cost that one
/// page its geometry, not the whole document its existence.
const MIN_PAGE_PT: f32 = 1.0;

/// US Letter in points — the size of the single blank page an empty document still emits.
const LETTER_PT: (f32, f32) = (612.0, 792.0);

/// Render `pages` to PDF bytes via krilla, embedding each [`DrawOp::Image`] whose `image_id` resolves
/// in `assets`. Faces come from `opts.fonts` — the host's library by default, the bundled set for a
/// render that must be reproducible off this machine.
///
/// `sections` is the document's band dictionary, which a structure tree classifies page furniture
/// from; it is empty for the pages-only entry points, which see no document.
pub fn render(
    pages: &[Page],
    assets: &BTreeMap<String, ImageAsset>,
    sections: &BTreeMap<String, SectionInfo>,
    opts: &PdfOptions,
) -> Result<Vec<u8>, PdfError> {
    render_with_fonts(pages, assets, sections, FontCache::new(opts.fonts), opts)
}

/// A one-page PDF whose only content is `err`, so an infallible caller returns a document that states
/// the failure instead of one that looks ordinary. Drawn with the bundled fallback face alone, so it
/// does not depend on the host font that may have caused the failure, and with no conformance claim,
/// so a document that failed an archival level still yields a page saying so.
pub fn render_failure_page(err: &PdfError) -> Vec<u8> {
    let page = failure_page(&err.to_string());
    render_with_fonts(
        std::slice::from_ref(&page),
        &BTreeMap::new(),
        &BTreeMap::new(),
        FontCache::new(FontSource::Bundled),
        &PdfOptions::default(),
    )
    .unwrap_or_else(|page_err| {
        // The failure page has neither a host font nor an asset — the only two things krilla declines
        // — so this is unreachable; if it happens, report both failures rather than one.
        panic!("the PDF failure page did not serialize ({page_err}) after: {err}")
    })
}

/// The failure page's content: a heading plus the message, broken into fixed-width lines because
/// there is no layout pass here to wrap it.
fn failure_page(message: &str) -> Page {
    /// Chars per line — conservative for 11 pt across a Letter page's printable width.
    const WRAP: usize = 90;
    /// Message lines drawn; a longer message is truncated rather than running off the page.
    const MAX_LINES: usize = 4;

    let mut page = Page::new(
        1,
        PageSize {
            width: Twips(12240),
            height: Twips(15840),
        },
    );
    let mut lines = vec!["This PDF could not be rendered.".to_string()];
    let chars: Vec<char> = message.chars().collect();
    for chunk in chars.chunks(WRAP).take(MAX_LINES) {
        lines.push(chunk.iter().collect());
    }
    for (i, line) in lines.iter().enumerate() {
        page.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(1440),
                top: Twips(1440 + i as i32 * 360),
                width: Twips(9360),
                height: Twips(300),
            },
            text: line.clone(),
            font: FontSpec {
                size_pt: 11.0,
                ..FontSpec::default()
            },
            color: Color {
                a: 255,
                r: 0,
                g: 0,
                b: 0,
            },
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            character_spacing: Twips(0),
            source: None,
        }));
    }
    page
}

/// krilla settings for a page of `w`×`h` points, clamped to [`MIN_PAGE_PT`] so the conversion is
/// total: page sizes come from `i32` twips, and clamping is what makes every one of them a size krilla
/// accepts.
fn page_settings(w: f32, h: f32) -> PageSettings {
    PageSettings::from_wh(w.max(MIN_PAGE_PT), h.max(MIN_PAGE_PT))
        .expect("a page size clamped to a positive minimum is accepted by krilla")
}

/// krilla's error, reduced to the [`PdfError`] the crate reports (krilla's own types stay private to
/// this module).
fn pdf_error(err: KrillaError, level: Conformance) -> PdfError {
    match err {
        KrillaError::Font(_, msg) => PdfError::Font(msg),
        KrillaError::Image(_, _, msg) => PdfError::Image(msg),
        KrillaError::SixteenBitImage(..) => {
            PdfError::Image("a 16-bit image needs PDF 1.5 or later".to_string())
        }
        KrillaError::Validation(errors) => PdfError::Conformance {
            level,
            reasons: errors.iter().map(|(e, _)| unmet_requirement(e)).collect(),
        },
        other => PdfError::Serialize(format!("{other:?}")),
    }
}

/// One validation error, as the requirement the document did not meet. krilla's own variants name
/// the offending resource in a `Debug` form that is unreadable in a CLI message, so the cases a
/// report can actually hit get a sentence and the rest fall back to the raw form.
fn unmet_requirement(err: &ValidationError) -> String {
    match err {
        ValidationError::Transparency(_) => {
            "the document paints with transparency, which this level forbids".to_string()
        }
        ValidationError::ContainsNotDefGlyph(_, _, text) => format!(
            "no embedded face has a glyph for {text:?}, and the .notdef box drawn in its place is \
             forbidden"
        ),
        ValidationError::RestrictedLicense(_) => {
            "an embedded face's licence forbids the unrestricted embedding this level requires"
                .to_string()
        }
        ValidationError::MissingCMYKProfile => {
            "a CMYK color was used without an ICC profile to make it device-independent".to_string()
        }
        ValidationError::MissingDocumentDate => "the document has no creation date".to_string(),
        ValidationError::ContainsPostScript(_) => {
            "the document contains a PostScript function".to_string()
        }
        ValidationError::RequiresNewerPdfVersion(feature, _) => {
            format!("{feature:?} needs a newer PDF version than this level allows")
        }
        ValidationError::MissingTagging => {
            "the document carries no structure tree, which this level requires".to_string()
        }
        ValidationError::MissingAltText(_) => {
            "a figure has no alternate text describing it".to_string()
        }
        ValidationError::NoDocumentTitle => "the document has no title".to_string(),
        ValidationError::NoDocumentLanguage => {
            "the document declares no natural language".to_string()
        }
        ValidationError::MissingDocumentOutline => "the document has no outline".to_string(),
        ValidationError::NoCodepointMapping(_, _, text) => {
            format!("the glyphs drawn for {text:?} map to no Unicode codepoint, so the text cannot be extracted or read aloud")
        }
        other => format!("{other:?}"),
    }
}

/// The krilla settings a set of options asks for: the default ones, plus the validator that turns the
/// requested [`Conformance`] into a checked claim, plus tagging for a level that needs a structure
/// tree.
fn serialize_settings(opts: &PdfOptions) -> SerializeSettings {
    let conformance = opts.conformance;
    let enable_tagging = opts.tagged || conformance.requires_tagging();
    let (archival, accessibility) = (archival(conformance), accessibility(conformance));
    if archival.is_none() && accessibility.is_none() {
        return SerializeSettings {
            enable_tagging,
            ..SerializeSettings::default()
        };
    }
    let mut builder = ConfigurationBuilder::new();
    if let Some(archival) = archival {
        builder = builder.with_archival_validator(archival);
    }
    if let Some(accessibility) = accessibility {
        builder = builder.with_accessibility_validator(accessibility);
    }
    SerializeSettings {
        configuration: builder
            .finish()
            .expect("one validator alone always resolves to its own maximum PDF version"),
        enable_tagging,
        ..SerializeSettings::default()
    }
}

/// The krilla archival validator for a conformance level, or `None` for a level that claims no
/// archival standard (an ordinary PDF, or PDF/UA-1 — which is an accessibility standard only).
fn archival(conformance: Conformance) -> Option<Archival> {
    match conformance {
        Conformance::None | Conformance::PdfUa1 => None,
        Conformance::PdfA1b => Some(Archival::A1_B),
        Conformance::PdfA2b => Some(Archival::A2_B),
        Conformance::PdfA3b => Some(Archival::A3_B),
        Conformance::PdfA1a => Some(Archival::A1_A),
        Conformance::PdfA2a => Some(Archival::A2_A),
        Conformance::PdfA3a => Some(Archival::A3_A),
    }
}

/// The krilla accessibility validator for a conformance level. Only PDF/UA-1 has one; the level-A
/// archival standards carry their accessibility requirements inside the archival validator.
fn accessibility(conformance: Conformance) -> Option<Accessibility> {
    match conformance {
        Conformance::PdfUa1 => Some(Accessibility::UA1),
        _ => None,
    }
}

/// The semantics a tagging level needs but neither the Page IR nor this backend can invent, checked
/// before a single page is drawn so the failure names what to supply rather than what krilla noticed.
///
/// None of these is a default waiting to be filled in. A French report declared `en` is read aloud in
/// the wrong voice; a title taken from the file name is not the document's title; and a document
/// whose bands were never classified has its running header and footer tagged as content and read out
/// on every page, which is precisely the accessibility a claim would assert and not deliver. Each is
/// refused rather than guessed.
fn check_semantics(
    opts: &PdfOptions,
    sections: &BTreeMap<String, SectionInfo>,
) -> Result<(), PdfError> {
    if !opts.conformance.requires_tagging() {
        return Ok(());
    }
    let mut reasons = Vec::new();
    if opts.semantics.language.is_none() {
        reasons.push(
            "the document declares no natural language, so the language of its text cannot be \
             determined (PdfOptions::semantics.language)"
                .to_string(),
        );
    }
    if opts.conformance.requires_title() && opts.semantics.title.is_none() {
        reasons.push("the document has no title (PdfOptions::semantics.title)".to_string());
    }
    if sections.is_empty() && opts.semantics.artifact_sections.is_none() {
        reasons.push(
            "the document carries no sections and no override was supplied, so the report's bands \
             are unclassified: page headers and footers cannot be marked as pagination artifacts \
             and would be read as document content on every page (render the whole PagedDocument, \
             or classify the bands with PdfOptions::semantics.artifact_sections)"
                .to_string(),
        );
    }
    if reasons.is_empty() {
        return Ok(());
    }
    Err(PdfError::Conformance {
        level: opts.conformance,
        reasons,
    })
}

/// The document metadata to attach, or `None` to write none at all — which is what an anonymous,
/// dateless, unconforming render asks for.
///
/// The producer names the software that wrote the file, in both the info dictionary and the XMP
/// packet; krilla writes neither on its own, so nothing is being overridden. The date is separate:
/// every PDF/A level requires one, so a conforming render without an explicit date falls back to the
/// epoch rather than emitting a file that fails validation over metadata the caller never mentioned,
/// while an ordinary render leaves it out rather than reading a clock the bytes would then depend on.
fn metadata(opts: &PdfOptions) -> Option<Metadata> {
    let created = match (opts.created, opts.conformance) {
        (Some(created), _) => Some(created),
        (None, Conformance::None) => None,
        (None, _) => Some(Timestamp::default()),
    };
    let producer = opts.producer.as_str();
    let title = opts.semantics.title.as_deref();
    let language = opts.semantics.language.as_deref();
    if created.is_none() && producer.is_none() && title.is_none() && language.is_none() {
        return None;
    }
    let mut metadata = Metadata::new();
    if let Some(title) = title {
        metadata = metadata.title(title.to_string());
    }
    if let Some(language) = language {
        metadata = metadata.language(language.to_string());
    }
    if let Some(producer) = producer {
        // Both entries name this engine: it is what turned the report into these pages (`/Creator`)
        // and what serialized them (`/Producer`). The creator is also the software agent a PDF/A
        // provenance record cites.
        metadata = metadata
            .creator(producer.to_string())
            .producer(producer.to_string());
    }
    if let Some(created) = created {
        metadata = metadata.creation_date(krilla_date(created));
    }
    Some(metadata)
}

/// A [`Timestamp`] as krilla's date, always UTC and always with every component set: PDF/A wants a
/// complete date, and a partial one is what readers reject.
fn krilla_date(t: Timestamp) -> DateTime {
    DateTime::new(t.year)
        .month(t.month)
        .day(t.day)
        .hour(t.hour)
        .minute(t.minute)
        .second(t.second)
        .utc_offset_hour(0)
        .utc_offset_minute(0)
}

/// The shared render body, taking the resolved font source: the caller's choice for a report, the
/// bundled face alone for the failure page.
fn render_with_fonts(
    pages: &[Page],
    assets: &BTreeMap<String, ImageAsset>,
    sections: &BTreeMap<String, SectionInfo>,
    mut fonts: FontCache,
    opts: &PdfOptions,
) -> Result<Vec<u8>, PdfError> {
    // Decode each distinct image once, keyed by a content hash of its bytes: identical bytes (the
    // per-page logo, duplicate thumbnails) then share one krilla image handle, so krilla emits a
    // single XObject referenced many times instead of duplicating it per placement.
    let mut images: HashMap<u64, Option<Image>> = HashMap::new();
    // Every PDF/A level forbids `/Interpolate true`, so a conforming render embeds its rasters
    // unsmoothed. An ordinary render keeps the smoothing, which is what an upscaled logo needs.
    let interpolate = opts.conformance == Conformance::None;
    check_semantics(opts, sections)?;
    let tagged = opts.tagged || opts.conformance.requires_tagging();
    let roles = tagging::artifact_roles(sections, &opts.semantics);
    let mut document = Document::new_with(serialize_settings(opts));
    if let Some(metadata) = metadata(opts) {
        document.set_metadata(metadata);
    }
    if opts.conformance == Conformance::PdfUa1 {
        // PDF/UA-1 requires the outline entry to exist. A report's own navigation is its group
        // hierarchy, which the Page IR does not carry (the same gap as the band kinds), so what is
        // written is the empty outline krilla accepts for a validator that demands the entry.
        document.set_outline(Outline::new());
    }

    // krilla always emits at least one page; keep the "≥1 page" contract by letting an empty slice
    // produce a single blank Letter page.
    if pages.is_empty() {
        document.start_page_with(page_settings(LETTER_PT.0, LETTER_PT.1));
    }

    let mut tree = TagTree::new();
    let mut missing_alt: BTreeSet<String> = BTreeSet::new();

    for page in pages {
        let w = pt(page.size.width.0) as f32;
        let h = pt(page.size.height.0) as f32;
        let mut kpage = document.start_page_with(page_settings(w, h));
        let mut surface = kpage.surface();

        // Draw-op coordinates are printable-relative (0-based); translate by the page origin (the
        // report margin) so content sits inside the physical margins. krilla's surface is y-down
        // with a top-left origin — the same convention as the Page IR — so there is no y-flip here.
        let (ox, oy) = (pt(page.origin.x.0) as f32, pt(page.origin.y.0) as f32);
        let shifted = ox != 0.0 || oy != 0.0;
        if shifted {
            surface.push_transform(&Transform::from_translate(ox, oy));
        }

        let mut ctx = DrawCtx {
            fonts: &mut fonts,
            assets,
            images: &mut images,
            interpolate,
        };
        if tagged {
            tag_and_draw(
                &mut surface,
                &mut ctx,
                page,
                opts,
                &roles,
                &mut tree,
                &mut missing_alt,
            );
        } else {
            for op in &page.ops {
                draw_op(&mut surface, &mut ctx, op);
            }
        }

        if shifted {
            surface.pop();
        }
        surface.finish();
        kpage.finish();
    }

    if !missing_alt.is_empty() && opts.conformance.requires_tagging() {
        // Reported before serializing: krilla would also refuse the document, but its own error names
        // no object, and "which picture" is the whole of what the caller has to act on.
        return Err(PdfError::Conformance {
            level: opts.conformance,
            reasons: missing_alt
                .into_iter()
                .map(|object| {
                    format!(
                        "the figure {object:?} has no alternate text describing it \
                         (PdfOptions::semantics.alt_text)"
                    )
                })
                .collect(),
        });
    }
    if tagged {
        document.set_tag_tree(tree);
    }

    document
        .finish()
        .map_err(|e| pdf_error(e, opts.conformance))
}

/// The per-document drawing resources every op needs, so the op dispatch is one argument wide.
struct DrawCtx<'a> {
    fonts: &'a mut FontCache,
    assets: &'a BTreeMap<String, ImageAsset>,
    images: &'a mut HashMap<u64, Option<Image>>,
    interpolate: bool,
}

/// Paint one draw-op onto the surface.
fn draw_op(surface: &mut Surface, ctx: &mut DrawCtx, op: &DrawOp) {
    match op {
        DrawOp::Rect(r) => draw_rect(surface, r),
        DrawOp::Ellipse(e) => draw_ellipse(surface, e),
        DrawOp::Line(l) => draw_line(surface, l),
        DrawOp::Polygon(p) => draw_polygon(surface, p),
        DrawOp::Text(t) => draw_text(surface, ctx.fonts, t),
        DrawOp::Image(i) => draw_image(surface, ctx.assets, ctx.images, i, ctx.interpolate),
    }
}

/// Draw a page's ops with each run wrapped in the content tag its meaning calls for, appending the
/// page's `Sect` groups to the document tag tree.
///
/// Drawing stays in paint order — it decides what covers what — while the tree's children are sorted
/// into reading order, which is the whole point of the tree being a separate structure.
#[allow(clippy::too_many_arguments)]
fn tag_and_draw(
    surface: &mut Surface,
    ctx: &mut DrawCtx,
    page: &Page,
    opts: &PdfOptions,
    roles: &BTreeMap<String, ArtifactRole>,
    tree: &mut TagTree,
    missing_alt: &mut BTreeSet<String>,
) {
    let lang = opts.semantics.language.as_deref();
    for band in tagging::plan(page, roles, &opts.semantics) {
        // Drawn in paint order, one node per non-artifact unit; the nodes are then collected in the
        // band's reading order, which is why the tree exists separately from the marks.
        let mut nodes: Vec<Option<Node>> = Vec::with_capacity(band.units.len());
        for unit in &band.units {
            let ops = &page.ops[unit.ops.clone()];
            nodes.push(match &unit.kind {
                UnitKind::Paragraph => Some(draw_paragraph(surface, ctx, ops, lang)),
                UnitKind::Figure { object, alt } => {
                    if alt.is_none() {
                        missing_alt.insert(object.clone());
                    }
                    Some(draw_figure(surface, ctx, ops, alt.as_deref()))
                }
                UnitKind::Artifact(kind) => {
                    draw_artifact(surface, ctx, ops, *kind);
                    None
                }
            });
        }
        // A band of nothing but rules and backgrounds contributes no logical structure.
        let mut group = TagGroup::new(TagKind::from(Tag::Div));
        for index in band.reading_order() {
            if let Some(node) = nodes[index].take() {
                group.push(node);
            }
        }
        if !group.children.is_empty() {
            tree.push(group);
        }
    }
}

/// Draw one placed text object: each wrapped line is its own `Span` — krilla asks a span hold at most
/// one line — and the lines together are one `P`.
fn draw_paragraph(
    surface: &mut Surface,
    ctx: &mut DrawCtx,
    ops: &[DrawOp],
    lang: Option<&str>,
) -> Node {
    let mut group = TagGroup::new(TagKind::from(Tag::P));
    let last = ops.len().saturating_sub(1);
    for (i, op) in ops.iter().enumerate() {
        let actual = match op {
            DrawOp::Text(run) => tagging::actual_text(run, i == last),
            _ => None,
        };
        let tag = SpanTag::empty()
            .with_lang(lang)
            .with_actual_text(actual.as_deref());
        let id = surface.start_tagged(ContentTag::Span(tag));
        draw_op(surface, ctx, op);
        surface.end_tagged();
        group.push(Node::Leaf(id));
    }
    Node::Group(group)
}

/// Draw one graphic under a single content sequence: a chart's hundreds of paths and labels are one
/// figure, and PDF asks for one `Figure` per graphic rather than one per path.
fn draw_figure(
    surface: &mut Surface,
    ctx: &mut DrawCtx,
    ops: &[DrawOp],
    alt: Option<&str>,
) -> Node {
    let id = surface.start_tagged(ContentTag::Other);
    for op in ops {
        draw_op(surface, ctx, op);
    }
    surface.end_tagged();
    let mut group = TagGroup::new(TagKind::from(Tag::Figure(alt.map(str::to_string))));
    group.push(Node::Leaf(id));
    Node::Group(group)
}

/// Draw a run that is not part of the logical content. Marking it is not optional — a conforming file
/// requires every mark to be either tagged or declared an artifact — but its identifier is a dummy
/// that never enters the tree.
fn draw_artifact(surface: &mut Surface, ctx: &mut DrawCtx, ops: &[DrawOp], kind: ArtifactKind) {
    surface.start_tagged(ContentTag::Artifact(Artifact::with_kind(artifact_type(
        kind,
    ))));
    for op in ops {
        draw_op(surface, ctx, op);
    }
    surface.end_tagged();
}

/// The krilla [`ArtifactType`] an [`ArtifactKind`] maps to. Below PDF 1.7 krilla folds
/// `Header`/`Footer` back into the generic pagination type itself, so this needs no version check.
fn artifact_type(kind: ArtifactKind) -> ArtifactType {
    match kind {
        ArtifactKind::Layout => ArtifactType::Layout,
        ArtifactKind::Pagination(ArtifactRole::Header) => ArtifactType::Header,
        ArtifactKind::Pagination(ArtifactRole::Footer) => ArtifactType::Footer,
        ArtifactKind::Pagination(ArtifactRole::Pagination) => ArtifactType::PaginationOther,
    }
}

fn stroke_for(color: Color, width_twips: i32) -> Stroke {
    let (rgb, opacity) = rgb_alpha(color);
    Stroke {
        paint: rgb.into(),
        // A stored width of 0 (hairline) still needs to render; clamp to a thin visible line.
        width: (pt(width_twips) as f32).max(MIN_STROKE_PT as f32),
        opacity,
        ..Stroke::default()
    }
}

fn draw_rect(surface: &mut Surface, r: &RectOp) {
    if r.fill.is_none() && r.stroke.is_none() {
        return; // nothing to paint (the basic writer emits a no-op `n` here)
    }
    let x = pt(r.bounds.left.0) as f32;
    let y = pt(r.bounds.top.0) as f32;
    let w = pt(r.bounds.width.0) as f32;
    let ht = pt(r.bounds.height.0) as f32;
    let radius = (pt(r.corner_radius.0) as f32).clamp(0.0, (w / 2.0).min(ht / 2.0));
    let mut pb = PathBuilder::new();
    if radius > 0.0 {
        // A rounded rect: straight edges joined by four cubic-Bézier quarter arcs (kappa control
        // offset), matching the ellipse's corner construction.
        let k = radius * KAPPA as f32;
        let (l, t, rr, b) = (x, y, x + w, y + ht);
        pb.move_to(l + radius, t);
        pb.line_to(rr - radius, t);
        pb.cubic_to(rr - radius + k, t, rr, t + radius - k, rr, t + radius);
        pb.line_to(rr, b - radius);
        pb.cubic_to(rr, b - radius + k, rr - radius + k, b, rr - radius, b);
        pb.line_to(l + radius, b);
        pb.cubic_to(l + radius - k, b, l, b - radius + k, l, b - radius);
        pb.line_to(l, t + radius);
        pb.cubic_to(l, t + radius - k, l + radius - k, t, l + radius, t);
    } else {
        pb.move_to(x, y);
        pb.line_to(x + w, y);
        pb.line_to(x + w, y + ht);
        pb.line_to(x, y + ht);
    }
    pb.close();
    let Some(path) = pb.finish() else {
        return;
    };
    // `draw_path` fills and/or strokes according to the surface's current fill/stroke state, so set
    // exactly the ones this rect has (and clear the other) before drawing. A gradient/hatch fill is
    // built against the surface first, since a tiling pattern needs its own stream from it.
    let fill = r
        .fill
        .as_ref()
        .map(|f| fill_of(surface, f, Bounds::of(&r.bounds)));
    surface.set_fill(fill);
    surface.set_stroke(r.stroke.map(|s| stroke_for(s.color, s.width.0)));
    surface.draw_path(&path);
}

/// An axis-aligned ellipse inscribed in the op's bounds, built from four cubic-Bézier quarter
/// arcs (krilla has no native ellipse primitive).
fn draw_ellipse(surface: &mut Surface, e: &EllipseOp) {
    if e.bounds.width.0 <= 0 || e.bounds.height.0 <= 0 {
        return;
    }
    if e.fill.is_none() && e.stroke.is_none() {
        return;
    }
    let cx = pt(e.bounds.left.0) as f32 + pt(e.bounds.width.0) as f32 / 2.0;
    let cy = pt(e.bounds.top.0) as f32 + pt(e.bounds.height.0) as f32 / 2.0;
    let rx = pt(e.bounds.width.0) as f32 / 2.0;
    let ry = pt(e.bounds.height.0) as f32 / 2.0;
    let k = KAPPA as f32;
    let (kx, ky) = (rx * k, ry * k);
    let mut pb = PathBuilder::new();
    pb.move_to(cx + rx, cy);
    pb.cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
    pb.cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
    pb.cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
    pb.cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
    pb.close();
    let Some(path) = pb.finish() else {
        return;
    };
    let fill = e
        .fill
        .as_ref()
        .map(|f| fill_of(surface, f, Bounds::of(&e.bounds)));
    surface.set_fill(fill);
    surface.set_stroke(e.stroke.map(|s| stroke_for(s.color, s.width.0)));
    surface.draw_path(&path);
}

fn draw_line(surface: &mut Surface, l: &LineOp) {
    let mut pb = PathBuilder::new();
    pb.move_to(pt(l.from.x.0) as f32, pt(l.from.y.0) as f32);
    pb.line_to(pt(l.to.x.0) as f32, pt(l.to.y.0) as f32);
    let Some(path) = pb.finish() else {
        return;
    };
    surface.set_fill(None);
    surface.set_stroke(Some(stroke_for(l.stroke.color, l.stroke.width.0)));
    surface.draw_path(&path);
}

fn draw_polygon(surface: &mut Surface, p: &PolygonOp) {
    if p.points.len() < 2 {
        return;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(pt(p.points[0].x.0) as f32, pt(p.points[0].y.0) as f32);
    for pt_ in &p.points[1..] {
        pb.line_to(pt(pt_.x.0) as f32, pt(pt_.y.0) as f32);
    }
    if p.closed {
        pb.close();
    }
    let Some(path) = pb.finish() else {
        return;
    };
    // Only a closed region fills; an open polyline just strokes.
    let fill = p.closed.then_some(p.fill.as_ref()).flatten().map(|f| {
        let bounds = Bounds::of_points(
            p.points
                .iter()
                .map(|q| (pt(q.x.0) as f32, pt(q.y.0) as f32)),
        );
        fill_of(surface, f, bounds)
    });
    surface.set_fill(fill);
    surface.set_stroke(p.stroke.map(|s| stroke_for(s.color, s.width.0)));
    surface.draw_path(&path);
}

/// Draw an image op: look up its bytes in `assets`, decode to a krilla image, and paint it filling
/// the op's box. An unresolved id or an undecodable/unsupported raster is skipped (the box's own
/// border, if any, still shows) — matching the other backends' "no bytes, no picture" behaviour.
fn draw_image(
    surface: &mut Surface,
    assets: &BTreeMap<String, ImageAsset>,
    images: &mut HashMap<u64, Option<Image>>,
    i: &ImageOp,
    interpolate: bool,
) {
    let Some(asset) = assets.get(&i.image_id) else {
        return;
    };
    // Decode once per distinct byte string; a repeated image reuses the cached krilla handle.
    let image = images
        .entry(rpt_render_util::content_hash(&asset.bytes))
        .or_insert_with(|| load_image(asset, interpolate))
        .clone();
    let Some(image) = image else {
        return;
    };
    let (bw, bh) = (pt(i.bounds.width.0) as f32, pt(i.bounds.height.0) as f32);
    // `Fill` draws the raster to the whole box (distorting aspect); `Contain` scales it uniformly to
    // the largest fit and centers it, leaving the surrounding space empty — Crystal letterboxes.
    let (dw, dh, ox, oy) = match i.fit {
        ImageFit::Fill => (bw, bh, 0.0, 0.0),
        ImageFit::Contain => {
            let (iw, ih) = image.size();
            let (iw, ih) = (iw as f32, ih as f32);
            if iw <= 0.0 || ih <= 0.0 {
                (bw, bh, 0.0, 0.0)
            } else {
                let s = (bw / iw).min(bh / ih);
                let (dw, dh) = (iw * s, ih * s);
                (dw, dh, (bw - dw) / 2.0, (bh - dh) / 2.0)
            }
        }
    };
    let Some(size) = Size::from_wh(dw, dh) else {
        return;
    };
    let (x, y) = (pt(i.bounds.left.0) as f32, pt(i.bounds.top.0) as f32);
    surface.push_transform(&Transform::from_translate(x + ox, y + oy));
    surface.draw_image(image, size);
    surface.pop();
}

/// Decode an image asset into a krilla [`Image`]. PNG/JPEG/GIF go through krilla's decoders; a BMP
/// (the format Crystal stores embedded OLE bitmaps as) is decoded here to RGBA and embedded raw,
/// since krilla has no BMP path. `None` for anything else or a decode failure.
fn load_image(asset: &ImageAsset, interpolate: bool) -> Option<Image> {
    match asset.media_type.as_str() {
        "image/png" => Image::from_png(asset.bytes.clone().into(), interpolate).ok(),
        "image/jpeg" => Image::from_jpeg(asset.bytes.clone().into(), interpolate).ok(),
        "image/gif" => Image::from_gif(asset.bytes.clone().into(), interpolate).ok(),
        "image/bmp" => {
            let (rgba, w, h) = rpt_render_util::decode_bmp_rgba(&asset.bytes)?;
            Some(Image::from_rgba8(rgba, w, h))
        }
        _ => None,
    }
}

/// Draw a justified line word by word, flushing both edges: each word is shaped and placed at a
/// running pen, and every inter-word gap advances by the space width plus `extra`.
///
/// The drawn glyphs and the pen advance come from the *same* shaping call, so a word can never be
/// drawn at one width and accounted for at another — the cumulative drift a justified line is prone
/// to is excluded by construction rather than by keeping two shapers in agreement.
///
/// Each gap draws a real space glyph, widened to the stretched gap, rather than moving the pen over
/// empty space: a glyph is what carries the character into the PDF's text, and without one the line
/// extracts as a single run-together token. It cannot move the words, because each of them is placed
/// at its own explicit pen.
///
/// `spacing` is the run's per-scalar character spacing; the space between two words is a scalar like
/// any other, so the gap owes it once as well.
fn draw_justified_words(
    surface: &mut Surface,
    shaper: &Shaper,
    size: f32,
    text: &str,
    origin: Point,
    extra: f32,
    spacing: f32,
) {
    let (mut gap, space_w) = shaper.shape(" ", size, spacing);
    // Advances are em fractions, so the justification extra converts into that space once. It rides
    // on the last glyph so a space that shaped into several keeps its parts together.
    if let Some(last) = gap.last_mut() {
        if size > 0.0 {
            last.x_advance += extra / size;
        }
    }
    let mut pen = origin.x;
    for (wi, word) in text.split(' ').enumerate() {
        if wi > 0 {
            if !gap.is_empty() {
                surface.draw_glyphs(
                    Point::from_xy(pen, origin.y),
                    &gap,
                    shaper.font(),
                    " ",
                    size,
                    false,
                );
            }
            pen += space_w + extra;
        }
        if word.is_empty() {
            continue;
        }
        let (glyphs, advance) = shaper.shape(word, size, spacing);
        if !glyphs.is_empty() {
            surface.draw_glyphs(
                Point::from_xy(pen, origin.y),
                &glyphs,
                shaper.font(),
                word,
                size,
                false,
            );
        }
        pen += advance;
    }
}

fn draw_text(surface: &mut Surface, fonts: &mut FontCache, t: &TextRun) {
    if t.text.is_empty() {
        return;
    }
    // Split the run into maximal single-face segments: the primary family where it has the glyph,
    // else the bundled symbol fallback for the chars it lacks (⚠ etc.) — so a missing glyph renders
    // from the fallback face instead of a `.notdef` box. Segments carry byte ranges into `t.text`.
    let segments = fonts.db.segment_by_coverage(&t.font, &t.text);
    if segments.is_empty() {
        return; // no usable face on this host — skip rather than emit nothing-glyphs
    }
    let size = t.font.size_pt.max(1.0);
    // Baseline = top + ascent (krilla places the run at its baseline; y-down, no flip). The
    // metrics-present ascent is the shared `baseline_offset_twips`, scaled from twips to points;
    // the no-metrics fallback stays in the surface's point/f32 space (its rounding differs from the
    // twips heuristic, and krilla output must be byte-stable), so only that arm is kept local.
    let ascent_pt = match &t.metrics {
        Some(_) => (baseline_offset_twips(t) / TWIPS_PER_PT) as f32,
        None => size * 0.8,
    };
    let baseline_y = pt(t.bounds.top.0) as f32 + ascent_pt;
    // Horizontal alignment: shift x by the run's stored advance for centre/right (else the
    // approximate width). Shared anchor math with the basic writer; only the point conversion
    // differs. Segments then flow left-to-right from this anchor by their own shaped advances.
    // Rigid character spacing is part of the producer's advance model, so it is already inside a
    // measured `metrics.advance`; the estimate has to add it itself to anchor a spaced run correctly.
    let spacing_pt = pt(t.character_spacing.0);
    let text_w = match &t.metrics {
        Some(m) => pt(m.advance.0),
        None => {
            approx_text_width(&t.text, size as f64) + spacing_pt * t.text.chars().count() as f64
        }
    };
    let x = aligned_x(t.align, pt(t.bounds.left.0), pt(t.bounds.width.0), text_w) as f32;
    // Text is fill-only. Clear any stroke left active by a preceding path op (a rule/line/rect) so
    // krilla does not fill *and* stroke the glyphs — a leaked stroke renders the run doubled/haloed.
    surface.set_fill(Some(solid(t.color)));
    surface.set_stroke(None);
    // Rotation is CCW degrees about the run's origin (top-left of bounds). krilla's surface is
    // y-down (like `from_rotate_at`'s CW-positive angle), so negate to render CCW. `0.0` pushes no
    // transform, keeping upright output identical.
    let rotated = t.rotation != 0.0;
    if rotated {
        let (px, py) = (pt(t.bounds.left.0) as f32, pt(t.bounds.top.0) as f32);
        surface.push_transform(&Transform::from_rotate_at(-t.rotation, px, py));
    }
    // Justified: spread the box's slack across the inter-word gaps so both edges flush. The layout
    // marks a paragraph's last line `Left`, so only interior wrapped lines reach this.
    let extra =
        rpt_render_util::justify_gap_extra(t.align, &t.text, pt(t.bounds.width.0), text_w) as f32;
    place_glyphs(
        surface,
        fonts,
        t,
        &segments,
        size,
        Point::from_xy(x, baseline_y),
        extra,
        spacing_pt as f32,
    );
    draw_text_decoration(surface, t, x, baseline_y, ascent_pt, size, text_w);
    if rotated {
        surface.pop();
    }
}

/// Place a run's glyphs from `origin` (its anchor on the baseline): every segment shaped here and
/// handed to krilla's `draw_glyphs`, which places glyphs a caller has already positioned. `draw_text`
/// is not used — it shapes with a single face and no BIDI, which a fallback-capable renderer is
/// outside the scope of, and it returns no advance, so a pen could not follow it anyway.
///
/// Segments flow left to right from `origin`, each advancing the pen by its own shaped width, so text
/// after a fallback glyph keeps its place. A justified run instead spreads `extra` across its
/// inter-word gaps; that needs one face across the whole line, so a mixed-face run flows normally.
///
/// `spacing` is [`TextRun::character_spacing`] in points, charged per Unicode scalar by the shaper.
#[allow(clippy::too_many_arguments)]
fn place_glyphs(
    surface: &mut Surface,
    fonts: &mut FontCache,
    t: &TextRun,
    segments: &[rpt_text::FaceRun],
    size: f32,
    origin: Point,
    extra: f32,
    spacing: f32,
) {
    // The one face a justified line needs — its words share a single pen, so the whole line must be
    // one non-substituted segment. `None` for any other run, which flows its segments instead.
    let justified = match segments {
        [seg] if extra > 0.0 && !seg.substituted => fonts.face(seg.face),
        _ => None,
    };
    if let Some(face) = justified {
        if let Some(shaper) = Shaper::new(&face) {
            draw_justified_words(surface, &shaper, size, &t.text, origin, extra, spacing);
        }
        return;
    }
    let mut pen_x = origin.x;
    for seg in segments {
        let Some(face) = fonts.face(seg.face) else {
            continue;
        };
        let Some(shaper) = Shaper::new(&face) else {
            continue;
        };
        let seg_text = &t.text[seg.range.clone()];
        let (glyphs, advance) = shaper.shape(seg_text, size, spacing);
        if !glyphs.is_empty() {
            surface.draw_glyphs(
                Point::from_xy(pen_x, origin.y),
                &glyphs,
                shaper.font(),
                seg_text,
                size,
                false,
            );
        }
        pen_x += advance;
    }
}

/// Underline / strikethrough: thin filled bars across the run's drawn extent (called inside any
/// active rotation transform so they follow rotated text). Underline sits just below the baseline;
/// strikethrough crosses the x-height. `text_w` is the same advance used for anchoring.
#[allow(clippy::too_many_arguments)]
fn draw_text_decoration(
    surface: &mut Surface,
    t: &TextRun,
    x: f32,
    baseline_y: f32,
    ascent_pt: f32,
    size: f32,
    text_w: f64,
) {
    if !(t.font.underline || t.font.strikethrough) {
        return;
    }
    let thickness = (size * 0.06).max(MIN_STROKE_PT as f32);
    let bar_w = text_w as f32;
    if t.font.underline {
        fill_bar(
            surface,
            x,
            baseline_y + thickness,
            bar_w,
            thickness,
            t.color,
        );
    }
    if t.font.strikethrough {
        fill_bar(
            surface,
            x,
            baseline_y - ascent_pt * 0.3,
            bar_w,
            thickness,
            t.color,
        );
    }
}

/// Fill an axis-aligned bar (underline / strikethrough) as a rectangle path.
fn fill_bar(surface: &mut Surface, x: f32, y: f32, w: f32, h: f32, color: Color) {
    if w <= 0.0 {
        return;
    }
    let h = h.max(MIN_STROKE_PT as f32);
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x + w, y);
    pb.line_to(x + w, y + h);
    pb.line_to(x, y + h);
    pb.close();
    let Some(path) = pb.finish() else {
        return;
    };
    surface.set_fill(Some(solid(color)));
    surface.set_stroke(None);
    surface.draw_path(&path);
}

/// A face loaded once for the PDF backend: the krilla [`Font`] (subset-embedded on write) plus the
/// raw bytes kept for shaping each segment (krilla does not expose a face's bytes back). `upem` is
/// cached so glyph advances normalise without re-parsing, and `shaper_data` holds harfrust's parsed
/// tables so the expensive part of shaping happens once per FACE rather than once per run.
struct LoadedFace {
    font: Font,
    data: Rc<Vec<u8>>,
    index: u32,
    upem: f32,
    shaper_data: harfrust::ShaperData,
}

/// Resolves host fonts via the shared [`rpt_text::FontDb`] and memoizes each loaded face **by id**,
/// so a face is read and subset once no matter how many families or fallback segments reference it —
/// this is what keeps krilla's per-face subsetting to one embedded copy. The resolution/coverage
/// policy (`segment_by_coverage`) lives in `FontDb`; this backend keeps only the krilla+shaping parse.
struct FontCache {
    db: FontDb,
    /// `None` value = we looked and the face failed to load (don't re-read).
    faces: HashMap<fontdb::ID, Option<Rc<LoadedFace>>>,
}

impl FontCache {
    /// A cache over the face library `source` names. [`FontSource::Bundled`] resolves the same way on
    /// every host — used for the failure page, whose whole job is to render when a host font did not.
    fn new(source: FontSource) -> FontCache {
        FontCache {
            db: source.load(),
            faces: HashMap::new(),
        }
    }

    /// The loaded face for `id`, read and parsed once then cached (one krilla `Font` per face → one
    /// subset embed). Keeps the raw bytes so segments can be shaped with harfrust.
    fn face(&mut self, id: fontdb::ID) -> Option<Rc<LoadedFace>> {
        if let Some(cached) = self.faces.get(&id) {
            return cached.clone();
        }
        let loaded = self
            .db
            .with_face_data(id, |data, index| {
                let bytes = Rc::new(data.to_vec());
                let font = Font::new(bytes.as_ref().clone().into(), index)?;
                let upem = font.units_per_em();
                // Parse the shaping tables once, here, while the bytes are in hand — a face is loaded
                // once per document however many families or fallback segments reach it.
                let shaper_data =
                    harfrust::ShaperData::new(&harfrust::FontRef::from_index(data, index).ok()?);
                Some(Rc::new(LoadedFace {
                    font,
                    data: bytes,
                    index,
                    upem,
                    shaper_data,
                }))
            })
            .flatten();
        self.faces.insert(id, loaded.clone());
        loaded
    }
}

/// One face made ready to shape with: a harfrust shaper over [`LoadedFace::data`], borrowed alongside
/// the krilla [`Font`] the shaped glyphs are drawn with.
///
/// **It shapes with harfrust, and it must.** The layout engine measures text through cosmic-text,
/// which shapes with harfrust; a second implementation here would leave the two free to disagree about
/// how wide a run is, and a PDF that draws text narrower than the box it was laid out for is the
/// failure that follows. One shaper on both sides makes that agreement structural rather than
/// something to keep re-measuring.
///
/// It does not make the two agree on its own: cosmic-text shapes word by word and this shapes whole
/// runs, so a kern pair straddling a word boundary is applied here and not there. That granularity
/// difference is the residual the PDF artifact checks measure, and it is not a shaper difference.
///
/// It also exists to hold the parse: building the shaper reparses the whole font, so that belongs to
/// the run rather than to each string shaped within it — a justified line shapes once per word, and
/// parsing per word made the font cost scale with the word count. It borrows the face's bytes, so it
/// is a short-lived value built at the top of a run's drawing path, not a field on `LoadedFace` (which
/// would make that type self-referential).
struct Shaper<'a> {
    face: &'a LoadedFace,
    shaper: harfrust::Shaper<'a>,
}

impl<'a> Shaper<'a> {
    /// Parse `face` for shaping; `None` if the font data does not parse.
    fn new(face: &'a LoadedFace) -> Option<Shaper<'a>> {
        let font = harfrust::FontRef::from_index(&face.data, face.index).ok()?;
        Some(Shaper {
            face,
            shaper: face.shaper_data.shaper(&font).build(),
        })
    }

    /// The krilla font shaped glyphs from this face must be drawn with.
    fn font(&self) -> Font {
        self.face.font.clone()
    }

    /// Shape one single-face string into krilla glyphs, returning them and the string's total advance
    /// in points. Advances are normalised by the face's units-per-em (as `KrillaGlyph` expects) then
    /// scaled by `size` for the returned pen advance. LTR/auto direction — each segment is one face
    /// and, in practice, one script; RTL visual order is deferred.
    ///
    /// `spacing` is [`TextRun::character_spacing`] in points, charged **per Unicode scalar** and added
    /// after the last glyph of each shaped cluster — so a cluster that shapes several scalars into one
    /// ligature glyph owes spacing for each of them. Per glyph would short exactly that case, and the
    /// producer measured (and wrapped) the run per scalar, so the two would silently disagree.
    ///
    /// PDF's own `Tc` operator cannot express this: it adds its spacing to every glyph *shown*, which
    /// is the per-glyph rule. The extra therefore rides in the glyph advances, which krilla writes as
    /// the run's `TJ` adjustments.
    fn shape(&self, text: &str, size: f32, spacing: f32) -> (Vec<KrillaGlyph>, f32) {
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let output = self.shaper.shape(buffer, &[]);
        let positions = output.glyph_positions();
        let infos = output.glyph_infos();
        let upem = self.face.upem;
        // Glyph advances are em fractions, so the per-scalar extra converts into the same space once.
        let spacing_norm = if size > 0.0 { spacing / size } else { 0.0 };
        let mut glyphs = Vec::with_capacity(output.len());
        let mut advance_norm = 0.0f32;
        for i in 0..output.len() {
            let pos = positions[i];
            let info = infos[i];
            // Cluster → byte range in `text`: this glyph's cluster start up to the next differing
            // cluster (LTR). harfrust clusters are byte offsets into the pushed string, so the range
            // is always on a UTF-8 boundary — correct for the ToUnicode mapping krilla derives from
            // it.
            let start = info.cluster as usize;
            let end = infos[i + 1..]
                .iter()
                .map(|n| n.cluster as usize)
                .find(|&c| c != start)
                .unwrap_or(text.len());
            let mut x_adv = pos.x_advance as f32 / upem;
            // Charge the cluster's spacing once, on its last glyph, so it lands between clusters
            // rather than inside one (a mark would otherwise drift off its base).
            let cluster_end = infos.get(i + 1).map(|n| n.cluster as usize) != Some(start);
            if cluster_end && spacing_norm != 0.0 {
                x_adv += spacing_norm * text[start..end].chars().count() as f32;
            }
            glyphs.push(KrillaGlyph::new(
                GlyphId::new(info.glyph_id),
                x_adv,
                pos.x_offset as f32 / upem,
                pos.y_offset as f32 / upem,
                pos.y_advance as f32 / upem,
                start..end,
                None,
            ));
            advance_norm += x_adv;
        }
        (glyphs, advance_norm * size)
    }
}

#[cfg(test)]
mod tests;
