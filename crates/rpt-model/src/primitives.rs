//! Shared value types for the semantic model.

/// A length in twips (1/1440 inch) — Crystal's internal unit for page geometry, object
/// positions, and margins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Twips(pub i32);

impl Twips {
    /// The value in inches.
    pub fn inches(self) -> f64 {
        self.0 as f64 / 1440.0
    }
}

/// A rectangle in twips (SDK object bounds: Left/Top/Width/Height).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    /// Distance from the container's left edge (SDK `Left`).
    pub left: Twips,
    /// Distance from the container's top edge (SDK `Top`).
    pub top: Twips,
    /// Horizontal extent (SDK `Width`).
    pub width: Twips,
    /// Vertical extent (SDK `Height`).
    pub height: Twips,
}

impl Rect {
    /// The right edge (`left + width`).
    pub fn right(&self) -> Twips {
        Twips(self.left.0 + self.width.0)
    }

    /// The bottom edge (`top + height`).
    pub fn bottom(&self) -> Twips {
        Twips(self.top.0 + self.height.0)
    }

    /// A copy shifted by `(dx, dy)` twips; the size is unchanged.
    pub fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect {
            left: Twips(self.left.0 + dx),
            top: Twips(self.top.0 + dy),
            width: self.width,
            height: self.height,
        }
    }

    /// Whether the point `(x, y)` lies within this rectangle, edges inclusive.
    pub fn contains(&self, x: Twips, y: Twips) -> bool {
        x.0 >= self.left.0 && x.0 <= self.right().0 && y.0 >= self.top.0 && y.0 <= self.bottom().0
    }
}

/// A report format version (SDK: Major/MinorVersion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Version {
    /// The major version component (SDK `MajorVersion`).
    pub major: i32,
    /// The minor version component (SDK `MinorVersion`).
    pub minor: i32,
}

/// A value with an optional conditional-formatting formula override (SDK: a property + its
/// `ConditionFormula`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Conditioned<T> {
    /// The plain (unconditional) property value.
    pub value: T,
    /// The conditional-formatting formula that overrides `value` at runtime, if any.
    pub formula: Option<Formula>,
}

/// An ARGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Color {
    /// Alpha channel (255 = opaque).
    pub a: u8,
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Color {
    /// Opaque white (the engine's default background colour).
    pub const WHITE: Color = Color {
        a: 255,
        r: 255,
        g: 255,
        b: 255,
    };

    /// The CSS `#rrggbb` hex string (lowercase, alpha dropped) — the shared colour serialization for
    /// the HTML and SVG backends.
    pub fn to_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// A Crystal formula expression (Crystal syntax), e.g. `&dateadd('d',-1,{Command.x})`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Formula(pub String);

/// A back-reference to the substrate record a DOM node was raised from, so edits can be
/// lowered surgically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RecordRef {
    /// Index of the record within its stream.
    pub index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twips_inches() {
        assert_eq!(Twips(1440).inches(), 1.0);
        assert_eq!(Twips(720).inches(), 0.5);
        assert_eq!(Twips(0).inches(), 0.0);
        assert_eq!(Twips(-1440).inches(), -1.0);
    }

    #[test]
    fn rect_edges_and_translate() {
        let r = Rect {
            left: Twips(100),
            top: Twips(200),
            width: Twips(300),
            height: Twips(400),
        };
        assert_eq!(r.right(), Twips(400));
        assert_eq!(r.bottom(), Twips(600));
        let t = r.translate(10, -20);
        assert_eq!(t.left, Twips(110));
        assert_eq!(t.top, Twips(180));
        // Size is preserved under translation.
        assert_eq!(t.width, r.width);
        assert_eq!(t.height, r.height);
        assert_eq!(t.right(), Twips(410));
    }

    #[test]
    fn rect_contains_is_edge_inclusive() {
        let r = Rect {
            left: Twips(100),
            top: Twips(100),
            width: Twips(200),
            height: Twips(200),
        };
        // Interior.
        assert!(r.contains(Twips(150), Twips(150)));
        // All four corners are inside (edges inclusive).
        assert!(r.contains(Twips(100), Twips(100)));
        assert!(r.contains(Twips(300), Twips(300)));
        // Just outside each edge.
        assert!(!r.contains(Twips(99), Twips(150)));
        assert!(!r.contains(Twips(301), Twips(150)));
        assert!(!r.contains(Twips(150), Twips(99)));
        assert!(!r.contains(Twips(150), Twips(301)));
    }

    #[test]
    fn color_to_hex_drops_alpha_and_lowercases() {
        assert_eq!(Color::WHITE.to_hex(), "#ffffff");
        assert_eq!(
            Color {
                a: 0,
                r: 0,
                g: 0,
                b: 0
            }
            .to_hex(),
            "#000000"
        );
        // Alpha is dropped; hex is zero-padded and lowercase.
        assert_eq!(
            Color {
                a: 128,
                r: 0xAB,
                g: 0x05,
                b: 0xFF
            }
            .to_hex(),
            "#ab05ff"
        );
    }
}
