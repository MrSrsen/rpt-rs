//! PDF inspection: inflate a document's content streams and project them into a reviewable operator
//! listing, or read the object graph a viewer resolves.
//!
//! Two read-only surfaces over finished PDF bytes, sharing one parser:
//!
//! - [`operator_listing`] — the pages laid out one by one with each page's font and image resources
//!   resolved and its glyph strings decoded back to text. It serves both the committed baselines and
//!   the op-level assertions (`the run must be fill-only`, `an ellipse is four Béziers`), because both
//!   want the same thing: the writer's operators, one per line, independent of how it broke its lines.
//!   PDF treats the whitespace between operands and operators as optional, so a raw inflated stream is
//!   not a stable assertion surface — `)]TJ` and `) ] TJ` are the same drawing.
//! - [`structure`] — the same document as *objects*: page count, each page's font and image resources
//!   with the width table and descriptor a reader needs, and the text shows with the advance a viewer
//!   computes from them. This is the surface for asserting properties of the artifact rather than
//!   diffing the writer's output.
//!
//! Only enough of the PDF object model is parsed to walk `Catalog → Pages → Kids` and read a page's
//! `/Resources` and `/Contents`. Anything else in the file is ignored.

use std::collections::BTreeMap;
use std::fmt;

/// Project `pdf` into a normalized, reviewable listing of what the writer emitted: one block per
/// page, holding the page's media box, the faces and images its resource dictionary binds, and its
/// content stream as an operator-per-line listing.
///
/// What is normalized, and why — the listing is a *baseline* surface, so it must move exactly when the
/// writer's output changes and never otherwise:
///
/// - **Glyph strings are decoded back to text** through the font's own `/ToUnicode` map. The bytes on
///   the wire are subset glyph indices assigned in order of first use, so any change renumbers every
///   later glyph: unreadable as a diff, and it moves for reasons that are not about the drawing. The
///   decoded text is what the run says, and a broken `/ToUnicode` map shows up as garbled text.
/// - **Face names lose their subset tag** (`/SHYXUL+LiberationSans` → `LiberationSans`). The tag
///   identifies a subset, not a face, and it is not something a reviewer can act on.
/// - **Numbers are rounded to 3 decimals** — 1/1000 pt, finer than the twip the layout works in, so no
///   real geometry change is hidden while f32 representation noise (`120.200005`) is.
///
/// Object and resource *numbering* is left alone: `/f0`, `/x0` and the page order are the writer's own
/// naming and a change in them is a change worth seeing.
pub fn operator_listing(pdf: &[u8]) -> String {
    let objs = objects(pdf);
    let pages = page_objects(&objs);
    let mut out = format!("% {} page(s)\n", pages.len());
    for (i, num) in pages.iter().enumerate() {
        let Some(page) = objs.get(num) else {
            out.push_str(&format!("% page {}: missing object {num}\n", i + 1));
            continue;
        };
        let media = dict_array(page.dict, "MediaBox").unwrap_or("?").trim();
        out.push_str(&format!("% page {} media [{media}]\n", i + 1));

        let resources = page_resources(&objs, page);
        let fonts = page_fonts(&objs, resources);
        for (name, font) in &fonts {
            out.push_str(&format!("% font /{name} = {}\n", font.face));
        }
        for (name, image) in page_images(&objs, resources) {
            out.push_str(&format!("% image /{name} = {image}\n"));
        }

        match dict_ref(page.dict, "Contents")
            .and_then(|n| objs.get(&n))
            .and_then(stream_bytes)
        {
            Some(bytes) => out.push_str(&normalize_content(&bytes, &fonts)),
            None => out.push_str("% <no content stream>\n"),
        }
    }
    out
}

/// Read `pdf`'s object graph: the pages a viewer would show, each with its resolved font and image
/// resources and the text its content stream draws.
///
/// The object-level counterpart to [`operator_listing`]. Where the listing is a *text projection* of
/// the operators (blessed as a baseline, so it normalizes anything incidental), this returns the
/// *values* a reader resolves — page count, a font's declared glyph widths, an image XObject's filter
/// — for assertions about the finished artifact that a diff of the operators cannot make.
pub fn structure(pdf: &[u8]) -> PdfStructure {
    let objs = objects(pdf);
    let pages = page_objects(&objs)
        .into_iter()
        .map(|num| {
            let Some(page) = objs.get(&num) else {
                return PdfPage::default();
            };
            let resources = page_resources(&objs, page);
            let fonts = page_fonts(&objs, resources);
            let shows = dict_ref(page.dict, "Contents")
                .and_then(|n| objs.get(&n))
                .and_then(stream_bytes)
                .map(|bytes| text_shows(&bytes, &fonts))
                .unwrap_or_default();
            PdfPage {
                media_box: dict_array(page.dict, "MediaBox")
                    .unwrap_or("")
                    .split_whitespace()
                    .filter_map(|n| n.parse().ok())
                    .collect(),
                images: page_images(&objs, resources),
                fonts,
                shows,
            }
        })
        .collect();
    PdfStructure { pages }
}

/// A rendered PDF's object graph, as the artifact checks read it.
#[derive(Debug, Clone, Default)]
pub struct PdfStructure {
    /// The pages in document order (`Catalog → /Pages → /Kids`).
    pub pages: Vec<PdfPage>,
}

