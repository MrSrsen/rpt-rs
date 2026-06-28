//! The record-type registry — the numeric ↔ symbolic mapping for TSLV record types.
//!
//! The registry is flat (`u16` keyed); sub-documents reuse the same vocabulary. Any unmapped
//! type is still a first-class [`RecordTag`], just without a name.

/// A TSLV record type. Always carries the raw numeric type; a human name is attached for the
/// types we have identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordTag(pub u16);

impl RecordTag {
    /// The type-`0xffff` stream header record.
    pub const STREAM_HEADER: RecordTag = RecordTag(0xFFFF);

    /// The raw numeric record type.
    pub fn value(self) -> u16 {
        self.0
    }

    /// The symbolic name for this record type, if identified.
    pub fn name(self) -> Option<&'static str> {
        match self.0 {
            0xFFFF => Some("StreamHeader"),
            0x0064 => Some("ReportRoot"),
            0x0003 => Some("PrinterInfo"),
            0x0007 => Some("PaperSize"),
            0x0071 => Some("NamedValue"),
            0x0073 => Some("FieldDef"),
            0x0076 => Some("Formula"),
            0x0078 => Some("ReportProperty"),
            0x008a => Some("Area"),
            0x008c => Some("Section"),
            0x009e => Some("ObjectName"),
            0x009f => Some("FieldObject"),
            0x00c2 => Some("TextObject"),
            0x0008 => Some("Font"),
            _ => None,
        }
    }

    /// True if this record type has been identified (has a name).
    pub fn is_known(self) -> bool {
        self.name().is_some()
    }
}

impl std::fmt::Display for RecordTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{name}({:#06x})", self.0),
            None => write!(f, "{:#06x}", self.0),
        }
    }
}
