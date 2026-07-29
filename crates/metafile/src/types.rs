//! The neutral, device-independent drawing vocabulary handed to a [`MetafileSink`](crate::MetafileSink).
//!
//! These types deliberately know nothing about any target scene: colors are RGBA bytes, coordinates
//! are `f64` in the metafile's own device space (bounded by [`MetafileHeader::bounds`]), and lengths
//! are logical units. A consumer maps [`MetafileHeader::bounds`] onto its own output box and scales
//! every coordinate through that one mapping.

/// An opaque or translucent RGBA color, one byte per channel.
///
/// Metafile `COLORREF`s are opaque, so [`Color::a`] is `255` for every color a parser produces; the
/// field exists so consumers can carry through their own alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red channel (0–255).
    pub r: u8,
    /// Green channel (0–255).
    pub g: u8,
    /// Blue channel (0–255).
    pub b: u8,
    /// Alpha channel (0–255); `255` is fully opaque.
    pub a: u8,
}

impl Color {
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    /// An opaque color from red/green/blue bytes.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// Decode a Win32 `COLORREF` (`0x00BBGGRR`, little-endian byte order R, G, B) into an opaque
    /// color. The high byte (flags/alpha) is ignored.
    #[must_use]
    pub const fn from_colorref(v: u32) -> Color {
        Color {
            r: (v & 0xff) as u8,
            g: ((v >> 8) & 0xff) as u8,
            b: ((v >> 16) & 0xff) as u8,
            a: 255,
        }
    }
}

/// A point in the metafile's device space (`rclBounds` units), after the world/window/viewport
/// transform has been applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Horizontal position.
    pub x: f64,
    /// Vertical position.
    pub y: f64,
}

impl Point {
    /// A point from its coordinates.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }
}

/// An axis-aligned rectangle in device space, stored as its edges. `right`/`bottom` are normalised
/// so they are never less than `left`/`top`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub left: f64,
    /// Top edge.
    pub top: f64,
    /// Right edge (≥ `left`).
    pub right: f64,
    /// Bottom edge (≥ `top`).
    pub bottom: f64,
}

impl Rect {
    /// A rectangle from two opposite corners, normalising the edges so `right ≥ left` and
    /// `bottom ≥ top`.
    #[must_use]
    pub fn from_corners(a: Point, b: Point) -> Rect {
        Rect {
            left: a.x.min(b.x),
            top: a.y.min(b.y),
            right: a.x.max(b.x),
            bottom: a.y.max(b.y),
        }
    }

    /// The rectangle's width (`right - left`, always ≥ 0).
    #[must_use]
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    /// The rectangle's height (`bottom - top`, always ≥ 0).
    #[must_use]
    pub fn height(&self) -> f64 {
        self.bottom - self.top
    }
}

/// A pen line style ([MS-EMF] `PenStyle`), controlling how a stroke is dashed.
///
/// [MS-EMF]: https://learn.microsoft.com/openspecs/windows_protocols/ms-emf/
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PenStyle {
    /// `PS_SOLID` — a continuous line.
    #[default]
    Solid,
    /// `PS_DASH` — a dashed line.
    Dash,
    /// `PS_DOT` — a dotted line.
    Dot,
    /// `PS_DASHDOT` — alternating dashes and dots.
    DashDot,
    /// `PS_DASHDOTDOT` — a dash followed by two dots.
    DashDotDot,
}

/// A resolved pen: the stroke a `SELECTOBJECT` of a pen produces. A `PS_NULL` (invisible) pen is
/// represented by the absence of a pen (`Option::None`) at the point of use, never by this struct.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pen {
    /// Stroke color.
    pub color: Color,
    /// Pen width in logical units. `0` denotes a cosmetic (one-device-pixel) pen; consumers floor it
    /// to a visible minimum after scaling.
    pub width: f64,
    /// Dash style.
    pub style: PenStyle,
}

/// A resolved brush (area fill). A `BS_NULL`/`BS_HOLLOW` brush is represented by the absence of a
/// brush (`Option::None`) at the point of use, never by this struct.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brush {
    /// Solid fill color. Hatched and pattern brushes are approximated by their foreground color.
    pub color: Color,
}

/// A resolved font (a subset of `LOGFONT`) for a text run. Consumers that lay out their own text use
/// [`Font::face`] and the sign/magnitude of [`Font::height`]; a parser hands the metafile's stored
/// logical values unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// Typeface name (`lfFaceName`); empty when the metafile leaves it default.
    pub face: String,
    /// Character height in logical units (`lfHeight`). A negative value is the em height, a positive
    /// value the cell height, per GDI convention; `0` requests the device default.
    pub height: f64,
    /// Stroke weight (`lfWeight`, 0–1000; 400 = normal, 700 = bold).
    pub weight: u16,
    /// Whether the font is italic (`lfItalic`).
    pub italic: bool,
    /// Whether the font is underlined (`lfUnderline`).
    pub underline: bool,
    /// Escapement/orientation in tenths of a degree (`lfEscapement`), counter-clockwise from the x
    /// axis.
    pub escapement: i32,
}

impl Default for Font {
    fn default() -> Font {
        Font {
            face: String::new(),
            height: 0.0,
            weight: 400,
            italic: false,
            underline: false,
            escapement: 0,
        }
    }
}

/// The container format of a [`Bitmap`] — a complete, self-contained image file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BitmapFormat {
    /// A Windows `.bmp` file (14-byte `BITMAPFILEHEADER` + DIB), reconstructed from an uncompressed
    /// device-independent bitmap. The common case.
    Bmp,
    /// A PNG file, carried verbatim from a `BI_PNG` DIB.
    Png,
    /// A JPEG file, carried verbatim from a `BI_JPEG` DIB.
    Jpeg,
}

impl BitmapFormat {
    /// The IANA media type (`image/bmp` / `image/png` / `image/jpeg`).
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            BitmapFormat::Bmp => "image/bmp",
            BitmapFormat::Png => "image/png",
            BitmapFormat::Jpeg => "image/jpeg",
        }
    }
}

/// A raster image a metafile blits (`STRETCHDIBITS`/`BITBLT`/…), delivered as a complete,
/// self-contained image file. The parser does no pixel work: an uncompressed device-independent
/// bitmap is repackaged as a `.bmp` (a DIB is a BMP without its 14-byte file header) and a
/// `BI_PNG`/`BI_JPEG` DIB is passed through unchanged, so a consumer hands [`bytes`](Self::bytes)
/// straight to an image decoder or embeds it as-is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    /// The container format of [`bytes`](Self::bytes).
    pub format: BitmapFormat,
    /// A complete image file (BMP/PNG/JPEG), not raw pixels.
    pub bytes: Vec<u8>,
}
