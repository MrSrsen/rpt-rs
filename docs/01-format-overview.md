# Format overview

A SAP Crystal Reports `.rpt` file stores a **report definition**: the data sources it reads, the parameters it prompts
for, the formulas it computes, and the way it lays everything out on the page. The format is proprietary and
undocumented; this document describes how it is structured and how `rpt-rs` decodes it.

## The big picture

A `.rpt` file is built as a stack of layers. Each layer wraps the one below it, and decoding peels them off in order:

```
┌─ .rpt file ───────────────────────────────────────────────────────────────┐
│  CFB / OLE2 compound file (a container of named streams)                  │
│  ┌─ Contents stream ────────────────────────────────────────────────────┐ │
│  │  stream header (plaintext: version, IV)                              │ │
│  │  ┌─ encrypted + compressed payload ─────────────────────────────────┐│ │
│  │  │  AES-128-CFB  →  zlib deflate  →  a flat sequence of records     ││ │
│  │  │  each record's content is itself a nested tree of records        ││ │
│  │  └──────────────────────────────────────────────────────────────────┘│ │
│  └──────────────────────────────────────────────────────────────────────┘ │
│  other streams: QESession, PromptManager, ReportInfo, SummaryInformation  │
│  nested storages: Subdocument N (subreports), Embedding N (OLE objects)   │
└───────────────────────────────────────────────────────────────────────────┘
```

The report definition itself is small — typically a few kilobytes — regardless of the overall file size. Large files are
large because of embedded images and cached data, not because of the definition.

## The decode pipeline

Decoding a report runs these stages in order. Each has its own document:

1. **Open the container.** A `.rpt` is a Microsoft Compound File (CFB/OLE2) — the same structured-storage container used
   by legacy `.doc`/`.xls`. It holds named _streams_ (files) and _storages_ (directories). See
   [The container](02-container.md).

2. **Read the stream header.** Each report stream begins with a small plaintext header record that carries the format
   version and the per-stream initialization vector (IV) for decryption. See [Stream decoding](03-stream-decoding.md).

3. **Decrypt.** The rest of the stream is encrypted with AES-128 in CFB mode, using a fixed key embedded in the engine
   and the per-stream IV.

4. **Decompress.** The decrypted bytes are a standard zlib deflate stream; inflating them yields the logical report
   bytes.

5. **Tile into records.** The logical bytes are a flat sequence of length-delimited records (the _TSLV_ framing: type,
   length, value). Cutting them apart is "tiling".

6. **Build the record tree.** Each record's content is itself a nested sequence of records. A masking scheme (an XOR
   keyed by the record types on the parse stack) makes the nested content readable. The result is a tree of records —
   the _lossless substrate_. See [The record tree](04-record-tree.md).

7. **Project the semantic model.** The record tree is walked and projected into a typed report model: database,
   parameters, formulas, groups, sections, and report objects. See [The semantic model](05-semantic-model.md).

Stages 1–6 are fully reversible and lossless: every record is preserved, including types the library does not yet
understand. Stage 7 is a projection on top — it grows over time without ever risking the round-trip.

## What "record" means

Almost everything in a report is a _record_ (also called a block): a small unit with a numeric **type**, a **length**,
and a **value** (its content). Records nest. A report is one root record whose content contains section records, which
contain object records, and so on. The numeric type identifies what the record is — a font, a text object, a formula, a
database field. The [block catalog](06-block-catalog.md) documents each type.

## A note on endianness

The format mixes byte orders. The Crystal-defined record framing (lengths, IDs, geometry) tends to be **big-endian**,
while value codes, flags, and embedded Windows structures tend to be **little-endian**. This is a property of the
format, not a decode choice; see [Endianness](appendix-endianness.md).
