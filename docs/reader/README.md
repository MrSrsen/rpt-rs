# The reader

The `rpt-reader` crate and the `rpt` binary: how the record tree becomes a typed report model, how much of the format that
model covers, and how to drive it — from the command line or from Rust. Read this once you know what
[the format](../format/README.md) holds and want to get at it.

Read front to back:

1. [The semantic model](01-semantic-model.md) — how the record tree is built into the typed report model.
2. [Support matrix](02-support-matrix.md) — which format features and record types are supported.
3. [The `rpt` CLI](03-cli.md) — the inspection commands, the JSON/KDL exports, and the byte-level write path.
4. [The `rpt-reader` library](04-library.md) — opening a report from Rust, the model API, derived analytics, and error
   handling.

---

← [Documentation index](../README.md) · **Start here:** [The semantic model](01-semantic-model.md) →
