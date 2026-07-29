# Building

How to build, test and gate this workspace — and, where a choice looks arbitrary, why it is what it is.

## Toolchain

You need Rust **1.92** or newer. The floor is checked in CI by a job that builds on exactly that version, so it cannot
drift by accident.

The rule for moving it: **the MSRV rises when a dependency we want requires it, and for no other reason.**
1.92 is what `krilla` 0.8.2 needs. State the reason whenever it changes — a bare number with no rationale is how the
previous floor (1.89) outlived the thing that set it, holding back a PDF library nobody could name a reason to hold
back.

One consequence worth knowing before you widen a version requirement: `kdl` is pinned below 6.6 because 6.6+ needs 1.95.
That cap is a live question rather than a fixed constraint — if something wants newer
`kdl`, raising the floor is an argued decision, not a blocker.

## The basics

```sh
cargo build                # the whole workspace
cargo test                 # every test; DB-backed ones skip without RPT_DB_URL (see below)
cargo fmt --all            # format
cargo clippy --all-targets --all-features -- -D warnings
```

Two guard scripts encode invariants clippy cannot express, and CI runs both:

```sh
bash scripts/check-workspace-deps.sh    # every shared dep inherited via { workspace = true }
bash scripts/check-error-handling.sh    # discarded formula diagnostics; doubly-printed error causes
```