impl PdfStructure {
    /// Every page's font resources, deduplicated by `/BaseFont` — the subset-tagged name, which
    /// identifies one embedded face. A face reached from several pages or several resource names
    /// appears once; the same name resolving to two objects means the writer embedded it twice.
    pub fn faces(&self) -> BTreeMap<String, Vec<u32>> {
        let mut out: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for font in self.pages.iter().flat_map(|p| p.fonts.values()) {
            let objects = out.entry(font.base_font.clone()).or_default();
            if !objects.contains(&font.object) {
                objects.push(font.object);
            }
        }
        out
    }
}

/// One page: its media box, the resources it binds, and the text it shows.
#[derive(Debug, Clone, Default)]
pub struct PdfPage {
    /// The `/MediaBox` numbers, in PDF points.
    pub media_box: Vec<f64>,
    /// The `/Font` resource map: resource name (`f0`) → the font it binds.
    pub fonts: BTreeMap<String, PdfFont>,
    /// The `/XObject` image resource map: resource name (`x0`) → the image it binds.
    pub images: BTreeMap<String, PdfImage>,
    /// The content stream's text-showing operators, in paint order.
    pub shows: Vec<PdfTextShow>,
}

/// A font as the content stream sees it: the face it draws with, the widths it declares for its
/// glyphs, and the map that turns its glyph codes back into text.
///
/// A composite (`/Type0`) font keeps its widths on the descendant CIDFont's `/W` array and a simple
/// font on its own `/Widths`; both are read into [`Self::widths`], keyed by glyph code.
#[derive(Debug, Clone, Default)]
pub struct PdfFont {
    /// The font object's number.
    pub object: u32,
    /// `/BaseFont` verbatim, subset tag included (`SHYXUL+LiberationSans`).
    pub base_font: String,
    /// The face name with its subset tag stripped (`LiberationSans`).
    pub face: String,
    /// `/Subtype` (`Type0` for the composite fonts krilla writes).
    pub subtype: String,
    /// The descendant CIDFont's object number, for a composite font.
    pub cid_object: Option<u32>,
    /// Whether the font (or its descendant) carries a `/FontDescriptor`.
    pub has_descriptor: bool,
    /// Whether that descriptor embeds the font program (`/FontFile`/`/FontFile2`/`/FontFile3`) — i.e.
    /// the face travels with the document instead of being expected on the reader's machine.
    pub has_font_program: bool,
    /// Glyph code → advance width in 1000-unit em space, from `/W` or `/Widths`.
    pub widths: BTreeMap<u32, f64>,
    /// The `/DW` default width, used for a glyph the width table omits.
    pub default_width: f64,
    /// Glyph code → the text it stands for, from the font's `/ToUnicode` CMap.
    pub to_unicode: BTreeMap<u32, String>,
    /// Type0 fonts address glyphs with two bytes per code (`/Identity-H`); a simple font uses one.
    pub two_byte: bool,
}

impl PdfFont {
    /// The width this font declares for glyph `code`, in 1000-unit em space.
    pub fn width(&self, code: u32) -> f64 {
        self.widths
            .get(&code)
            .copied()
            .unwrap_or(self.default_width)
    }
}

/// An image XObject: enough to tell a dropped, re-encoded or resampled picture from a passed-through
/// one without putting pixel bytes in a test.
#[derive(Debug, Clone, Default)]
pub struct PdfImage {
    /// The XObject's object number.
    pub object: u32,
    /// `/Subtype` (`Image`).
    pub subtype: String,
    /// The stream `/Filter`, space-separated when it is a filter chain; `none` when unfiltered.
    pub filter: String,
    /// `/Width` in samples.
    pub width: i64,
    /// `/Height` in samples.
    pub height: i64,
    /// `/ColorSpace`.
    pub color_space: String,
}

impl fmt::Display for PdfImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}x{} {}",
            self.subtype, self.filter, self.width, self.height, self.color_space
        )
    }
}

/// One text-showing operator (`Tj`/`TJ`), with the pen advance a reader computes for it.
#[derive(Debug, Clone, Default)]
pub struct PdfTextShow {
    /// The `/Font` resource name the preceding `Tf` selected.
    pub font: String,
    /// The `Tf` size operand, in points.
    pub size: f64,
    /// The glyph codes shown, in order.
    pub glyphs: Vec<u32>,
    /// The text those glyphs stand for, decoded through the font's `/ToUnicode`.
    pub text: String,
    /// What the *font* prices this show at, in points: the widths its own width table declares for
    /// these glyphs, scaled by the `Tf` size. This is the value a text metric must agree with — the
    /// width table is where a reader gets its geometry, so a table that disagrees with the advance the
    /// text was laid out at displaces every glyph after the first.
    pub declared_pt: f64,
    /// Where the pen lands, in points: [`Self::declared_pt`] less the `TJ` position adjustments the
    /// writer emitted. The difference between the two is the writer's own kerning.
    pub pen_pt: f64,
    /// The `TJ` position adjustments, as `(index into `glyphs` the adjustment precedes, offset in
    /// 1000-unit em space)`. A positive offset pulls the following glyph closer — i.e. a kern. Kept
    /// positioned rather than summed so a caller can tell *which* pair each one belongs to.
    pub kerns: Vec<(usize, f64)>,
}

// ---------------------------------------------------------------------------------------------
// Object layer
// ---------------------------------------------------------------------------------------------

/// One indirect object: its dictionary as text, plus its stream bytes when it has one.
struct Object<'a> {
    dict: &'a str,
    stream: Option<&'a [u8]>,
}

