//! Per-report dedup tables: typography (`fc`), adornment/border (`ad`), and embedded-image (`im`)
//! classes, each interned once and referenced by class in the emitted document.

use crate::px;
use rpt_model::Color;
use rpt_pages::{LineStyle, RectOp};
use std::collections::HashMap;

/// A deduplicated typography class (`fc<uid>-N`).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct FontKey {
    /// Point size × 1000 (so `f32` sizes compare/hash exactly).
    pub(crate) size_milli: i32,
    pub(crate) rgb: (u8, u8, u8),
    pub(crate) family: String,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
}

impl FontKey {
    pub(crate) fn new(font: &rpt_pages::FontSpec, color: Color) -> FontKey {
        FontKey {
            size_milli: (font.size_pt * 1000.0).round() as i32,
            rgb: (color.r, color.g, color.b),
            family: font.family.clone(),
            bold: font.bold,
            italic: font.italic,
            underline: font.underline,
            strikethrough: font.strikethrough,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BorderStyle {
    Solid,
    Double,
    Dashed,
    Dotted,
}

impl BorderStyle {
    pub(crate) fn from(style: LineStyle) -> BorderStyle {
        match style {
            LineStyle::Single => BorderStyle::Solid,
            LineStyle::Double => BorderStyle::Double,
            LineStyle::Dashed => BorderStyle::Dashed,
            LineStyle::Dotted => BorderStyle::Dotted,
        }
    }
    pub(crate) fn css(self) -> &'static str {
        match self {
            BorderStyle::Solid => "solid",
            BorderStyle::Double => "double",
            BorderStyle::Dashed => "dashed",
            BorderStyle::Dotted => "dotted",
        }
    }
}

/// A deduplicated adornment class (`ad<uid>-N`): optional fill plus a per-side border.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct AdornKey {
    pub(crate) bg: Option<(u8, u8, u8)>,
    pub(crate) border_rgb: (u8, u8, u8),
    /// (style, width-px) for left, right, top, bottom. Width 0 = no visible border on that side.
    pub(crate) sides: [(BorderStyle, u32); 4],
    /// Corner radius in px (rounded box); 0 = square corners.
    pub(crate) radius_px: u32,
}

impl AdornKey {
    /// A borderless, unfilled object's default adornment (black border color, zero widths).
    pub(crate) fn plain() -> AdornKey {
        AdornKey {
            bg: None,
            border_rgb: (0, 0, 0),
            sides: [(BorderStyle::Solid, 0); 4],
            radius_px: 0,
        }
    }

    /// Derive the adornment from a Box rect (an object's border/fill descriptor).
    pub(crate) fn from_rect(r: &RectOp) -> AdornKey {
        // A pure-white fill is the "no fill" default in the native output — omit it. A gradient/hatch
        // box fill collapses to its representative solid colour here (the div model has no pattern).
        let bg = r
            .fill
            .as_ref()
            .map(|f| f.representative_color())
            .and_then(|c| {
                if (c.r, c.g, c.b) == (255, 255, 255) {
                    None
                } else {
                    Some((c.r, c.g, c.b))
                }
            });
        let (border_rgb, side) = match &r.stroke {
            Some(s) => (
                (s.color.r, s.color.g, s.color.b),
                (BorderStyle::from(s.style), px(s.width.0).max(1) as u32),
            ),
            None => ((0, 0, 0), (BorderStyle::Solid, 0)),
        };
        AdornKey {
            bg,
            border_rgb,
            sides: [side; 4],
            radius_px: px(r.corner_radius.0).max(0) as u32,
        }
    }

    pub(crate) fn has_border(&self) -> bool {
        self.sides.iter().any(|(_, w)| *w > 0)
    }
}

/// One distinct embedded image, emitted once into `<style>` as a `background-image` class and
/// referenced by class at every placement (so identical bytes are never inlined more than once).
pub(crate) struct ImageClass {
    pub(crate) media_type: String,
    pub(crate) bytes: Vec<u8>,
}

/// The per-report, first-appearance-ordered dedup tables emitted into `<style>`: fonts, adornments,
/// and images (images keyed by a content hash of their bytes, so the same picture used N times is
/// embedded once and referenced N times).
#[derive(Default)]
pub(crate) struct Tables {
    pub(crate) fonts: Vec<FontKey>,
    fmap: HashMap<FontKey, usize>,
    pub(crate) adorns: Vec<AdornKey>,
    amap: HashMap<AdornKey, usize>,
    pub(crate) images: Vec<ImageClass>,
    imap: HashMap<u64, usize>,
}

impl Tables {
    pub(crate) fn font(&mut self, k: FontKey) -> usize {
        if let Some(&i) = self.fmap.get(&k) {
            return i;
        }
        let i = self.fonts.len();
        self.fmap.insert(k.clone(), i);
        self.fonts.push(k);
        i
    }

    /// Intern an image by a content hash of its bytes, returning its class index. Identical bytes
    /// (the repeated header logo, or duplicate product thumbnails) collapse to one entry.
    pub(crate) fn image(&mut self, media_type: &str, bytes: &[u8]) -> usize {
        let key = rpt_render_util::content_hash(bytes);
        if let Some(&i) = self.imap.get(&key) {
            return i;
        }
        let i = self.images.len();
        self.imap.insert(key, i);
        self.images.push(ImageClass {
            media_type: media_type.to_string(),
            bytes: bytes.to_vec(),
        });
        i
    }

    pub(crate) fn adorn(&mut self, k: AdornKey) -> usize {
        if let Some(&i) = self.amap.get(&k) {
            return i;
        }
        let i = self.adorns.len();
        self.amap.insert(k.clone(), i);
        self.adorns.push(k);
        i
    }

    /// A deterministic per-report id for the class families — derived from the (order-independent)
    /// contents of both tables, so it is stable across runs but distinct per report. The native
    /// engine uses a random GUID here; the parity tooling normalizes it, and determinism keeps our
    /// tests stable.
    pub(crate) fn uid(&self) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |bytes: &[u8]| {
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        for f in &self.fonts {
            mix(&f.size_milli.to_le_bytes());
            mix(&[f.rgb.0, f.rgb.1, f.rgb.2]);
            mix(f.family.as_bytes());
            mix(&[f.bold as u8, f.italic as u8, f.underline as u8]);
            // Only a strikethrough font mixes an extra marker, so a run without one keeps the same
            // class hash as before this attribute existed (existing snapshots stay stable).
            if f.strikethrough {
                mix(b"strike");
            }
        }
        for a in &self.adorns {
            mix(&[a.border_rgb.0, a.border_rgb.1, a.border_rgb.2]);
            for (s, w) in &a.sides {
                mix(&[*s as u8]);
                mix(&w.to_le_bytes());
            }
            // Only rounded boxes contribute the radius, so square-box reports keep a stable uid.
            if a.radius_px > 0 {
                mix(&a.radius_px.to_le_bytes());
            }
        }
        format!("{h:016x}")
    }
}
