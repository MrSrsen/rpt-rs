# rpt-rs documentation

Technical documentation for the `.rpt` format and the `rpt-rs` library.

The format documents are programming-language-agnostic: they describe the on-disk `.rpt` structure itself. Read them in
order — each builds on the previous one.

## Format

1. [Format overview](01-format-overview.md) — the big picture: what a `.rpt` file is and the full decode pipeline from
   bytes to a typed report.
2. [The container](02-container.md) — the CFB/OLE compound file and the streams inside it.
3. [Stream decoding](03-stream-decoding.md) — the stream header, the cipher, decompression, and how raw bytes become a
   flat sequence of records.
4. [The record tree](04-record-tree.md) — how records nest, the per-record masking, and the lossless record substrate.
5. [The semantic model](05-semantic-model.md) — how the record tree is projected into the typed report model.

## Reference

- [Block catalog](06-block-catalog.md) — every record (block) type the library decodes: what it means, its byte layout,
  and the blocks that are recognized but not yet decoded.
- [Support matrix](09-support-matrix.md) — which format features and record types are supported.
- [Endianness](appendix-endianness.md) — the format mixes big- and little-endian; this is the map.

## Using the library

- [The codebase](07-codebase.md) — the crates and modules, what each contains, and why the boundaries are where they
  are.
- [Usage](08-usage.md) — the CLI tools and the library API, with examples.
