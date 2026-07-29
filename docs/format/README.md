# The `.rpt` format

The on-disk structure of a Crystal Reports `.rpt` file: the compound-file container, the stream cipher and compression,
the record tree, and the record types themselves. These pages are programming-language-agnostic — they describe the
format, not this implementation — so they read the same whether you are using `rpt-rs` or writing a reader of your own.

Read front to back:

1. [Format overview](01-overview.md) — the big picture: what a `.rpt` file is and the full decode pipeline from bytes to
   a typed report.
2. [The container](02-container.md) — the CFB/OLE compound file and the streams inside it.
3. [Stream decoding](03-stream-decoding.md) — the stream header, the cipher, decompression, and how raw bytes become a
   flat sequence of records.
4. [The record tree](04-record-tree.md) — how records nest, the per-record masking, and the lossless record layer.
5. [Saved data](05-saved-data.md) — how a report's cached rows (saved with data) are laid out and decoded.
6. [Block catalog](06-block-catalog.md) — every record (block) type the library decodes: what it means, its byte layout,
   and the blocks that are recognized but not yet decoded.
7. [Endianness](07-endianness.md) — the format mixes big- and little-endian; this is the map.

---

← [Documentation index](../README.md) · **Start here:** [Format overview](01-overview.md) →