/// Index every `N 0 obj` in `pdf` by object number.
///
/// A stream's extent comes from the dictionary's `/Length` when that is a direct integer (the only
/// reliable bound — binary stream data can contain `endstream`), else from the next `endstream`
/// keyword.
fn objects(pdf: &[u8]) -> BTreeMap<u32, Object<'_>> {
    const HEAD: &[u8] = b" 0 obj";
    let mut out = BTreeMap::new();
    let mut i = 0;
    while let Some(rel) = find(&pdf[i..], HEAD) {
        let kw = i + rel;
        let body_at = kw + HEAD.len();
        i = body_at;
        // Walk back over the object number; it must start a line.
        let mut start = kw;
        while start > 0 && pdf[start - 1].is_ascii_digit() {
            start -= 1;
        }
        if start == kw || (start > 0 && !matches!(pdf[start - 1], b'\n' | b'\r')) {
            continue;
        }
        let Some(num) = std::str::from_utf8(&pdf[start..kw])
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let stream_at = find_token(&pdf[body_at..], b"stream");
        let endobj_at = find_token(&pdf[body_at..], b"endobj");
        let dict_end = match (stream_at, endobj_at) {
            (Some(s), Some(e)) => s.min(e),
            (Some(s), None) => s,
            (None, Some(e)) => e,
            (None, None) => break,
        };
        let dict = std::str::from_utf8(&pdf[body_at..body_at + dict_end]).unwrap_or("");
        let stream = (stream_at == Some(dict_end)).then(|| {
            let mut data = body_at + dict_end + b"stream".len();
            if pdf.get(data) == Some(&b'\r') {
                data += 1;
            }
            if pdf.get(data) == Some(&b'\n') {
                data += 1;
            }
            // `/Length` bounds the stream exactly, which matters because binary stream data can
            // contain `endstream`. An indirect length (`/Length 9 0 R`) is not resolved — the
            // keyword search is the fallback.
            let end = match dict_int(dict, "Length") {
                Some(len)
                    if dict_ref(dict, "Length").is_none() && data + len as usize <= pdf.len() =>
                {
                    data + len as usize
                }
                _ => find_token(&pdf[data..], b"endstream").map_or(pdf.len(), |e| data + e),
            };
            i = end;
            &pdf[data..end]
        });
        out.insert(num, Object { dict, stream });
    }
    out
}

/// The page objects, in document order: `Catalog → /Pages → /Kids`, descending into a nested page
/// tree. Falls back to every `/Type /Page` object when there is no catalog to walk.
fn page_objects(objs: &BTreeMap<u32, Object<'_>>) -> Vec<u32> {
    let root = objs
        .iter()
        .find(|(_, o)| dict_name(o.dict, "Type") == Some("Catalog"))
        .and_then(|(_, o)| dict_ref(o.dict, "Pages"));
    let mut out = Vec::new();
    match root {
        Some(root) => collect_kids(objs, root, &mut out, 0),
        None => out.extend(
            objs.iter()
                .filter(|(_, o)| dict_name(o.dict, "Type") == Some("Page"))
                .map(|(n, _)| *n),
        ),
    }
    out
}

/// Append the leaf pages under the page-tree node `num`. `depth` bounds the recursion so a malformed
/// (cyclic) tree cannot hang a test.
fn collect_kids(objs: &BTreeMap<u32, Object<'_>>, num: u32, out: &mut Vec<u32>, depth: u32) {
    if depth > 8 {
        return;
    }
    let Some(node) = objs.get(&num) else {
        return;
    };
    if dict_name(node.dict, "Type") == Some("Page") {
        out.push(num);
        return;
    }
    for kid in refs_in(dict_array(node.dict, "Kids").unwrap_or("")) {
        collect_kids(objs, kid, out, depth + 1);
    }
}

// ---------------------------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------------------------

/// A page's `/Resources` dictionary text, whether the page carries it inline (`/Resources << … >>`)
/// or points at a shared one (`/Resources 2 0 R`). A writer that deduplicates identical resource
/// dictionaries across pages emits the reference form, so both have to resolve.
fn page_resources<'a>(objs: &BTreeMap<u32, Object<'a>>, page: &Object<'a>) -> &'a str {
    nested_dict(page.dict, "Resources")
        .or_else(|| {
            dict_ref(page.dict, "Resources")
                .and_then(|num| objs.get(&num))
                .map(|obj| obj.dict)
        })
        .unwrap_or("")
}

/// Resolve a page's `/Font` resource map to `resource name → font`, following a composite font to its
/// descendant CIDFont for the width table and descriptor.
fn page_fonts(objs: &BTreeMap<u32, Object<'_>>, resources: &str) -> BTreeMap<String, PdfFont> {
    let mut out = BTreeMap::new();
    for (name, num) in named_refs(nested_dict(resources, "Font").unwrap_or("")) {
        let Some(obj) = objs.get(&num) else { continue };
        let base_font = dict_name(obj.dict, "BaseFont").unwrap_or("?");
        let to_unicode = dict_ref(obj.dict, "ToUnicode")
            .and_then(|n| objs.get(&n))
            .and_then(stream_bytes)
            .map(|bytes| parse_to_unicode(&String::from_utf8_lossy(&bytes)))
            .unwrap_or_default();
        // A composite font keeps its metrics on the descendant CIDFont; a simple font on itself.
        let cid_object = refs_in(dict_array(obj.dict, "DescendantFonts").unwrap_or(""))
            .first()
            .copied();
        let metrics = cid_object.and_then(|n| objs.get(&n)).unwrap_or(obj);
        let descriptor = dict_ref(metrics.dict, "FontDescriptor").and_then(|n| objs.get(&n));
        out.insert(
            name,
            PdfFont {
                object: num,
                base_font: base_font.to_string(),
                face: base_font
                    .rsplit('+')
                    .next()
                    .unwrap_or(base_font)
                    .to_string(),
                subtype: dict_name(obj.dict, "Subtype").unwrap_or("?").to_string(),
                cid_object,
                has_descriptor: descriptor.is_some(),
                has_font_program: descriptor.is_some_and(|d| {
                    ["FontFile", "FontFile2", "FontFile3"]
                        .iter()
                        .any(|k| value_of(d.dict, k).is_some())
                }),
                widths: glyph_widths(objs, metrics),
                default_width: dict_num(metrics.dict, "DW").unwrap_or(1000.0),
                to_unicode,
                two_byte: dict_name(obj.dict, "Subtype") == Some("Type0"),
            },
        );
    }
    out
}

