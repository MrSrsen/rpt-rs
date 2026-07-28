# rpt-rs documentation

Technical documentation for the `.rpt` format and the `rpt-rs` library.

The format documents are programming-language-agnostic: they describe the on-disk `.rpt` structure itself. Every page
ends with a link to the next one, so the whole set reads front to back in the order below.

## Format

1. [Format overview](01-format-overview.md) — the big picture: what a `.rpt` file is and the full decode pipeline from
   bytes to a typed report.
2. [The container](02-container.md) — the CFB/OLE compound file and the streams inside it.
3. [Stream decoding](03-stream-decoding.md) — the stream header, the cipher, decompression, and how raw bytes become a
   flat sequence of records.
4. [The record tree](04-record-tree.md) — how records nest, the per-record masking, and the lossless record substrate.
5. [The semantic model](05-semantic-model.md) — how the record tree is projected into the typed report model.
6. [Saved data](06-saved-data.md) — how a report's cached rows (saved with data) are laid out and decoded.

## Reference

7. [Block catalog](07-block-catalog.md) — every record (block) type the library decodes: what it means, its byte layout,
   and the blocks that are recognized but not yet decoded.
8. [Support matrix](08-support-matrix.md) — which format features and record types are supported.
9. [Endianness](09-endianness.md) — the format mixes big- and little-endian; this is the map.

## Using the library

10. [The codebase](10-codebase.md) — the crates and modules, what each contains, and why the boundaries are where they
    are.
11. [Usage](11-usage.md) — the two CLI binaries and the library API, with examples.
12. [Rendering](12-rendering.md) — the render pipeline (data → layout → Page IR → backends), the public API for driving
    a render, the coordinate model, locale/format resolution, the `rpt-render` CLI, and the render-test corpora.
13. [Render examples](13-render-examples.md) — copy-paste recipes for driving the renderer: saved data, live DB, a
    custom `RowSource`, and WASM.

## The formula engine

The `crystal-formula` crate — the Crystal/Basic formula language (lexer, parser, AST, type system, bytecode VM) — is
documented in [`formula-engine/`](formula-engine/):

14. [Overview](formula-engine/README.md) — what the crate is, a quick start, and why it stands alone.
15. [Architecture & VM](formula-engine/01-architecture.md) — the pipeline, the value model, variables/scopes,
    references, the per-record cache, and error handling.
16. [Language reference](formula-engine/02-language.md) — both dialects (Crystal & Basic): lexis, operators,
    expressions, statement bodies, with an EBNF sketch.
17. [Builtin functions](formula-engine/03-builtins.md) — the builtin library by family, with signatures and semantics.
18. [Validation](formula-engine/04-validation.md) — the semantic diagnostics pass behind the Crystal LSP and the
    `rpt formulas` report checker.

---

**Start here:** [Format overview](01-format-overview.md) →
