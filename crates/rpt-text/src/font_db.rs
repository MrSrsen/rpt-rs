//! [`FontDb`] — the shared face-resolution policy for the physical render backends.
//!
//! The PDF and raster backends both need the same thing: locate an OS face for a [`FontSpec`]'s
//! family (with bold/italic), then hand its bytes to a parser (krilla's `Font`, fontdue's `Font`).
//! They differ only in that parse step. This type owns the [`fontdb`] database and the resolution
//! policy — the [`fontdb::Query`] built from a `FontSpec` (named family, generic sans-serif fallback;
//! weight from `bold`, style from `italic`) — so the policy lives in one place instead of being
//! re-implemented per backend.
//!
//! This module is dependency-light (just `fontdb`) and always compiled, independent of the
//! cosmic-text feature — so a backend depends on `rpt-text` with `default-features = false` and pulls
//! only `fontdb`, not the whole shaping stack.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use fontdb::{Database, Family, Query, Stretch, Style, Weight, ID};
use rpt_pages::FontSpec;

/// A maximal run of `text` that a single face covers: the byte `range` (a slice of the original
/// string) drawn with `face`. `substituted` marks a run served by the bundled symbol fallback
/// because the requested family lacked those glyphs — the signal behind `FontSubstituted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceRun {
    /// The face that covers every char in this run.
    pub face: ID,
    /// Byte range of this run within the segmented text.
    pub range: Range<usize>,
    /// True when `face` is the symbol fallback (the primary family lacked these glyphs).
    pub substituted: bool,
}

/// An OS font database plus the shared [`FontSpec`] → face resolution policy. Load it once
/// ([`FontDb::with_system_fonts`]) and resolve many specs; a backend keeps only its own parse+cache
/// of the resolved face bytes.
pub struct FontDb {
    db: Database,
    /// Memoized family resolution: `(family, bold, italic)` → resolved primary face id (`None` = no
    /// match). [`segment_by_coverage`](FontDb::segment_by_coverage) runs once per text op and the
    /// underlying fontdb family query is a linear scan over every installed face, so caching keeps it
    /// to one query per distinct font spec — the difference between a fast render and a slow one.
    primary_cache: RefCell<HashMap<(String, bool, bool), Option<ID>>>,
    /// The bundled symbol fallback id, resolved lazily once (its query never varies).
    symbol_cache: RefCell<Option<Option<ID>>>,
}

impl std::fmt::Debug for FontDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontDb")
            .field("faces", &self.db.len())
            .finish()
    }
}

impl FontDb {
    /// A database loaded with the OS-installed fonts (native) plus the always-present bundled
    /// Liberation fallback. This is the scan the backends want to do once rather than per render.
    /// The bundled fallback is registered last (lowest priority) so a named-but-absent family
    /// resolves deterministically to a metric-compatible face instead of falling to [`first_face`].
    ///
    /// [`first_face`]: FontDb::first_face
    pub fn with_system_fonts() -> FontDb {
        let mut db = Database::new();
        db.load_system_fonts();
        crate::bundled::register_fallback(&mut db);
        FontDb::from_db(db)
    }

    /// A database with **only** the bundled Liberation fallback — no OS scan. The deterministic,
    /// dependency-free path for headless/WASM hosts: every named family resolves through the generic
    /// sans-serif default to a metric-compatible bundled face, so [`query`] never returns `None` for
    /// lack of a system library.
    ///
    /// [`query`]: FontDb::query
    pub fn bundled() -> FontDb {
        let mut db = Database::new();
        crate::bundled::register_fallback(&mut db);
        FontDb::from_db(db)
    }

    /// Wrap a loaded [`Database`] with empty resolution caches.
    fn from_db(db: Database) -> FontDb {
        FontDb {
            db,
            primary_cache: RefCell::new(HashMap::new()),
            symbol_cache: RefCell::new(None),
        }
    }

    /// The resolved primary face for `spec`, memoized by `(family, bold, italic)` so the fontdb
    /// family scan runs once per distinct spec rather than once per text op. Same result as
    /// [`query`](FontDb::query).
    fn primary_for(&self, spec: &FontSpec) -> Option<ID> {
        let key = (spec.family.clone(), spec.bold, spec.italic);
        if let Some(&id) = self.primary_cache.borrow().get(&key) {
            return id;
        }
        let id = self.query(spec);
        self.primary_cache.borrow_mut().insert(key, id);
        id
    }

