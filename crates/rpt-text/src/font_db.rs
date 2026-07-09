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

use fontdb::{Database, Family, Query, Stretch, Style, Weight, ID};
use rpt_pages::FontSpec;

/// An OS font database plus the shared [`FontSpec`] → face resolution policy. Load it once
/// ([`FontDb::with_system_fonts`]) and resolve many specs; a backend keeps only its own parse+cache
/// of the resolved face bytes.
pub struct FontDb {
    db: Database,
}

impl std::fmt::Debug for FontDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontDb")
            .field("faces", &self.db.len())
            .finish()
    }
}

impl FontDb {
    /// A database loaded with the OS-installed fonts (native). This is the scan the backends want to
    /// do once rather than per render.
    pub fn with_system_fonts() -> FontDb {
        let mut db = Database::new();
        db.load_system_fonts();
        FontDb { db }
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
}