/// A font's declared glyph widths in 1000-unit em space, from a CIDFont's `/W` or a simple font's
/// `/Widths` + `/FirstChar`.
///
/// `/W` mixes two forms — `c_first c_last w` gives one width to a whole code range, `c [w …]` gives
/// consecutive codes their own — and both appear in real output, so both are read.
fn glyph_widths(objs: &BTreeMap<u32, Object<'_>>, font: &Object<'_>) -> BTreeMap<u32, f64> {
    let mut out = BTreeMap::new();
    if let Some(body) = array_value(objs, font.dict, "W") {
        let spaced = body.replace('[', " [ ").replace(']', " ] ");
        let tokens: Vec<&str> = spaced.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            let Ok(first) = tokens[i].parse::<u32>() else {
                i += 1;
                continue;
            };
            if tokens.get(i + 1) == Some(&"[") {
                let mut code = first;
                i += 2;
                while let Some(tok) = tokens.get(i) {
                    i += 1;
                    if *tok == "]" {
                        break;
                    }
                    if let Ok(w) = tok.parse::<f64>() {
                        out.insert(code, w);
                        code += 1;
                    }
                }
                continue;
            }
            match (
                tokens.get(i + 1).and_then(|t| t.parse::<u32>().ok()),
                tokens.get(i + 2).and_then(|t| t.parse::<f64>().ok()),
            ) {
                (Some(last), Some(w)) if last >= first => {
                    for code in first..=last.min(first.saturating_add(0xffff)) {
                        out.insert(code, w);
                    }
                    i += 3;
                }
                _ => i += 1,
            }
        }
        return out;
    }
    let first = dict_int(font.dict, "FirstChar").unwrap_or(0) as u32;
    for (i, w) in array_value(objs, font.dict, "Widths")
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|t| t.parse::<f64>().ok())
        .enumerate()
    {
        out.insert(first + i as u32, w);
    }
    out
}

/// Describe a page's `/XObject` image resources — enough that a picture silently dropped, re-encoded
/// or resampled is visible, without putting pixel bytes in a test.
fn page_images(objs: &BTreeMap<u32, Object<'_>>, resources: &str) -> BTreeMap<String, PdfImage> {
    let mut out = BTreeMap::new();
    for (name, num) in named_refs(nested_dict(resources, "XObject").unwrap_or("")) {
        let Some(obj) = objs.get(&num) else { continue };
        let filter = dict_name(obj.dict, "Filter")
            .map(str::to_string)
            .or_else(|| {
                dict_array(obj.dict, "Filter").map(|a| a.replace('/', " ").trim().to_string())
            })
            .unwrap_or_else(|| "none".to_string());
        out.insert(
            name,
            PdfImage {
                object: num,
                subtype: dict_name(obj.dict, "Subtype").unwrap_or("?").to_string(),
                filter,
                width: dict_int(obj.dict, "Width").unwrap_or(-1),
                height: dict_int(obj.dict, "Height").unwrap_or(-1),
                color_space: dict_name(obj.dict, "ColorSpace").unwrap_or("?").to_string(),
            },
        );
    }
    out
}

/// Parse a `/ToUnicode` CMap's `bfchar`/`bfrange` sections into `glyph code → text`.
fn parse_to_unicode(cmap: &str) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    for section in cmap.split("beginbfchar").skip(1) {
        let body = section.split("endbfchar").next().unwrap_or("");
        let mut items = hex_items(body);
        while let (Some(src), Some(dst)) = (items.next(), items.next()) {
            if let (Some(code), Some(text)) = (hex_u32(&src), utf16be(&dst)) {
                out.insert(code, text);
            }
        }
    }
    for section in cmap.split("beginbfrange").skip(1) {
        let body = section.split("endbfrange").next().unwrap_or("");
        let mut items = hex_items(body);
        while let (Some(lo), Some(hi), Some(dst)) = (items.next(), items.next(), items.next()) {
            let (Some(lo), Some(hi)) = (hex_u32(&lo), hex_u32(&hi)) else {
                continue;
            };
            // `<lo> <hi> <dst>` maps the range consecutively from `dst`; the array form
            // `<lo> <hi> [<d0> <d1> …]` is not emitted by the writer under test and is skipped.
            let Some(text) = utf16be(&dst) else { continue };
            let base = text.chars().next().unwrap_or('\u{fffd}') as u32;
            for (i, code) in (lo..=hi.min(lo.saturating_add(0xffff))).enumerate() {
                if let Some(c) = char::from_u32(base + i as u32) {
                    out.insert(code, c.to_string());
                }
            }
        }
    }
    out
}

/// The `<…>` hex strings in `body`, in order.
fn hex_items(body: &str) -> impl Iterator<Item = String> + '_ {
    body.split('<')
        .skip(1)
        .filter_map(|s| s.split('>').next().map(|s| s.trim().to_string()))
}