    /// The bundled symbol fallback id, resolved once and cached ([`symbol_face`](FontDb::symbol_face)
    /// otherwise re-runs its query on every non-ASCII run).
    fn symbol_cached(&self) -> Option<ID> {
        if let Some(id) = *self.symbol_cache.borrow() {
            return id;
        }
        let id = self.symbol_face();
        *self.symbol_cache.borrow_mut() = Some(id);
        id
    }

    /// Resolve a [`FontSpec`] to a face id via the shared query: the named family first, then the
    /// generic sans-serif fallback; weight from `bold`, style from `italic`, stretch normal. `None`
    /// when nothing matches at all.
    pub fn query(&self, spec: &FontSpec) -> Option<ID> {
        let query = Query {
            families: &[Family::Name(&spec.family), Family::SansSerif],
            weight: if spec.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            },
            stretch: Stretch::Normal,
            style: if spec.italic {
                Style::Italic
            } else {
                Style::Normal
            },
        };
        self.db.query(&query)
    }

    /// Run `f` over a resolved face's raw bytes and face index (for the backend's own parser). `None`
    /// if the id is unknown or its data can't be read.
    pub fn with_face_data<T>(&self, id: ID, f: impl FnOnce(&[u8], u32) -> T) -> Option<T> {
        self.db.with_face_data(id, f)
    }

    /// The first available face id — a last-resort fallback when no family matches at all (the PDF
    /// backend uses this so a page never renders with zero usable fonts).
    pub fn first_face(&self) -> Option<ID> {
        self.db.faces().next().map(|f| f.id)
    }

    /// The bundled symbol fallback face (DejaVu Sans) — covers ⚠/✓/✗/arrows etc. that the text
    /// faces lack. Always present (registered by the `bundled` module); `None` only if that face
    /// somehow failed to load.
    pub fn symbol_face(&self) -> Option<ID> {
        self.db.query(&Query {
            families: &[Family::Name(crate::bundled::SYMBOL_FALLBACK_FAMILY)],
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        })
    }

    /// Whether `face` has a glyph for codepoint `c`. Reads the face's cmap; `false` if the face is
    /// unknown/unparseable. A control char (newline etc.) counts as covered — it is not drawn.
    pub fn face_covers(&self, face: ID, c: char) -> bool {
        if c.is_control() {
            return true;
        }
        self.with_face_data(face, |data, index| {
            ttf_parser::Face::parse(data, index)
                .ok()
                .and_then(|f| f.glyph_index(c))
                .is_some()
        })
        .unwrap_or(false)
    }

    /// Split `text` into maximal `FaceRun`s each covered by a single face: the resolved primary
    /// face for `spec` where it has the glyph, else the bundled symbol fallback for the runs it
    /// lacks. Adjacent chars sharing a face coalesce into one run. Returns an empty vec for empty
    /// text; if no primary face resolves at all, the whole string maps to the symbol face (or, if
    /// even that is absent, to a single non-substituted run on [`first_face`] so callers still draw
    /// something).
    ///
    /// [`first_face`]: FontDb::first_face
    pub fn segment_by_coverage(&self, spec: &FontSpec, text: &str) -> Vec<FaceRun> {
        if text.is_empty() {
            return Vec::new();
        }
        let primary = self.primary_for(spec).or_else(|| self.first_face());
        // Fast path: pure-ASCII text is fully covered by any Latin text face, so skip the per-char
        // cmap probing entirely and emit one primary run. This is the overwhelming majority of text
        // ops (names, numbers, dates) — the coverage scan below is only for the rare non-ASCII run.
        if text.is_ascii() {
            if let Some(face) = primary {
                return vec![FaceRun {
                    face,
                    range: 0..text.len(),
                    substituted: false,
                }];
            }
        }
        // Non-ASCII: parse each candidate face *once* here and probe its cmap per char via
        // `glyph_index`, rather than re-parsing the whole face on every character (which turned
        // segmentation into the render's hot path). The primary is parsed in the outer closure, the
        // symbol fallback nested inside it, so both are live while we classify every char.
        let symbol = self.symbol_cached();
        let classify = |pface: Option<&ttf_parser::Face>, sface: Option<&ttf_parser::Face>| {
            let covers = |face: Option<&ttf_parser::Face>, c: char| {
                c.is_control() || face.and_then(|f| f.glyph_index(c)).is_some()
            };
            let mut runs: Vec<FaceRun> = Vec::new();
            for (offset, c) in text.char_indices() {
                // The primary if it covers this char, else the symbol fallback if that does, else
                // stay on the primary (draw its .notdef rather than lose the char's advance).
                let (face, substituted) = if primary.is_some() && covers(pface, c) {
                    (primary, false)
                } else if symbol.is_some() && covers(sface, c) {
                    (symbol, true)
                } else {
                    (primary, false)
                };
                let Some(face) = face else { continue };
                let end = offset + c.len_utf8();
                match runs.last_mut() {
                    Some(last) if last.face == face && last.substituted == substituted => {
                        last.range.end = end;
                    }
                    _ => runs.push(FaceRun {
                        face,
                        range: offset..end,
                        substituted,
                    }),
                }
            }
            runs
        };
        // Parse primary (outer) then symbol (inner) once each, then classify. `with_face_data`
        // returns `None` only if the id is unknown/unreadable — then classify with that face absent.
        let with_symbol = |pface: Option<&ttf_parser::Face>| match symbol {
            Some(sid) => self
                .with_face_data(sid, |data, idx| {
                    classify(pface, ttf_parser::Face::parse(data, idx).ok().as_ref())
                })
                .unwrap_or_else(|| classify(pface, None)),
            None => classify(pface, None),
        };
        match primary {
            Some(pid) => self
                .with_face_data(pid, |data, idx| {
                    with_symbol(ttf_parser::Face::parse(data, idx).ok().as_ref())
                })
                .unwrap_or_else(|| with_symbol(None)),
            None => with_symbol(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(family: &str) -> FontSpec {
        FontSpec {
            family: family.into(),
            ..FontSpec::default()
        }
    }

    #[test]
    fn bundled_resolves_unknown_family_deterministically() {
        // No OS scan, only the bundled fallback: an absent family still resolves (through the
        // sans-serif generic to Liberation Sans) rather than returning None → nondeterministic
        // first_face on headless/WASM hosts.
        let db = FontDb::bundled();
        let unknown = spec("Totally Absent Family 12345");
        let id = db.query(&unknown);
        assert!(
            id.is_some(),
            "unknown family must resolve via bundled fallback"
        );
        assert_eq!(id, db.query(&unknown), "resolution is deterministic");
    }

    #[test]
    fn symbol_fallback_covers_the_glyphs_the_text_faces_lack() {
        // The bundled symbol face resolves and covers ⚠ (U+26A0); the metric-compat Liberation text
        // face (what "Arial" resolves to) does not — which is exactly why segmentation falls back.
        let db = FontDb::bundled();
        let symbol = db.symbol_face().expect("symbol face registered");
        assert!(db.face_covers(symbol, '\u{26A0}'), "DejaVu covers ⚠");
        let arial = db.query(&spec("Arial")).expect("Arial → Liberation");
        assert!(db.face_covers(arial, 'A'), "text face covers ASCII");
        assert!(
            !db.face_covers(arial, '\u{26A0}'),
            "the Latin text face lacks ⚠ (drives the fallback)"
        );
    }

    #[test]
    fn segments_split_primary_and_symbol_fallback() {
        // "⚠ HAZ" in Arial: the ⚠ maps to the substituted symbol face, the rest to the primary. The
        // space is covered by the primary, so it joins the trailing run.
        let db = FontDb::bundled();
        let runs = db.segment_by_coverage(&spec("Arial"), "\u{26A0} HAZ");
        assert_eq!(runs.len(), 2, "one symbol run + one primary run: {runs:?}");
        assert!(runs[0].substituted, "leading ⚠ is substituted");
        assert_eq!(runs[0].range, 0..'\u{26A0}'.len_utf8());
        assert!(!runs[1].substituted, "' HAZ' is the primary face");
        assert_eq!(runs[0].face, db.symbol_face().unwrap());
        // The runs partition the string with no gaps or overlaps.
        assert_eq!(runs[0].range.end, runs[1].range.start);
        assert_eq!(runs[1].range.end, "\u{26A0} HAZ".len());
    }

    #[test]
    fn all_ascii_is_a_single_unsubstituted_run() {
        let db = FontDb::bundled();
        let runs = db.segment_by_coverage(&spec("Arial"), "HAZ");
        assert_eq!(runs.len(), 1);
        assert!(!runs[0].substituted);
        assert_eq!(runs[0].range, 0.."HAZ".len());
        assert!(db.segment_by_coverage(&spec("Arial"), "").is_empty());
    }

    #[test]
    fn bundled_resolves_the_crystal_default_families() {
        // The Crystal defaults are metric-compatible with the bundled Liberation faces.
        let db = FontDb::bundled();
        for family in ["Arial", "Times New Roman", "Courier New"] {
            assert!(
                db.query(&spec(family)).is_some(),
                "{family} must resolve via the bundled fallback"
            );
        }
    }
}