A third script, `scripts/check-release-version.sh v<x.y.z>`, guards a release rather than a change: it runs in CI only
on a version tag (see [CI](#ci)) and is worth running by hand before pushing one.

`THIRD-PARTY-NOTICES.md` is generated, not hand-edited — regenerate it with `scripts/gen-third-party-notices.sh`
(needs `cargo install cargo-about --locked --features cli`) whenever a dependency is added, removed or bumped. It lists
every crate linked into the binaries, across all five release targets, plus the two bundled font licences, and both the
release archives and the Docker image ship it. `--check` fails if the committed file is stale; the release job runs
that before creating a release.

The dependency guard exists because a member crate re-declaring a `[workspace.dependencies]` crate with a bare version
silently compiles **two copies** that can drift apart — a failure that shows up as a baffling type mismatch, or worse,
as two font databases disagreeing.

## Docker

A multistage build produces a `scratch` image containing the two statically linked binaries and, under
`/usr/local/share/licenses/rpt-rs/`, the `LICENSE` and `THIRD-PARTY-NOTICES.md` the image redistributes on their
behalf (there is no shell to read them with — extract them with `docker cp`):

```sh
docker build -t rpt-rs .
```

Mount the directory holding the report as `/data` and run either of them:

```sh
docker run --rm -v "$PWD:/data" rpt-rs rpt inspect report.rpt
docker run --rm -v "$PWD:/data" rpt-rs rpt-render report.rpt -o out.pdf
```

The image has no system fonts, and it does not need any: a render uses the metric-compatible Liberation faces bundled in
`rpt-text` by default, so it produces the same output there as anywhere else. (Passing `--system-fonts` inside the image
finds nothing to scan and falls back to the same bundled faces.)

## Feature flags

There are few, deliberately, and none of them changes what a correct build *produces*:

| Crate                                         | Feature                                   | Effect                                                                                                                                           |
|-----------------------------------------------|-------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------|
| `rpt-reader`, `rpt-model`, `rpt-format-value` | `serde`                                   | Derive `Serialize`/`Deserialize` on the model and its value types.                                                                               |
| `rpt-text`                                    | `cosmic` (default)                        | The cosmic-text shaping stack (`CosmicLayout` + `FontProvider`). With it off, only the `FontDb` face resolver the PDF backend needs is compiled. |
| `rpt-render-cli`                              | `db-postgres`, `db-sqlite` (both default) | Compile the live-DB backends. The `rpt-render` **library** never links a driver, so there is nothing to turn off there.                          |
| `rpt-formula`                                 | `differential`                            | Expose the tree-walking evaluator (the VM's differential-test reference) to external test crates.                                                |

`rpt-render` has **no** features, deliberately: every flag above changes *linkage*, and none changes *output*. A flag
that selected the text layout would change where a report paginates, so two builds of the same commit could disagree on
its page count — a build knob that moves the page breaks is not one worth having.

`ApproxLayout` is still useful — it reads no fonts at all, which is why the data-driven render baselines pass it
explicitly to `render_dataset_with`. It is chosen at the call site, never fallen into by omission.

## Targets

`wasm32-unknown-unknown` is a first-class target for the pipeline, and CI builds 13 crates for it on every push: the
whole render/data path plus its dependency-free leaves. Two crates are excluded **by design**
rather than by defect — `rpt-db-postgres` and `rpt-db-sqlite`. Native database clients live behind the
`RowSource` trait precisely so the portable core never links them.

Font handling needs no filesystem: the Liberation faces are compiled in with `include_bytes!`, and the bundled set is
the default source. A WASM host gets real shaping and real metrics, not an approximation.

`rpt-reader` and `rpt-json` happen to compile for wasm32 today but are **not** gated, because they are reader-side and
their portability is not a promise — gating them would turn an accident into a contract.

## Fonts, and why a render can differ between machines

By default a render uses the **bundled** Liberation/DejaVu faces, so the same report produces the same geometry
everywhere. `rpt-render --system-fonts` opts into the host's installed library instead, which is what you want when the
host has the report's real fonts — and which makes the output a property of that machine.

When something looks wrong, start here:

```sh
rpt-render --list-fonts              # count, source, generic mappings, searched directories
rpt-render --list-fonts -v           # every face with its path
rpt-render --list-fonts --system-fonts -v
```

That prints the directories it searched **whether or not they exist**, which is usually the answer: a font that is
absent because its directory was never there looks identical, in a render, to a font that failed to load.

## Tests, and the four layers

The pipeline is tested at four boundaries so a failure names the stage that broke rather than requiring a bisection:

| Layer   | Harness                                        | Isolates                                                             |
|---------|------------------------------------------------|----------------------------------------------------------------------|
| **L1**  | `apps/rpt-cli/tests/json_baseline.rs`          | the decoder — the full serde dump of the decoded model               |
| **L2**  | `crates/rpt-render/tests/dataset_baselines.rs` | `rpt-data`: selection, sort, grouping, summaries, formula evaluation |
| **L3**  | `crates/rpt-render/tests/postgres_fixtures.rs` | layout and pagination — the Page IR                                  |
| **L4a** | `crates/rpt-render/tests/pdf_baselines.rs`     | the PDF writer's serialization, as an operator listing               |
| **L4b** | `crates/rpt-render/tests/pdf_artifacts.rs`     | the finished artifact: structure, `/Widths`, does a reader parse it  |

`crates/rpt-render/tests/typography_baselines.rs` is a sixth harness at L3, over the data-free typography fixtures.

Every layer but L4b compares against a committed baseline and blesses with `RPT_BLESS=1`; L4b asserts relationships
within the artifact instead, so it has nothing to bless:

```sh
RPT_BLESS=1 cargo test -p rpt-cli --test json_baseline
RPT_BLESS=1 cargo test -p rpt-render --test pdf_baselines
```

**Read the diff before blessing.** A one-line change that moves forty unrelated values is the harness telling you the
change was broader than you thought.

Only `postgres_fixtures` needs a database. Every other harness — including the L3 typography one, whose fixtures bind no
datasource — is hermetic and runs in the plain `cargo test` job.

This table is the layer map; what each corpus *contains* — the Meridian universe, the deprecated legacy fixtures, and
what every fixture at each layer buys — is [Testing the renderer](../rendering/09-testing-parity.md).

## The database-backed corpus

```sh
docker compose up -d --wait
export RPT_DB_URL=postgres://rpt:rpt@localhost:55432/rptfixtures
cargo test -p rpt-render --test postgres_fixtures
```

**`postgres_fixtures` skips silently when `RPT_DB_URL` is unset.** A DB-less `cargo test` therefore reports green while
verifying nothing at that layer. Every harness asserts a non-zero fixture count in both check and bless mode to stop
that being mistaken for coverage — but if you are relying on L3 having run, check that it did.

## CI

Ten jobs. Eight gate a release: `check`, `test`, `msrv`, `feature-matrix`, `baseline`, `cargo-deny`,
`render-fixtures` and `wasm` all have to pass before `create-release` runs on a version tag.

`create-release` first checks that the tag matches the version the binaries carry (`scripts/check-release-version.sh`,
runnable locally with the tag you are about to push). The version has one source —
`[workspace.package] version` in the root `Cargo.toml`, inherited by every crate and compiled in as
`CARGO_PKG_VERSION` — so the tag is verified against the manifest rather than injected into the build, and a mistyped
tag fails before the release exists instead of shipping binaries whose `--version` names something else. It then
checks that `THIRD-PARTY-NOTICES.md` still matches the lock file, since every archive ships a copy of it.

`render-fixtures` gating is deliberate despite needing a service container — it already runs on every push, so it costs
no extra CI time, and for a release whose claim is that the render pipeline is trustworthy it is the most consequential
of the three that were once ungated.

A tagged release also emits **SLSA build provenance** for every artifact it uploads. The checksum beside an archive is
computed by the same job that produced it, so it attests to nothing an attacker inside that job could not forge; the
attestation is signed with an OIDC identity tied to this workflow and commit, so a consumer can verify *where* a binary
was built:

```sh
gh attestation verify rpt-rs-vX.Y.Z-<target>.tar.gz --repo <owner>/<repo>
```

A tagged release builds the binaries, splits their debug symbols into a sidecar, and uploads the symbols as a **separate
asset** — so the ordinary download is small and symbols are fetched only to decode a trace. Drop the sidecar next to the
binary and backtraces resolve names and source lines again; delete it and the binary still runs, still reporting the
panic message and where it was raised (that much comes from `#[track_caller]` data compiled in, not from debug info). To
symbolicate an archived trace offline:

```sh
addr2line -e rpt-render.debug -f -C <addr>
```

Every action is pinned by commit SHA with its version in a trailing comment. Note that
`dtolnay/rust-toolchain` derives its Rust version from the ref it is invoked with, so pinning it to a SHA removes that
signal — every call site states `toolchain:` explicitly, and the `msrv` job is what proves the wiring, since a dropped
input would otherwise just build on the runner's default.

---

← [The codebase](codebase.md) · **Back to the** [project index](README.md)