fn hex_u32(hex: &str) -> Option<u32> {
    u32::from_str_radix(hex, 16).ok()
}

/// Decode a UTF-16BE hex string (a CMap destination) to text.
fn utf16be(hex: &str) -> Option<String> {
    let units: Vec<u16> = hex
        .as_bytes()
        .chunks(4)
        .filter(|c| c.len() == 4)
        .filter_map(|c| u16::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
        .collect();
    (!units.is_empty()).then(|| String::from_utf16_lossy(&units))
}

// ---------------------------------------------------------------------------------------------
// Content stream normalization
// ---------------------------------------------------------------------------------------------

/// One operand of a content-stream operator.
enum Operand {
    Num(f64),
    /// A PDF string, still in wire form (glyph codes for a text-showing operator).
    Str(Vec<u8>),
    Name(String),
    /// Anything passed through verbatim: `[`, `]`, `<<`, `>>`, `true`/`false`/`null`.
    Raw(String),
}

/// Re-emit `content` as one operator per line, with numbers rounded and glyph strings decoded through
/// the font each `Tf` selects.
fn normalize_content(content: &[u8], fonts: &BTreeMap<String, PdfFont>) -> String {
    let mut out = String::new();
    let mut operands: Vec<Operand> = Vec::new();
    let mut font: Option<&PdfFont> = None;
    let mut lex = Lexer { s: content, i: 0 };
    while let Some(tok) = lex.next_token() {
        match tok {
            Token::Operand(op) => operands.push(op),
            Token::Operator(op) => {
                if op == "Tf" {
                    font = operands
                        .iter()
                        .find_map(|o| match o {
                            Operand::Name(n) => Some(n),
                            _ => None,
                        })
                        .and_then(|n| fonts.get(n));
                }
                let shows_text = matches!(op.as_str(), "Tj" | "TJ" | "'" | "\"");
                for operand in &operands {
                    match operand {
                        Operand::Num(n) => out.push_str(&fmt_num(*n)),
                        Operand::Name(n) => {
                            out.push('/');
                            out.push_str(n);
                        }
                        Operand::Raw(r) => out.push_str(r),
                        Operand::Str(bytes) => {
                            out.push('(');
                            out.push_str(&match (shows_text, font) {
                                (true, Some(f)) => decode_glyphs(bytes, f),
                                _ => escape(&String::from_utf8_lossy(bytes)),
                            });
                            out.push(')');
                        }
                    }
                    out.push(' ');
                }
                out.push_str(&op);
                out.push('\n');
                operands.clear();
            }
        }
    }
    out
}

/// Walk `content` and report every text-showing operator with the two advances a reader can compute
/// for it: what the selected font's width table prices the glyphs at, and where the pen lands once the
/// `TJ` position adjustments are applied.
///
/// Character spacing (`Tc`) and horizontal scaling (`Tz`) are tracked because they scale that advance.
/// Word spacing (`Tw`) is not: it applies only to single-byte code 32, which a `/Identity-H` composite
/// font never emits.
fn text_shows(content: &[u8], fonts: &BTreeMap<String, PdfFont>) -> Vec<PdfTextShow> {
    let mut out = Vec::new();
    let mut operands: Vec<Operand> = Vec::new();
    let mut selected: Option<(String, f64)> = None;
    let mut char_spacing = 0.0f64;
    let mut h_scale = 1.0f64;
    let mut lex = Lexer { s: content, i: 0 };
    while let Some(tok) = lex.next_token() {
        match tok {
            Token::Operand(op) => operands.push(op),
            Token::Operator(op) => {
                match op.as_str() {
                    "Tf" => {
                        let name = operands.iter().find_map(|o| match o {
                            Operand::Name(n) => Some(n.clone()),
                            _ => None,
                        });
                        let size = operands.iter().rev().find_map(|o| match o {
                            Operand::Num(n) => Some(*n),
                            _ => None,
                        });
                        selected = name.zip(size);
                    }
                    "Tc" => char_spacing = numeric(&operands).unwrap_or(0.0),
                    "Tz" => h_scale = numeric(&operands).unwrap_or(100.0) / 100.0,
                    "Tj" | "TJ" | "'" | "\"" => {
                        if let Some((font, (name, size))) = selected
                            .clone()
                            .and_then(|sel| fonts.get(&sel.0).map(|f| (f, sel)))
                        {
                            let mut show = PdfTextShow {
                                font: name,
                                size,
                                ..PdfTextShow::default()
                            };
                            let mut declared = 0.0f64;
                            let mut adjustments = 0.0f64;
                            for operand in &operands {
                                match operand {
                                    Operand::Str(bytes) => {
                                        for code in glyph_codes(bytes, font) {
                                            declared += font.width(code);
                                            show.text.push_str(
                                                font.to_unicode
                                                    .get(&code)
                                                    .map_or("", String::as_str),
                                            );
                                            show.glyphs.push(code);
                                        }
                                    }
                                    // A number inside a `TJ` array moves the pen back by n/1000 em,
                                    // between the glyph before it and the one after.
                                    Operand::Num(n) => {
                                        adjustments += n;
                                        show.kerns.push((show.glyphs.len(), *n));
                                    }
                                    _ => {}
                                }
                            }
                            let scale = |em: f64| {
                                (em / 1000.0 * size + char_spacing * show.glyphs.len() as f64)
                                    * h_scale
                            };
                            show.declared_pt = scale(declared);
                            show.pen_pt = scale(declared - adjustments);
                            out.push(show);
                        }
                    }
                    _ => {}
                }
                operands.clear();
            }
        }
    }
    out
}

/// The first numeric operand, for the single-operand text-state operators.
fn numeric(operands: &[Operand]) -> Option<f64> {
    operands.iter().find_map(|o| match o {
        Operand::Num(n) => Some(*n),
        _ => None,
    })
}

/// Split a text-showing operand's bytes into glyph codes, two bytes per code for a composite font.
fn glyph_codes(bytes: &[u8], font: &PdfFont) -> Vec<u32> {
    if font.two_byte {
        bytes
            .chunks(2)
            .map(|c| {
                let hi = u32::from(c[0]);
                c.get(1).map_or(hi, |lo| (hi << 8) | u32::from(*lo))
            })
            .collect()
    } else {
        bytes.iter().map(|b| u32::from(*b)).collect()
    }
}

/// Turn a text-showing operand's glyph codes back into the text they stand for. An unmapped code is
/// reported as `<cid:N>` rather than silently dropped — a missing `/ToUnicode` entry is a real defect.
fn decode_glyphs(bytes: &[u8], font: &PdfFont) -> String {
    let mut out = String::new();
    for code in glyph_codes(bytes, font) {
        match font.to_unicode.get(&code) {
            Some(text) => out.push_str(&escape(text)),
            None => out.push_str(&format!("<cid:{code}>")),
        }
    }
    out
}

/// Escape a decoded string for the listing: PDF string delimiters, and anything outside printable
/// ASCII as `\u{…}` so the baseline stays plain ASCII.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            _ => out.push_str(&format!("\\u{{{:04x}}}", c as u32)),
        }
    }
    out
}

