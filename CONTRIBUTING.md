# Contributing to rpt-rs

Thanks for your interest in improving rpt-rs. This document covers how to build, test, and structure changes.

## Getting started

You need a Rust toolchain (the minimum supported version is **1.85**). Then:

```sh
cargo build            # build the workspace
cargo test             # run the test suite
cargo fmt --all        # format
cargo clippy --all-targets --all-features -- -D warnings   # lint (CI fails on warnings)
```

The workspace forbids `unsafe` code (`unsafe_code = "forbid"`); keep contributions safe Rust.

## Workspace layout

rpt-rs is a four-crate workspace, and the boundary between the crates is intentional:

| Crate        | Role                                                                                          |
| ------------ | --------------------------------------------------------------------------------------------- |
| `rpt`        | Pure I/O. Decodes the **stored** facts from the bytes: container → records → typed model.     |
| `rpt-engine` | **Derived** analytics computed on top of the model (e.g. field use counts, formula analysis). |
| `rpt-to-xml` | Exports a report to a structured XML document.                                                |
| `rpt-cli`    | The `rpt` inspection CLI.                                                                     |

**The boundary is load-bearing.** If a value is read directly from the file, decode it in `rpt`. If a value is _computed
or inferred_ (not present in the bytes), it belongs in `rpt-engine` — not as a stored field on an `rpt` model struct.
Keeping I/O separate from derivation is the core design rule.

## The `.rpt` format

`.rpt` is an undocumented, encrypted binary format. If you're working on decoding, start with the documentation in
[`docs/`](docs/):

- [Format overview](docs/01-format-overview.md), [container](docs/02-container.md),
  [stream decoding](docs/03-stream-decoding.md), [record tree](docs/04-record-tree.md), and
  [semantic model](docs/05-semantic-model.md) explain the pipeline end to end.
- The [block catalog](docs/06-block-catalog.md) and [support matrix](docs/09-support-matrix.md) describe each decoded
  record type and what is and isn't supported.

## Tests and XML baselines

The regression suite in `crates/rpt-to-xml/tests/baseline.rs` exports a set of public sample reports (under
`tests/fixtures/`) to XML and compares the output against committed baselines. To keep results deterministic across
machines, the exporter runs inside a [Bubblewrap](https://github.com/containers/bubblewrap) sandbox with the report
bind-mounted at a fixed path, so path-derived attributes are identical everywhere.

Run it (requires `bwrap` on Linux):

```sh
cargo test -p rpt-to-xml --test baseline
```

When a change _intentionally_ alters the XML output, regenerate the baselines and review the diff:

```sh
RPT_BLESS=1 cargo test -p rpt-to-xml --test baseline
```

Only commit baseline changes you can explain. A mismatch prints a git-style unified diff showing exactly which lines
changed.

## Adding test fixtures

Only add **publicly available** sample reports as fixtures or reports that you yourself made. Never commit reports
containing credentials, or any private data.

## Pull requests

- Keep changes focused; one logical change per PR.
- Make sure `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` all pass.
- Update [`CHANGELOG.md`](CHANGELOG.md) under `## [Unreleased]` for user-visible changes.
- Update the docs when you change behavior or add support for a record type.
