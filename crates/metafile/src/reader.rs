//! Bounds-checked little-endian scalar reads over a byte slice.
//!
//! Every metafile field read goes through these helpers so a truncated or garbage stream yields
//! `None` (turned into an [`Error`](crate::Error) by the caller) rather than panicking.

/// Read a little-endian `u32` at `off`, or `None` if the slice is too short.
pub(crate) fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes(s.try_into().ok()?))
}

/// Read a little-endian `i32` at `off`.
pub(crate) fn i32_at(b: &[u8], off: usize) -> Option<i32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(i32::from_le_bytes(s.try_into().ok()?))
}

/// Read a little-endian `u16` at `off`.
pub(crate) fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off.checked_add(2)?)?;
    Some(u16::from_le_bytes(s.try_into().ok()?))
}

/// Read a little-endian `i16` at `off`.
pub(crate) fn i16_at(b: &[u8], off: usize) -> Option<i16> {
    let s = b.get(off..off.checked_add(2)?)?;
    Some(i16::from_le_bytes(s.try_into().ok()?))
}

/// Read an IEEE-754 little-endian `f32` at `off` (EMF `XFORM` matrix elements).
pub(crate) fn f32_at(b: &[u8], off: usize) -> Option<f32> {
    let s = b.get(off..off.checked_add(4)?)?;
    Some(f32::from_le_bytes(s.try_into().ok()?))
}