/// Format a number rounded to 3 decimals, trailing zeros trimmed, with no `-0`.
fn fmt_num(n: f64) -> String {
    let rounded = (n * 1000.0).round() / 1000.0;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let mut s = format!("{rounded:.3}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

/// A content-stream token: an operand, or the operator that consumes the operands before it.
enum Token {
    Operand(Operand),
    Operator(String),
}

/// A PDF content-stream lexer — enough of one to split operands from operators and keep string
/// contents intact.
struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn next_token(&mut self) -> Option<Token> {
        self.skip_space();
        let b = *self.s.get(self.i)?;
        match b {
            b'(' => {
                self.i += 1;
                Some(Token::Operand(Operand::Str(self.literal_string())))
            }
            b'<' => {
                if self.s.get(self.i + 1) == Some(&b'<') {
                    self.i += 2;
                    return Some(Token::Operand(Operand::Raw("<<".into())));
                }
                self.i += 1;
                Some(Token::Operand(Operand::Str(self.hex_string())))
            }
            b'>' => {
                self.i += if self.s.get(self.i + 1) == Some(&b'>') {
                    2
                } else {
                    1
                };
                Some(Token::Operand(Operand::Raw(">>".into())))
            }
            b'[' | b']' | b'{' | b'}' => {
                self.i += 1;
                Some(Token::Operand(Operand::Raw((b as char).to_string())))
            }
            b'/' => {
                self.i += 1;
                let word = self.word();
                Some(Token::Operand(Operand::Name(word)))
            }
            b'%' => {
                while self.i < self.s.len() && !matches!(self.s[self.i], b'\n' | b'\r') {
                    self.i += 1;
                }
                self.next_token()
            }
            _ => {
                let word = self.word();
                if word.is_empty() {
                    self.i += 1;
                    return self.next_token();
                }
                if let Ok(n) = word.parse::<f64>() {
                    return Some(Token::Operand(Operand::Num(n)));
                }
                if matches!(word.as_str(), "true" | "false" | "null") {
                    return Some(Token::Operand(Operand::Raw(word)));
                }
                Some(Token::Operator(word))
            }
        }
    }

    fn skip_space(&mut self) {
        while self
            .s
            .get(self.i)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t' | b'\0' | 0x0c))
        {
            self.i += 1;
        }
    }

    /// A regular token: bytes up to the next whitespace or delimiter.
    fn word(&mut self) -> String {
        let start = self.i;
        while self.s.get(self.i).is_some_and(|b| {
            !matches!(
                b,
                b' ' | b'\n'
                    | b'\r'
                    | b'\t'
                    | b'\0'
                    | 0x0c
                    | b'('
                    | b')'
                    | b'<'
                    | b'>'
                    | b'['
                    | b']'
                    | b'{'
                    | b'}'
                    | b'/'
                    | b'%'
            )
        }) {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    /// The bytes of a `(…)` literal string, un-escaped. `self.i` is just past the opening paren.
    fn literal_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(&b) = self.s.get(self.i) {
            self.i += 1;
            match b {
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { break };
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'\n' => {}
                        b'\r' => {
                            if self.s.get(self.i) == Some(&b'\n') {
                                self.i += 1;
                            }
                        }
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.s.get(self.i) {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(*d - b'0');
                                        self.i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                _ => out.push(b),
            }
        }
        out
    }

    /// The bytes of a `<…>` hex string. `self.i` is just past the opening angle bracket.
    fn hex_string(&mut self) -> Vec<u8> {
        let start = self.i;
        while self.s.get(self.i).is_some_and(|b| *b != b'>') {
            self.i += 1;
        }
        let hex: String = String::from_utf8_lossy(&self.s[start..self.i])
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect();
        self.i += 1; // the closing `>`
        hex.as_bytes()
            .chunks(2)
            .filter_map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Dictionary and stream primitives
// ---------------------------------------------------------------------------------------------

/// Inflate an object's stream when it is `/FlateDecode`.
fn inflate(obj: &Object<'_>) -> Option<Vec<u8>> {
    let data = obj.stream?;
    if !obj.dict.contains("FlateDecode") {
        return None;
    }
    miniz_oxide::inflate::decompress_to_vec_zlib(data).ok()
}

/// An object's stream data: inflated when it is `/FlateDecode`, verbatim when it is stored plain (a
/// `/ToUnicode` CMap is, and an uncompressed writer's content stream would be).
fn stream_bytes(obj: &Object<'_>) -> Option<Vec<u8>> {
    inflate(obj).or_else(|| obj.stream.map(<[u8]>::to_vec))
}

/// `/Key /Value` — the name value of `key`, without its slash.
fn dict_name<'a>(dict: &'a str, key: &str) -> Option<&'a str> {
    let rest = value_of(dict, key)?.trim_start();
    let rest = rest.strip_prefix('/')?;
    let end = rest
        .find(|c: char| c.is_whitespace() || "/<>[]()".contains(c))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// `/Key 42` — the integer value of `key`.
fn dict_int(dict: &str, key: &str) -> Option<i64> {
    let rest = value_of(dict, key)?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// `/Key 12 0 R` — the object number `key` refers to.
fn dict_ref(dict: &str, key: &str) -> Option<u32> {
    let rest = value_of(dict, key)?.trim_start();
    let mut parts = rest.split_whitespace();
    let num: u32 = parts.next()?.parse().ok()?;
    (parts.next() == Some("0") && is_ref_keyword(parts.next()?)).then_some(num)
}

/// Whether `token` is the `R` closing an indirect reference. The `R` may be butted straight up
/// against the delimiter that ends the value (`/Contents 10 0 R>>`), since the whitespace before it
/// is optional and a writer minimizing its output omits it.
fn is_ref_keyword(token: &str) -> bool {
    token.strip_prefix('R').is_some_and(|tail| {
        tail.is_empty() || tail.starts_with(['/', '<', '>', '[', ']', '(', ')'])
    })
}

/// `/Key [ … ]` — the contents of `key`'s array, brackets excluded. Stops at the first `]`, so it is
/// for the flat arrays (`/MediaBox`, `/Kids`); see [`array_value`] for one that may nest.
fn dict_array<'a>(dict: &'a str, key: &str) -> Option<&'a str> {
    let rest = value_of(dict, key)?.trim_start().strip_prefix('[')?;
    Some(&rest[..rest.find(']')?])
}

/// `/Key 1.5` — the numeric value of `key`.
fn dict_num(dict: &str, key: &str) -> Option<f64> {
    let rest = value_of(dict, key)?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && !matches!(c, '-' | '+' | '.'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The contents of `key`'s array, whether it is written inline or reached through an indirect
/// reference (`/W 12 0 R`), with nested arrays kept intact — a CIDFont's `/W` mixes bare widths with
/// `[ … ]` groups, so the brackets have to be balanced rather than cut at the first one.
fn array_value(objs: &BTreeMap<u32, Object<'_>>, dict: &str, key: &str) -> Option<String> {
    if let Some(num) = dict_ref(dict, key) {
        return balanced_array(objs.get(&num)?.dict).map(str::to_string);
    }
    balanced_array(value_of(dict, key)?).map(str::to_string)
}

/// The body of the first `[ … ]` in `text`, brackets balanced and excluded.
fn balanced_array(text: &str) -> Option<&str> {
    let open = text.find('[')?;
    let body = &text[open + 1..];
    let mut depth = 1usize;
    for (i, c) in body.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&body[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// `/Key << … >>` — the contents of `key`'s sub-dictionary, delimiters excluded.
fn nested_dict<'a>(dict: &'a str, key: &str) -> Option<&'a str> {
    let rest = value_of(dict, key)?.trim_start().strip_prefix("<<")?;
    let bytes = rest.as_bytes();
    let mut depth = 1usize;
    let mut i = 0;
    while i + 1 < bytes.len() {
        match (bytes[i], bytes[i + 1]) {
            (b'<', b'<') => {
                depth += 1;
                i += 2;
            }
            (b'>', b'>') => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rest[..i]);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    None
}

/// The text following `/key` in `dict`, matched on a whole name (so `/Length` does not match
/// `/Length1`).
fn value_of<'a>(dict: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("/{key}");
    let mut from = 0;
    while let Some(rel) = dict[from..].find(&needle) {
        let at = from + rel;
        let rest = &dict[at + needle.len()..];
        let terminated = rest
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || "/<>[]()".contains(c));
        if terminated {
            return Some(rest);
        }
        from = at + needle.len();
    }
    None
}

/// The object numbers in an array of references (`12 0 R 14 0 R …`), in order.
fn refs_in(array: &str) -> Vec<u32> {
    let parts: Vec<&str> = array.split_whitespace().collect();
    parts
        .windows(3)
        .filter(|w| w[1] == "0" && is_ref_keyword(w[2]))
        .filter_map(|w| w[0].parse().ok())
        .collect()
}

/// A `/name 12 0 R` mapping dictionary (a `/Font` or `/XObject` resource map), as `name → object`.
fn named_refs(dict: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for entry in dict.split('/').skip(1) {
        let mut parts = entry.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let Some(Ok(num)) = parts.next().map(str::parse::<u32>) else {
            continue;
        };
        if parts.next() == Some("0") && parts.next().is_some_and(is_ref_keyword) {
            out.insert(name.to_string(), num);
        }
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Find `needle` as a standalone keyword: not preceded by a regular character, and followed by
/// whitespace or end of input. This is what keeps a search for `stream` off `endstream`.
fn find_token(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = find(&hay[from..], needle) {
        let at = from + rel;
        let before_ok = at == 0 || !hay[at - 1].is_ascii_alphanumeric();
        let after = hay.get(at + needle.len());
        let after_ok = after.is_none_or(|b| !b.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal one-page PDF with an uncompressed content stream, built by hand so the parser is
    /// tested against a document it did not produce.
    fn tiny_pdf() -> Vec<u8> {
        let content = b"1 0 0 -1 0 792.000004 cm\n0 0 100 -0 re\nf\n";
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.extend_from_slice(b"\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"\n2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n");
        pdf.extend_from_slice(
            b"\n3 0 obj\n<< /Type /Page /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
        );
        pdf.extend_from_slice(
            format!("\n4 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        pdf.extend_from_slice(content);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        pdf
    }

    #[test]
    fn the_listing_walks_the_page_tree_and_rounds_numbers() {
        let listing = operator_listing(&tiny_pdf());
        assert!(listing.starts_with("% 1 page(s)\n"), "{listing}");
        assert!(
            listing.contains("% page 1 media [0 0 612 792]"),
            "{listing}"
        );
        // 792.000004 is f32 noise around 792, and `-0` is not a distinct number.
        assert!(listing.contains("1 0 0 -1 0 792 cm\n"), "{listing}");
        assert!(listing.contains("0 0 100 0 re\n"), "{listing}");
    }

    #[test]
    fn a_stream_keyword_is_matched_as_a_token() {
        // `endstream` and a `/Length`-style dictionary key both contain the keyword; neither may open
        // a stream. If they did, this object's dictionary would be truncated and its media box lost.
        let mut pdf = tiny_pdf();
        pdf.extend_from_slice(
            b"\n5 0 obj\n<< /Type /Page /MediaBox [0 0 1 1] /Streamish /endstream >>\nendobj\n",
        );
        let objs = objects(&pdf);
        assert_eq!(objs.len(), 5);
        assert!(objs[&5].stream.is_none());
        assert_eq!(dict_array(objs[&5].dict, "MediaBox"), Some("0 0 1 1"));
    }

    #[test]
    fn a_named_key_is_not_matched_by_a_prefix() {
        // /Length1 is a distinct key of a font-program stream dictionary.
        let dict = "<< /Length1 400 /Length 12 >>";
        assert_eq!(dict_int(dict, "Length"), Some(12));
        assert_eq!(dict_int(dict, "Length1"), Some(400));
    }

    #[test]
    fn a_cmap_decodes_both_section_forms() {
        let cmap = "\
            2 beginbfchar\n<0001> <0041>\n<0002> <0042>\nendbfchar\n\
            1 beginbfrange\n<0010> <0012> <0061>\nendbfrange\n";
        let map = parse_to_unicode(cmap);
        assert_eq!(map[&1], "A");
        assert_eq!(map[&2], "B");
        assert_eq!(map[&0x10], "a");
        assert_eq!(map[&0x12], "c");
    }

    #[test]
    fn glyph_codes_decode_through_the_font_and_an_unmapped_one_is_named() {
        let font = PdfFont {
            face: "LiberationSans".into(),
            to_unicode: [(1, "H".to_string()), (2, "i".to_string())]
                .into_iter()
                .collect(),
            two_byte: true,
            ..PdfFont::default()
        };
        assert_eq!(decode_glyphs(&[0, 1, 0, 2, 0, 9], &font), "Hi<cid:9>");
    }

    #[test]
    fn a_cid_width_array_reads_both_of_its_forms() {
        // `c_first c_last w` covers a range; `c [w …]` gives consecutive codes their own width.
        let font = Object {
            dict: "<< /DW 0 /W [0 0 750 1 2 500 5 [600 700]] >>",
            stream: None,
        };
        let widths = glyph_widths(&BTreeMap::new(), &font);
        assert_eq!(widths[&0], 750.0);
        assert_eq!(widths[&1], 500.0);
        assert_eq!(widths[&2], 500.0);
        assert_eq!(widths[&5], 600.0);
        assert_eq!(widths[&6], 700.0);
        assert_eq!(widths.len(), 5);
    }

    #[test]
    fn a_simple_fonts_widths_are_keyed_from_first_char() {
        let font = Object {
            dict: "<< /FirstChar 65 /Widths [700 800] >>",
            stream: None,
        };
        let widths = glyph_widths(&BTreeMap::new(), &font);
        assert_eq!(widths[&65], 700.0);
        assert_eq!(widths[&66], 800.0);
    }

    /// The two advances: the widths the font declares, and the pen once `TJ` adjustments apply.
    #[test]
    fn a_text_show_reports_both_the_declared_width_and_the_pen() {
        let fonts: BTreeMap<String, PdfFont> = [(
            "f0".to_string(),
            PdfFont {
                widths: [(1, 500.0), (2, 250.0)].into_iter().collect(),
                to_unicode: [(1, "A".to_string()), (2, "B".to_string())]
                    .into_iter()
                    .collect(),
                two_byte: true,
                ..PdfFont::default()
            },
        )]
        .into_iter()
        .collect();
        let shows = text_shows(b"BT /f0 10 Tf [ <0001> -100 <0002> ] TJ ET", &fonts);
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].text, "AB");
        assert_eq!(shows[0].size, 10.0);
        // Declared: (500 + 250)/1000 * 10pt. Pen: the -100 adjustment moves it 1pt further right.
        assert!((shows[0].declared_pt - 7.5).abs() < 1e-9, "{:?}", shows[0]);
        assert!((shows[0].pen_pt - 8.5).abs() < 1e-9, "{:?}", shows[0]);
    }
}
