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
    pub left: Twips,
    pub top: Twips,
    pub width: Twips,
    pub height: Twips,
}

/// A report format version (SDK: Major/MinorVersion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Version {
    pub major: i32,
    pub minor: i32,
}

/// A value with an optional conditional-formatting formula override (SDK: a property + its
/// `ConditionFormula`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Conditioned<T> {
    pub value: T,
    pub formula: Option<Formula>,
}

/// An ARGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
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
