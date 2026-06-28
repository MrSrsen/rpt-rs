# Appendix: endianness

The `.rpt` format genuinely mixes big- and little-endian. This is not a decode artifact — it is a property of the file,
and getting it wrong is a common mistake. This appendix is the map.

## Why both appear

A `.rpt` is a stack of layers authored by different code over a long history and never harmonized: the CFB container,
the encrypted/compressed stream, Crystal's own record framing, and embedded native Windows structures. Each layer keeps
its own byte order.

- **Big-endian — the Crystal record/format layer.** The higher-level, Crystal-defined structures (length prefixes,
  reference IDs, offsets, geometry) lean big-endian — typical of a long-lived, originally cross-platform format that
  chose a portable byte order for serialized fields.
- **Little-endian — where it rides on Windows/x86.** Value-type codes, flag fields, and anything mirroring a Win32
  structure (such as `DEVMODE`) are little-endian, because that is what the platform produced when the value was
  written.
- **GUIDs are themselves mixed-endian** (the first three fields little-endian, the last two big-endian), so a single
  GUID value can look internally inconsistent.

## The tendency, by field kind

| Field kind                                                                  | Byte order            |
| --------------------------------------------------------------------------- | --------------------- |
| Record/string length prefixes                                               | big-endian            |
| Reference IDs, offsets, indices (e.g. subdocument index)                    | big-endian            |
| Geometry and page measurements (twips: margins, paper rectangle, font size) | big-endian            |
| Font weight                                                                 | big-endian (`u16`)    |
| Field/parameter value-type codes                                            | little-endian (`u16`) |
| `DEVMODE`-derived fields (orientation, paper size, source)                  | little-endian         |
| Single-byte flags and bitfields                                             | endian-neutral        |

## Special encodings

- **Variable-width coordinates.** Box/line geometry uses a `read_coord` scheme: 2 bytes normally, 4 bytes when the value
  exceeds `0x7FFF`.
- **GUIDs** are mixed-endian as noted above.
- **Stored numeric values in parameters** use their own encodings: numbers/currency as a big-endian `f64` divided by
  100, dates as a big-endian Julian day number.

## The debugging signal

A decoded length or offset that comes out absurdly large (millions or billions) is the classic tell of a flipped endian
assumption. ASCII text read as a big-endian `u32` is at least `0x20202020` (~540 million), so a "length" in that range
is almost certainly mis-read bytes.

This is also used as a feature: some records self-reject in a decoder precisely because their ASCII content, read as a
big-endian length, produces an out-of-bounds value that fails a bounds check — which cleanly distinguishes them from the
records that decoder is meant to handle.

## Rule of thumb

Treat endianness as a **per-field fact**, not a global convention. When adding a decode, try the layer's tendency first
(framing length/ID → big-endian; value code/flag → little-endian), then confirm against a known-good value; if it comes
out wildly wrong, flip and re-check.
