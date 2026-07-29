# Contributing to rpt-rs

Thanks for your interest in improving rpt-rs. This document covers how to build, test, and structure changes.

## Getting started

You need a Rust toolchain (the minimum supported version is **1.92**). Then:

```sh
cargo build            # build the workspace
cargo test             # run the test suite
cargo fmt --all        # format
cargo clippy --all-targets --all-features -- -D warnings   # lint (CI fails on warnings)

# Two guards CI also runs, for rules clippy cannot express:
bash scripts/check-workspace-deps.sh     # no bare per-crate version of a workspace dependency
bash scripts/check-error-handling.sh     # no discarded parse diagnostics; no double-printed causes
```

The workspace forbids `unsafe` code (`unsafe_code = "forbid"`); keep contributions safe Rust.

## Workspace layout

The reader crates, and the boundary between them, are intentional:

| Crate             | Role                                                                                                                                                                                                      |
|-------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `rpt-reader`      | Pure I/O. Decodes the **stored** facts from the bytes: container → records → typed model.                                                                                                                 |
| `rpt-model`       | The format-neutral, pure-data semantic model `rpt-reader` produces — no I/O, no decoder dependency. |
| `rpt-formula` | The standalone Crystal/Basic formula-language engine (no `.rpt` coupling).                                                                                                                                |
| `rpt-json`        | The exhaustive JSON export **library** (the `rpt json-dump` surface) — the decode regression surface. A pure projection of the decoded model: stored facts only, nothing derived. |
| `rpt-kdl`         | The sparse, human-readable KDL authoring projection of the same model (the `rpt kdl` surface). |
| `rpt-cli`         | The `rpt` inspection CLI; its `json-dump` / `kdl` subcommands are thin callers of those two libraries. |

**The boundary is load-bearing.** If a value is read directly from the file, decode it in `rpt-reader`. If a value is
_computed or inferred_ (not present in the bytes), it is computed on demand by the consumer that needs it — never as a
stored field on an `rpt-model` struct, and never in the export. Keeping I/O separate from derivation is the core design
rule, and keeping derivation out of `json-dump` is what makes a baseline diff mean "the decode changed".

## The `.rpt` format

`.rpt` is an undocumented, encrypted binary format. If you're working on decoding, start with the documentation in
[`docs/`](docs/):

- [Format overview](docs/format/01-overview.md), [container](docs/format/02-container.md),
  [stream decoding](docs/format/03-stream-decoding.md), [record tree](docs/format/04-record-tree.md), and
  [semantic model](docs/reader/01-semantic-model.md) explain the pipeline end to end.
- The [block catalog](docs/format/06-block-catalog.md) and [support matrix](docs/reader/02-support-matrix.md) describe
  each decoded record type and what is and isn't supported.

## Tests and JSON baselines

The regression suite in `apps/rpt-cli/tests/json_baseline.rs` runs `rpt json-dump` over every sample report under
`tests/fixtures/reports/` and compares the output against committed baselines. The dump is the **full** serde
serialization of the decoded model — every field, including defaults — so any change to any decoded value shows up as a
diff, and nothing else does.

It needs no sandbox: the dump takes no locale, `source_path` is a stored field rather than the invocation path, and
maps are emitted in sorted-key order, so the output is byte-identical on every machine and platform.

```sh
cargo test -p rpt-cli --test json_baseline
```

When a change _intentionally_ alters the decoded model, regenerate the baselines and review the diff:

```sh
RPT_BLESS=1 cargo test -p rpt-cli --test json_baseline
```

Only commit baseline changes you can explain. A mismatch prints a git-style unified diff showing exactly which lines
changed — and because the dump omits nothing, a one-line fix that moves values you did not expect is the harness
telling you the change was broader than you thought.

## Adding test fixtures

Only add **publicly available** sample reports as fixtures or reports that you yourself made. Never commit reports
containing credentials, or any private data.

## Pull requests

- Keep changes focused; one logical change per PR.
- Make sure `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`, and the three `scripts/check-*.sh` guards pass.
- Update [`CHANGELOG.md`](CHANGELOG.md) under `## [Unreleased]` for user-visible changes.
- Update the docs when you change behavior or add support for a record type.

## Releasing

The version lives in exactly one place: `version` under `[workspace.package]` in the root `Cargo.toml`. Every crate
inherits it (`version.workspace = true`), and the binaries compile it in via `CARGO_PKG_VERSION` — which is what
`rpt --version` and `rpt-render --version` print, and what their `--help` headers carry. A git tag names a commit; it
never supplies the version, so a binary reports the same thing whether it came from a release, a checkout, or
`cargo install`.

To cut a release:

1. Bump `[workspace.package] version` in the root `Cargo.toml`, and `cargo build` so `Cargo.lock` follows.
2. Roll `## [Unreleased]` in [`CHANGELOG.md`](CHANGELOG.md) into `## [x.y.z] - YYYY-MM-DD`.
3. Check the tag you are about to push before you push it:

   ```sh
   bash scripts/check-release-version.sh v0.4.0
   ```

4. Commit, then tag the commit `vx.y.z` and push the tag. CI's `create-release` job runs the same check and fails the
   release if the tag and the manifest disagree, so a mistyped tag cannot ship binaries that name a different version.
