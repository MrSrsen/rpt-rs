# The `rpt` CLI

The `rpt` binary: inspection, JSON/KDL export, and a byte-level write path. Every command reads the file alone — no
database connection is ever needed.

Build the binaries first (`cargo build --release`); they land in `target/release/` as `rpt` and `rpt-render`. The
examples below assume they are on your `PATH` — otherwise call them by path (e.g. `./target/release/rpt`) or via
[Docker](../project/building.md#docker).

## Inspection

The `rpt-cli` app (`apps/rpt-cli`) builds the `rpt` binary. Most of its commands are read-only inspectors — they open
the compound file, decrypt and decode its streams, and report on them; three write-path commands (`anonymize` /
`reencode` /
`patch`, below) run the byte-faithful re-encoder to a new file. Every command takes a file; most also take `--json`
(`json-dump`, `kdl`,
`reencode` and `patch` emit a fixed format and warn that they are ignoring it).

```
rpt <COMMAND> <file.rpt> [--json] [--depth N] [--color | --no-color]
```

| Command           | What it prints                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
|-------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `inspect <file>`  | A one-screen summary: report version, summary info (title / author / timestamps / application), each chart's data binding (value + category fields), and a per-stream overview.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `inputs <file>`   | The report's external inputs — every parameter it defines, with its type, its default values and the last-used value saved with the report — in declaration order. A range value is written `[start..end]`, with a round bracket for an excluded or open end.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `tree <file>`     | A structural tree of the decoded records, grouped by source stream. Each node is tagged by kind — `CfbStream(<name>)` (first tier: the main report's `Contents`, then each subreport's `Subdocument N/Contents`), `Branch(<type>)` (a node with nested children), or `Leaf(<type>)` (a node with none) — where `<type>` is the registry name or raw `0xNNNN` word. For record types the decoder understands (the field-format records, the paper-size record, group-area options, summary definitions, OLE object items, a field object's special-field type, and cross-tab custom members) the node shows a concise decoded summary of the record's stored values (e.g. `DecimalPlaces=2 Negative=Bracketed`); every other record shows a truncated preview of its raw content. Node kinds use plain CFB/tree vocabulary, not project-specific terms. With `--json`, a recognized node carries the decoded values as a `decoded` object alongside `preview`.                                                                                                                                                                                                                                                                                                                        |
| `streams <file>`  | Record coverage per stream: how many **outermost** records the stream holds and how many its **record tree** holds, how many of the latter are still Unknown (undecoded), logical vs on-disk byte sizes, and the top record types (named in that stream's own vocabulary), plus what the saved-data path made of the report's stored rows. The meter for decode coverage. The two populations answer different questions, and the output names which is which: the outermost records are the linear walk, where a record's content spans the records nested inside it, and only its spans lie side by side — so the uncovered-byte account is read off it; the tree is every record at every depth (what `tree` and `dump` count), and the unrecognized-type census is read off it, since a record type that only ever occurs nested never reaches the linear walk.                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `dump <file…>`    | The byte-layout workbench for a record's raw bytes. Selects records by `--type` (hex `0x76` or a registry name like `Formula`) and dumps each one's **own field bytes**, demasked, as annotated hex plus the length-prefixed strings they contain, read through the reader's own scanner (`rpt_reader::raw::lp_strings`) so the workbench and the decoder cannot disagree about what is text. A record's content is a sequence of pieces — runs of its own field bytes and the records nested between them — and the hex joins the runs, so a record with children shows its seam offsets and bytes either side of a seam are not adjacent on disk. What follows the hex depends on the record type: one decoded from a declarative **field table** gets that table's own reading (every field's name, value and byte range, its skip runs left visible, and whether the table consumed the record exactly — the check a new table has to pass), while a type without one gets the **scalar-probe grid** (u16/u32, big- and little-endian, at every offset) — the instrument for locating a field no table names yet. With no `--type` it prints the stream's record-type index; given two or more files it also prints a byte-aligned minimal-pair diff of the first match in each. |
| `saved <file>`    | The report's **decoded saved-data rows**: the column schema (names + value types) and the cached rowset a report carries when saved with data (stored record order; not the engine's result rowset). `--schema` prints only the columns + record count; `--limit N` caps the rows (default 20; `all` for every row). The decoded-rows counterpart to `dump`'s raw bytes — reports the record count even when the batch class doesn't decode, so you know to `dump` the raw bytes. Only the **top-level** report's batch is decoded; a report whose rows live in a subreport's own subdocument is reported as such rather than as a report with none.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `sql <file>`      | **Every SQL statement the report can run against its database**, each tagged with where it came from: the engine's generated join `SELECT` (built from the table/link graph, pruned to the referenced tables, with the record-selection formula pushed into `WHERE`), each **stored SQL Command** (`Table.CommandText`) emitted verbatim, and each **SQL Expression field** — recursively through subreports. Also summarises the **connections** (server / database / driver / user) and the table list. `--dialect postgres` (default) `/ sqlite / mysql` picks the generated query's dialect. Static analysis — **no database connection is made**; it's the SQL the report *would* issue.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `formulas <file>` | **Every formula the report defines, listed and checked** — one line per formula marked `ok`/`warn`/`ERROR`/`empty`, with findings indented beneath, so you can see what was covered and not just how many; `--source` quotes each body. Covers formula fields, the record- and group-selection formulas, and conditional-format formulas wherever they hang (section, object format, border, font color), through every subreport. Reports syntax errors (the parser) and semantic ones (unknown functions, wrong arity, operator type errors) with the offending byte span. Exits 1 if any formula has an error, so it works as a CI gate. Reads the file alone — no database, no render. `--quiet` reports only through the exit status. This is the only thing that rejects a broken formula: everywhere else the parser recovers, the evaluator runs the partial parse, and the field renders blank.                                                                                                                                                                                                                                                                                                                                                                             |

`--json` emits the command's output as JSON instead of text, for scripting. `--depth N` (for `tree`) caps the tree at N
record levels (the stream tier is always shown); deeper nodes are collapsed to a `… N more` marker.
`--dialect D` (for `sql`) selects the generated query's SQL dialect (`postgres` (default) / `sqlite` / `mysql`).
`rpt <command> --help` prints scoped, per-command help. `rpt --version` (`-V`) prints the version the binary was built
from, which is also the first line of `--help`.

`dump` options: `--type T`, `--nth N` (0-based match index), `--stream S` (`contents` (default) / `qe` / `all` / a
stream-id substring), `--grid` (force the scalar-probe grid for a type that has a field table), `--probe N` (scalar-grid
cap, default 64; `0` off, `all` every byte shown), `--whole` (dump the whole masked on-disk span instead of the record's
own demasked field bytes), `--offset O --len L` (raw span escape hatch), the corpus-sweep trio
`--glob P` / `--cols SPEC,…` / `--anchor-string TXT` (one row per file×record, with columns pulled from absolute or
LP-string-anchored offsets), and `--saved` (inspect the saved-data batches: the decoded schema, the batch directory, and
each batch's derived decrypt IV + whether it inflates — a batch that does not decrypt reporting its directory entry and
derived-IV metadata, so an unmodelled batch class is still readable).

A record type number is **per stream**, so `dump` names and reads every record in the vocabulary of the stream it came
from: `0x0003` is `PrinterInfo` in `Contents` and `QeTable` in `QESession`, and each gets its own field table. Four
vocabularies are read — the report definition (`Contents`, and each subreport's own), the query-engine session
(`QESession`), the saved-data catalog (`DataSourceManager`) and the saved parameter values (`ReportParametersStream`) —
the last two reached by naming the stream, e.g. `--stream ReportParametersStream`.
`--type` accepts a name from any of them (`--type QeIndex`, `--type CurrentValueRecord`), since the name identifies its
own stream.

`tree` and `sql` colorize by **prominence** — for `tree`, recognized record types and field/text content are
highlighted, large embedded data blobs (images / saved data, shown as `[N B blob]`) are flagged in magenta, while
scaffolding (unknown types, small byte runs, tree connectors) is dimmed; for `sql`, section headers, table names, and
each statement's source are highlighted while scaffolding (indices, separators, field labels) is dimmed and the SQL
bodies stay plain for readability. Color is on by default when writing to a terminal and off when piped/redirected.
`--color` forces it on (e.g. to keep colors through a pager), `--no-color` forces it off; the `NO_COLOR` and
`CLICOLOR_FORCE` environment variables are also honored. The codes are standard ANSI SGR sequences — no extra
dependency.

```sh
# Quick look
rpt inspect report.rpt

# Parameters, machine-readable
rpt inputs report.rpt --json

# What's inside, as a structural tree (first 3 levels)
rpt tree report.rpt --depth 3

# Keep the colors when paging
rpt tree report.rpt --color | less -R

# How much of each stream is decoded (undecoded-record coverage)
rpt streams report.rpt

# Byte layout: annotated field bytes of the first formula record
rpt dump report.rpt --type Formula --nth 0

# Minimal-pair diff: which bytes moved between two near-identical reports?
rpt dump base.rpt variant.rpt --type 0x0121

# The decoded saved-data rows (schema + cached rowset)
rpt saved report.rpt --limit all

# Every SQL the report can issue (generated queries + stored commands), with provenance
rpt sql report.rpt

# The same as JSON (e.g. to extract just the stored SQL Commands), targeting SQLite
rpt sql report.rpt --dialect sqlite --json

# Check every formula for syntax and semantic errors, without rendering (exit 1 if any error)
rpt formulas report.rpt
```

`rpt formulas` exists because nothing else rejects a broken formula: the parser recovers from a syntax error, the
evaluator runs the partial parse, and the field renders blank — indistinguishable from a null value or an unimplemented
feature. It **lists every formula it checked**, marked `ok` / `warn` / `ERROR`, with any findings indented beneath — a
count alone would not tell you whether the formula you care about was among them:

```console
$ rpt formulas report.rpt
report.rpt
  ok     the record-selection formula                    crystal, 1 line
  ERROR  section Details's Section_Visibility formula    crystal, 1 line
           error: expected `)` at byte 14 (near `;`)
  empty  formula "Unused"                                crystal, 0 lines

1 formula checked, 1 declared but empty — 1 error, 0 warnings
```

`empty` is a formula field the report declares but left blank — listed so the accounting is complete, and excluded from
the checked count since there is nothing to verify. `--source` quotes each formula's body under its line:

```console
$ rpt formulas report.rpt --source
report.rpt
  ok     formula "Xcount"  crystal, 12 lines
         │ Local NumberVar Index;
         │ Local NumberVar Xcount := 0;
         │ For Index := 1 to Length({Product.Size}) Step 1 Do (
         │     If ({Product.Size}[Index] = "x") Then (Xcount := Xcount + 1;)
         │ );
         │ Xcount
```

Coverage is formula fields, the record- and group-selection formulas, and conditional-format formulas wherever they
hang — on a **section**, an object's format, its **border**, or a field/text object's **font color** — through every
subreport. (Sections are where most of them live in practice.) `--json` always includes each formula's source, kind,
syntax, and size, with no `--source` needed; `--quiet` reports only through the exit status.

## The write path (`anonymize` / `reencode` / `patch`)

Three commands run the **byte-faithful re-encoder** — the write path of the `rpt-reader` library — to a fresh `.rpt`.
All take an explicit output path and only ever write that one file; none edits in place. This is a record-level writer:
it round-trips and edits one decoded record's fields. There is **no model→records lowering** — you cannot mutate the
decoded semantic model and serialize it back; an edit names a field of one record.
(See [the support matrix](02-support-matrix.md)
and [the codebase](../project/codebase.md) for the boundary.)

| Command                                                 | What it does                                                                                                                                                                                                                                                                                    |
|---------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `reencode <in.rpt> <out.rpt>`                           | Re-encodes `<in.rpt>`'s `Contents` stream from its own logical bytes (a no-op writer round-trip) and writes `<out.rpt>`. The result re-opens to byte-identical record bytes; only the compressed file bytes differ (deflate is non-canonical).                                                  |
| `patch <in.rpt> <tag> <nth> <target> <value> <out.rpt>` | Locates the `<nth>` (0-based, pre-order) record of type `<tag>` in the `Contents` record tree, stores `<value>` in the field `<target>` names, then re-encodes to `<out.rpt>`. A value of a different width is written: the record's length prefix and every enclosing record's are recomputed. |
| `anonymize <in.rpt> <out.rpt>`                          | Removes personally identifying authoring metadata (below) and writes `<out.rpt>`. `--dry-run` reports without writing; `--json` for machine-readable output.                                                                                                                                    |

`patch` arguments: `<tag>` is the record type as hex (e.g. `0x64`) or decimal; `<nth>` is the 0-based occurrence of that
type in pre-order; `<target>` is the field to change, named as its record type's **field table** names it
(`group_indent`, `element_styles[3].weight` — `rpt dump <in.rpt> --type <tag> --nth <nth>` lists them); `<value>` is
read at that field's declared wire type — a decimal or `0x` number, a float, a string, `true`/`false`, or hex bytes for
an undecoded run.

A field is the addressable unit because a byte offset into a record is not a constant: it moves with the length of the
string before it, the count of the repeat before it, and the record's schema version. The table that says where a
field's bytes are also says how wide they are, so naming the field is both shorter and the only form that stays correct.

`<target>` also takes `@<offset>` — a byte offset into the record's **field bytes** (its runs joined, children spliced
out), with `<value>` as hex bytes whose length sets the region size. That form is **same-size only** and exists for
record types that have no field table. From Rust the same pair is `Rpt::patch_record_field` and
`Rpt::patch_record_bytes` / `Rpt::patch_record_bytes_resize` — see [the codebase](../project/codebase.md).

**The gate.** A field-addressed edit is refused, and nothing written, unless the record type's field table reproduces
that record byte for byte *and* the written record reads back with the named field at its new value and every other
field unchanged. The first is the evidence that the table accounts for the whole record, so nothing in it is left for an
edit to desynchronize; the second catches what no table can state — a field whose value decides how many rows a later
repeat has, or whether a later field is written at all.

A raw `@<offset>` edit has neither property available, so it falls back to the hand-maintained *cleared for safe
editing* allow-list (`crates/rpt-reader/src/io/cleared.rs`), to which a record type is added only with evidence that
editing it cannot desynchronize anything.

`--force` (`EditPolicy::Forced` from Rust) writes the edit anyway, skipping every check above. That is the right flag
when writing a deliberately invalid record is the point — probing what a field means — and the wrong one for editing a
report you intend to keep.

```sh
# Prove the writer round-trips: re-encode Contents to a new file
rpt reencode report.rpt out.rpt

# Set the first section's height, and rename it (a longer name grows the record)
rpt patch report.rpt 0x8c 0 height 1234 out.rpt
rpt patch report.rpt 0x8c 0 name "Details b" out.rpt

# Overwrite 2 bytes at offset 12 of the first ReportRoot (0x64) record's field bytes.
# 0x64 is not cleared for editing, so this is refused unless --force is passed.
rpt patch --force report.rpt 0x64 0 @12 01ff out.rpt

# See what authoring metadata a report carries, without writing anything
rpt anonymize report.rpt --dry-run
```

### Anonymizing a report

A `.rpt` records who made it and where. The OLE `SummaryInformation` property set holds the **author** and the **last
person to save**; a re-imported subreport holds the full path of the `.rpt` it came from (`\\HOST\user\Documents\…`).
None of it affects rendering, and all of it follows the file into any corpus it is committed to. `rpt anonymize` removes
it:

- **`author`, `last_saved_by`** — blanked. They are identity and nothing else.
- **`reimport.source_path`** — reduced to its **file name**, not blanked. A non-empty source path is the only evidence
  in the file that a subreport was imported at all, and `SubreportObject.IsImported` is resolved from it, so emptying it
  would silently turn a true fact false. The directory prefix is the identifying part; the bare file name is the
  subreport's own name, which the `Subdocument` storage already records.

The database connection's stored path is deliberately **left alone**: it is a live datasource locator, not authoring
metadata, and blanking it would break the report against its own data.

Every edit is **same-length** — a value's length prefix is untouched and only its characters are rewritten, then padded
with NULs, which readers of both formats stop at. No record length, property offset or section size moves, so the result
is a structurally identical file whose decoded model is unchanged apart from those fields, and which the real Crystal
engine still opens. A report with nothing to remove is returned byte-identical, so the command is idempotent and safe to
re-run across a corpus.

From Rust the same pass is [`Rpt::anonymize`], which returns the new bytes alongside a report of every value removed and
what it was replaced with.

[`Rpt::anonymize`]: https://docs.rs/rpt-reader/latest/rpt_reader/struct.Rpt.html#method.anonymize

## Export (`json-dump` / `kdl`)

Two subcommands export the whole decoded report, differing in how much they say. Both take an optional output path and
write to stdout when it is omitted; neither needs a database. They are thin callers of the `rpt-json` and `rpt-kdl`
libraries — the same surfaces you can call in-process (see [the codebase](../project/codebase.md)).

```
rpt json-dump <file.rpt> [out.json] [--strict]
rpt kdl       <file.rpt> [out.kdl]  [--strict]
```

- **`json-dump`** writes the **exhaustive** JSON document: the full serde serialization of the decoded model under
  `model` — every field, including defaults, and the whole subreport tree. **Stored facts only**: nothing inferred or
  recomputed, so a change in the output always means a change in the decode. Output is deterministic (sorted-key maps,
  two-space indent, trailing newline) and depends on nothing outside the `.rpt`, which is what makes it usable as a
  regression baseline and as one side of an engine comparison. An embedded picture's bytes are a **lowercase hex
  string** (two characters per byte, byte *i* at character *2i*), not an array of integers — so anything parsing the
  dump has to decode `data` from hex. Every byte is still there, which is the point: a picture that changes still moves
  a baseline.
- **`kdl`** writes the model as a [KDL](https://kdl.dev) document: a sparse, human-readable, lossless view. Given an
  output path it also writes each embedded picture beside it as a sidecar file (the KDL carries a `source=` reference
  instead of the bytes); on stdout the pictures are referenced but not written.

```sh
# Everything the decoder knows, as JSON
rpt json-dump report.rpt out.json

# The same model, human-readable
rpt kdl report.rpt out.kdl
```

Use `json-dump` for scripting, structural diffs, and regression baselines; `kdl` for reading and reviewing a report
definition in version control. To see the raw record tree instead of the model, use `rpt tree`.

**Incomplete decodes.** Building the model is infallible by design — a record the reader does not recognize becomes a
default rather than an error — so an export missing content otherwise looks exactly like a faithful one. Both commands
warn on stderr (never stdout, so a piped export is unaffected) when the report did not decode completely, naming the
unrecognized record types (or the saved-data batch that would not decode) and pointing at `rpt streams` for the
breakdown. `--strict` makes that a failure instead (exit 1) for CI, where a silent partial export is worse than a
stopped build; the document is still written, so it remains available for working out what is missing.

---

← [Support matrix](02-support-matrix.md) · [Index](README.md) · **Next:** [The `rpt-reader` library](04-library.md) →
