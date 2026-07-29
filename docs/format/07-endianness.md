# Endianness

The `.rpt` format genuinely mixes big- and little-endian. This is not a decode artifact — it is a property of the file,
and getting it wrong is a common mistake. This page is the map.

## Why both appear

A `.rpt` is a stack of layers authored by different code over a long history and never harmonized: the CFB container,
the encrypted/compressed stream, Crystal's own record framing, and embedded native Windows structures. Each layer keeps
its own byte order.

- **Big-endian — the Crystal record/format layer.** The higher-level, Crystal-defined structures (length prefixes,
  reference IDs, offsets, geometry) lean big-endian — typical of a long-lived, originally cross-platform format that
  chose a portable byte order for serialized fields.
- **Little-endian — where it rides on Windows/x86.** Some value-type codes and flag fields are little-endian, because
  that is what the platform produced when the value was written. Note that a Win32-derived structure is not
  automatically little-endian here: the compacted `DEVMODE` in `0x0007` is written out big-endian like the rest of the
  record layer.
- **GUIDs are themselves mixed-endian** (the first three fields little-endian, the last two big-endian), so a single
  GUID value can look internally inconsistent.

## The tendency, by field kind

| Field kind                                                                  | Byte order                                   |
|-----------------------------------------------------------------------------|----------------------------------------------|
| Record/string length prefixes                                               | big-endian                                   |
| Record header schema word (the record type's version)                       | big-endian (`u16`)                           |
| Reference IDs, offsets, indices (e.g. subdocument index)                    | big-endian                                   |
| Geometry and page measurements (twips: margins, paper rectangle, font size) | big-endian                                   |
| Font weight                                                                 | big-endian (`u16`)                           |
| `Contents` field/parameter value-type codes (`0x0071 NamedValue`)           | narrowing (1 byte, 2 with the `0x80` escape) |
| `QESession` field/parameter value-type codes (`0x0004`, `0x0007`)           | big-endian (`u32`)                           |
| `DEVMODE`-derived fields (orientation, paper size, source)                  | big-endian (unlike Win32)                    |
| Single-byte flags and bitfields                                             | endian-neutral                               |

## Special encodings

- **Narrowing integers**, whose width follows their magnitude and whose wide form carries its marker in the top bit (so
  the value is unsigned by construction, and the encoding is not LEB128 — only the variable width is shared with one).
  Two widths occur: an enum or small count is 1 byte, or 2 with `0x80` set; a twip coordinate is 2 bytes, or 4 when the
  value exceeds `0x7FFF`. Box/line geometry is the second form throughout.
- **GUIDs** are mixed-endian as noted above.
- **Stored numeric values in parameters** use their own encodings: numbers/currency as a big-endian `f64` divided by
  100, dates as a big-endian Julian day number.

## The debugging signal

A decoded length or offset that comes out absurdly large (millions or billions) is the classic tell of a flipped endian
assumption. ASCII text read as a big-endian `u32` is at least `0x20202020` (~540 million), so a "length" in that range
is almost certainly mis-read bytes.

Where one type number names two structurally unrelated records, though, that is not what tells them apart. The decoder
routes a record by its type, its **schema word** and its stream's vocabulary, so a record written at the other version
simply has no field table and is never read as the one it is not — a decision made before any byte of content is
touched, rather than a mis-read length failing a bounds check.

## Rule of thumb

Treat endianness as a **per-field fact**, not a global convention. When adding a decode, try the layer's tendency first
(framing length/ID → big-endian; value code/flag → little-endian), then confirm against a known-good value; if it comes
out wildly wrong, flip and re-check. For a record type that already has a field table, `rpt dump` prints that table's
reading — name, kind, value and byte range per field, and every multi-byte kind names its byte order (`u16be`,
`f32le`) — which settles it outright; for one that does not, its scalar-probe grid prints every offset both ways at
once. See [Usage](../reader/03-cli.md).

---

← [Block catalog](06-block-catalog.md) · **Back to the** [format index](README.md)
