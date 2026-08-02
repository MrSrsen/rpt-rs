# Changelog

All notable changes to rpt-rs will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),

## [Unreleased]

WIP.

## [0.4.0]

More than 340 tracked issues went into this release.

Its focus is the **`rpt-reader` rewrite**: every record type now decodes through a declarative field table — a stated
sequence of typed reads — in place of the byte-offset arithmetic, string scans and landmark probes that came before. A
positional reading is right about the corpus by construction; a transcribed one is right about the format, and the
difference only shows on a report the corpus does not contain. The rewrite corrected a long tail of such readings, and
seven computed values came off the semantic model with it, so the decode surface is now stored facts and nothing else.

The other half is subtraction. The **raster, SVG and HTML backends are gone**, each having earned little practical use
beside the PDF one, and each a second-class view of the same pages — painting from metrics the layout engine never
measured with, or rounding twips away. **PDF is now the only output, and the one the project will build on.** It gains
tagged output, PDF/UA-1 and PDF/A, real gradient and hatch fills, hierarchical grouping, per-section paging limits, and
a large body of chart, date/time and numeric parity work measured against the native engine. Four test layers — decode,
`Dataset`, Page IR, PDF content stream — now pin the pipeline, so a failure names the stage that broke.

### Breaking changes

**Toolchain and features**

- **The minimum supported Rust version is 1.92** (was 1.89).
- Three Cargo features are gone, each having become unconditional: `rpt-pages`'s `json`, `rpt-reader`'s `std`, and
  `rpt-render` / `rpt-render-cli`'s `cosmic`. `--no-default-features` no longer yields an `ApproxLayout`-only render.
- **`crystal-formula` is renamed `rpt-formula`.** Change the dependency key and every `crystal_formula::` path, or add a
  Cargo rename. Nothing inside the crate moved.
- **`rpt_formula::func_id` is replaced by `is_builtin`.** The formula type system no longer keys its return-type,
  string-width and arity rules through a numeric id: each builtin now carries all three in one name-keyed table, so
  there is no id to hand out. Callers only ever asked whether a name was known, which `is_builtin(&str) -> bool`
  answers directly.

**Seven derived values leave the semantic model, the JSON dump and the KDL export.** Each was computed rather than read,
so a change to the derivation moved decode baselines with the bytes untouched. A consumer that needs one derives it:

- `Section::section_code` and `ReportObject::section_code` — nothing consumed them.
- `PictureObject::{original_width, original_height, x_scaling, y_scaling}` — call `rpt_model::natural_extent`.
- `FieldObject::value_type` is populated for summary objects only; the rest resolve through
  `rpt_model::analysis::field_object_value_type`.
- `LineShape` / `BoxShape`'s `end_section_name` → `DrawingShape::end_section_index`, which is stored rather than
  rediscovered by walking heights.
- `DrawingShape::{line_style, line_color}` and `BoxShape::fill_color` mirrored `ReportObject::border` — read that.
- `ParameterField::discrete_or_range_kind`; and `prompt_text` is `None` rather than a synthesised `"Enter {name}:"`.
- `CrossTabObject::measures` — call `rpt_model::crosstab_measures(&Report)`.

**Reader API**

- `FieldFormat::currency_numeric` is serialized and `numeric` is no longer swapped: a field stores **two** numeric
  slots, both decoded verbatim. Read `currency_numeric` for a Currency field.
- A field heading left at `Alignment::DefaultAlign` keeps its stored byte; the rules that rewrote it moved to
  `rpt-layout`. No rendered output changes.
- Cross-tab count words are named for what they count: `CrossTabGridFormat::raw` → `cell_count`, and
  `CrossTabObject::{column,row}_axis_options` → `{column,row}_level_count`. Values unchanged, so every cross-tab
  baseline moves as a pure key rename.
- `StreamCoverage::records` → `outermost_records`, plus a new `tree_records`; `unknown_records` / `unknown_types` now
  count over the whole tree. `rpt streams --json` follows.
- `RecordNode::leaf_bytes` → `joined_runs` and `raw::Part::Leaf` → `Part::Run` (likewise `FieldRead::leaf` → `joined`,
  `leaf_segments` → `run_spans`): the buffer splices nested records out, so a fixed offset into it can address bytes
  that are not adjacent on disk. `RecordNode::runs` / `first_run` are what a positional read wants.
- Renamed without change of meaning: `Rpt::record_dom` → `typed_record_tree`, `subreport_record_doms` →
  `subreport_typed_record_trees`, `RecordStream::tile_logical` → `from_logical_bytes`, `patch_record_leaf[_resize]` →
  `patch_record_bytes[_resize]`, `SavedBatchInspection::dir_leaf` → `dir_entry`, `Error::Project` → `Error::Edit`,
  `ProjectErrorKind` → `EditErrorKind`.
- `raw::Dialect`, `fields::FieldKind` and `fields::FieldValue` are `#[non_exhaustive]`; `annotate::summarize`,
  `raw::Unknown::type_name`, `RecordTag::name` and `RecordTag::is_known` take a `Dialect`, since a type number is per
  stream.
- `Report` gains `authoring_version`; `Rpt::record_fields` returns `Result`, not `Option`.
- Decode errors can no longer be constructed outside the crate: `ContainerError`, `CryptoError`, `CodecError` and
  `NotAReportError` are `#[non_exhaustive]` with crate-private builders. Inspecting and matching are untouched.
- Removed: `rpt_reader::prelude`, `raw::Value::Int`, `FieldKind::Custom`, `RecordFields::ranges_verified`, `Serialize`
  on `DecodeCoverage` / `StreamCoverage`, `Rpt::{save, write, saved_index}`, `install_panic_hook` and the `diagnostics`
  module, and `RecordStream::qe_record_tree`'s visibility. The lossless guarantee sits on `Rpt::original_bytes`;
  `EditPolicy` moved to `io::edit` with its root re-export unchanged.

**CLI**

- **The JSON dump encodes a picture's bytes as a lowercase hex string**, not an array of integers — decode `data` from
  hex. It round-trips exactly, and the committed L1 baselines drop from 327 MB to 42 MB.
- **`rpt patch` edits by field name, not byte offset**: `rpt patch <in.rpt> <tag> <nth> <target> <value> <out.rpt>`,
  `<target>` being the field table's name (`group_indent`, `element_styles[3].weight`). Such an edit may change width,
  and the clearance gate is now a property the file demonstrates rather than a hand-maintained list. `@<offset>` still
  takes a raw same-size overwrite on a table-less type.
- `rpt dump`'s field-kind column names its byte order (`u16be`, `varu32`, …); decoding is byte-identical.
- `rpt streams`' `top types:` histogram names each type in its stream's vocabulary and counts over the tree — anything
  scraping it for a hex word breaks.
- `rpt inputs` prints a `current:` line per parameter, and both `default:` and `current:` render a range in interval
  notation (`[1..100]`, `[1..)`).

**Render pipeline**

- **PNG, HTML and SVG output are gone**, with the `rpt-render-raster` / `-html` / `-svg` crates and every facade entry
  point that named them. Render to PDF.
- `-f` / `--format` and extension inference go with them: `-o` names a path and any name is accepted, though an
  `.html` / `.svg` / `.png` path is refused by name. Default output is PDF (was HTML on a bare stdout run); `--force` is
  now an unknown option.
- `PdfWriter`, `PdfOptions::writer` and `render_pages_basic` are gone — krilla is the only writer. `PdfOptions` gains
  `producer` and is no longer `Copy`.
- `rpt-render-util`'s markup-era surface goes with the markup backends: `base64_encode`, `escape_xml_text`,
  `escape_xml_attr`, `dash_pattern`, `TWIPS_PER_PX`, `TWIPS_PER_INCH` and `POINTS_PER_INCH`, plus `JustifyUnit`'s `f32`
  instantiation. `rpt-layout` drops the dependency.
- **Renders use the bundled faces by default, not the host's installed fonts** — both the metrics layout measures with
  (`RenderOptions::fonts`, new) and the faces the backend embeds (`PdfOptions::fonts`). A report naming an installed
  font can shift its wrap points, can-grow heights and therefore **page count**, and embeds `LiberationSans` where it
  embedded `ArialMT`. Scanning the host made the output a property of the machine; `rpt-render --system-fonts` asks for
  it back. This moved no committed baseline.
- New fields needing an exhaustive literal, all defaulting to previous behaviour: `TextRun::character_spacing`,
  `PagedDocument::sections`, `GroupInstance::{date_condition, hierarchy_children}`, `NumberFormat::{reserve_sign,
  reverse_sign, zero_value}`, `TimeFormat::am_pm`. Every stored Page IR dump still deserializes.
- `FormatSpec::DateTime` is a struct variant `{ date, time, separator, time_first }`.
- `ReportDocument::export_pdf_to_disk` returns `Result<(), rpt_render::ExportError>`; writing a PDF never touches the
  reader. The message is unchanged.

### Added

**Reader**

- `rpt_model::AuthoringVersion` / `Report::authoring_version` — which Crystal Reports wrote the file.
- **The saved-data path says why it decoded nothing**, where every failure used to produce `saved_data: null`.
  `Rpt::saved_data_status` names the reason — no catalog, no stored field, a missing row stream, an undecryptable batch,
  a short rowset. It rides on `DecodeCoverage`, so a lost batch fires `warning()` and `rpt json-dump --strict` refuses.
  Nothing enters the model or the dump.
- **`rpt_reader::fields`** — the field-table reading as a public accessor (`read`, `table_name`, `tabled_types`), with
  the table machinery still private, plus `RecordStream::fields` / `field` as the primary reading.
- **`raw::RecordSearch`** — the record-finding primitive the format uses: ask by type, bounded by the container's end
  marker, stepping over anything else. That is what tells an optional record never written from running off the end, and
  why a file written by a newer engine opens in an older one.
- `Dialect::ALL` and a `Catalog` variant, `RecordTag::{from_name, label}`, `SummaryOperation::{token, full_name}`.
- **The chart definition's styling block decodes** — bar/pie/marker sizing, marker shape, colour mode, legend layout,
  data point, data-value number format, two more gridline modes, and a per-axis run (min, max, number format, auto range
  and scale, division method and count) for the value, secondary value and series axes. Nine new enums carry an
  `Other(u8)` arm so an unknown code is kept rather than folded into a neighbour. `ChartElementFont` gains `weight`,
  `italic` and `is_default`, and `element_fonts` carries all ten elements.
- **The `0x00f6` time leaf is fully decoded** — `time_base`, `am_pm_format`, `am_string`, `pm_string` and both
  separators, all six previously documented as absent.
- `DateFieldFormat` decodes `era`, `calendar`, `day_of_week_position`, `day_of_week_enclosure` and the five literal
  separators, so a date format can be reproduced from the model.
- `NumericFieldFormat` decodes `use_lead_zero`, `display_reverse_sign`, `one_currency_symbol_per_page` and
  `zero_value_string`.
- `ReportDefinition::kind` as a stored fact rather than a restatement of the multi-column geometry, and
  `AreaFormat::visible_records_per_page`, which had no decode path at all.
- **The report's creation, last-saved and last-printed timestamps are decoded** — the `\x05SummaryInformation` parser
  read string properties only, so each `VT_FILETIME` fell through.
- **`rpt dump` shows a record type's field-table reading** where a table exists — name, wire type, value and byte range
  per field, then whether the table consumed the record exactly and where it stopped. `--grid` forces the old probe
  grid; `--json` carries the same under `fieldTable`, with a new `int` key for signed fields.

**Render**

- **Tagged PDF, PDF/UA-1 and the accessible PDF/A levels.** `PdfOptions::tagged` emits the structure tree assistive
  technology needs, with reading order recovered per band by row; draws stay in paint order, so appearance is unchanged.
  The levels are **earned, not selected**: what the report genuinely lacks — language, title, alternate text per figure
  — comes from `PdfOptions::semantics`, and the render is **refused**, naming what is missing, when it does not.
  Verified with veraPDF 1.30.2 against ISO 14289-1: 56 corpus reports pass, none fail, 48 are refused for undescribed
  figures. `rpt_render::semantics_of(&Report)` fills in what the file states; the CLI adds `--tagged`, `--pdfua`,
  `--pdfa 1a|2a|3a`, `--lang`, `--title` and `--alt <Object>=<text>`.
- **PDF/A archival output — `rpt-render --pdfa 1b|2b|3b`, or `PdfOptions::conformance`** — as a checked claim: a
  document that does not meet the level fails with `PdfError::Conformance` naming each unmet requirement rather than
  being written with a claim it does not honour. `PdfOptions::created` sets the required date; without one a conforming
  render falls back to the Unix epoch so its bytes stay reproducible.
- **Every rendered PDF names the engine that produced it** in `/Producer`, `/Creator` and the XMP equivalents. It costs
  no reproducibility — the identity is a build constant, never a clock, host or path. `PdfOptions::producer` rebrands or
  opts out.
- **Hierarchical grouping renders as the tree it describes** rather than flat in raw key order: a depth-first walk from
  the roots, siblings in the group's own sort order, each subtree bracketed by its own header and footer, every object
  shifted right by `depth × GroupIndent`. Malformed hierarchies lay out rather than hang or vanish — an orphan and a
  self-parenting instance are roots, a cycle terminates, every instance prints once.
- **The per-section paging limits paginate.** "Records per page" and "Groups per page" were decoded but read by nothing.
  A stored `0` still means no limit; the record cap breaks before the next group's header, so it is not orphaned.
- **`OneCurrencySymbolPerPage` is applied.** It cannot live in the value formatter — a band's text resolves before the
  page-break decision that follows it — so it is a post-pagination pass over the same fixup list `TotalPageCount` uses.
  The Page IR is unchanged.
- **Real gradient and hatch fills in the PDF backend**, as a PDF axial shading and a real tiling pattern; both
  previously degraded silently to a representative solid. Nothing in the pipeline constructs either fill yet.
- **The Page IR carries the report's section structure and paragraph character spacing.** `PagedDocument::sections` lets
  a consumer tell a running page header from document content. A backend that re-shapes must add `character_spacing` per
  *scalar in each shaped cluster*, never per glyph, or a ligature desynchronizes the drawn width from the measured one.
- **A font seam over both halves of the stack** — `RenderOptions::fonts` and `PdfOptions::fonts`, plus
  `FontProvider::from_source` so one value configures both and they cannot drift. **`rpt-render --list-fonts`** prints
  the library a render would use: face count, origins, directories searched, `--verbose` adding a line per face.
- **`try_render_pages` / `try_render_pages_with_assets`** (with the new `PdfError`): entry points that *report* a
  serialization failure instead of absorbing it. The infallible signatures are unchanged.
- **`rpt --version` and `rpt-render --version`**, taken from `[workspace.package]` so the printed version is the one the
  binary was built from.

**Tests**

- **Committed `Dataset` snapshot baselines (L2)** — eight fixtures diff columns and value types, surviving `row_count`,
  parameter values, each formula field with its classified evaluation time, the fail-open diagnostics, the grand total,
  and the indented group tree. Values are **typed, never formatted**, so the host locale cannot enter the baseline.
- **Committed PDF content-stream baselines (L4a)** — eight fixtures diff a normalized operator listing, so with the Page
  IR green a diff here means the *writer* changed. Both harnesses read their own saved data with fonts pinned to the
  bundled faces, so neither needs a database or host fonts.
- `rpt_test_support::pdf` — read-only inspection of rendered PDF bytes (`inflated_content`, `operator_listing`).
- The HTML backend's 73 committed baselines are replaced one-for-one by **Page IR** baselines blessed from the same
  renders — strictly stronger, since HTML rounded twips to CSS px and so could not see a sub-15-twip shift.
- The field-table harness fails a table whose two fields share a name, and a version-gate census measures the spread of
  schema words the corpus stores per record type.
- **`THIRD-PARTY-NOTICES.md`**, generated from the lock file by `scripts/gen-third-party-notices.sh` and shipped in
  every release archive and in the Docker image. The binaries are statically linked and embed the bundled fonts, so an
  archive redistributes its whole dependency tree; it previously carried only this project's own `LICENSE`, and the
  image carried no licence text at all. `metafile` also ships the `LICENSE-MIT` / `LICENSE-APACHE` texts its declared
  dual licence needs.

### Changed

The decoder's readings are now **declarative**: a record type states its field sequence once, and the reader walks it. A
positional reading is right about the corpus by construction; a transcribed one is right about the format, and the
difference only shows on a report the corpus does not contain. Every table accounts for its corpus records exactly and
re-emits them byte for byte; no decode baseline moves unless an entry says so.

**Framing**

- **A record's schema word is an opaque version number**, read whole and compared numerically, so a reader accepts
  anything up to the newest layout it knows and adapts to anything older. Its high byte identifies nothing.
- **A record type with a field table declares its children**, so a tabled type stops being scanned. The tree is
  byte-identical. The two scan filters underneath stay until the last untabled type retires.
- **A record says which of the two string wire formats its content uses**, honoured per record instead of assumed. All
  374,539 corpus records declare the enhanced form, so nothing decodes differently — but a writer must set it.
- **An empty record must state its version to be recognised as one**; admitting the narrowest header wherever a table
  declared the type turned bytes of a save timestamp into two child records.
- **A version ceiling belongs to a record, not to its number**, keyed by dialect and type the way the registry routes.
- **The typed record tree carries a record's content in wire order** — `parts: Vec<Part>`, alternating runs of field
  bytes and the records nested between them, replacing the separate `values` / `children` lists.
- **The wire-type vocabulary names its byte order** (`U16Be`, `VarU32`, …), since the format is not uniformly
  big-endian; a table can state "a string" independently of how it is spelled, and a field reference as one composite.

**Record readings**

- **Two of twenty record types read the wrong field width or flag**: `0x006c MultiColumn`'s four `i32` dimensions were
  read as 16-bit slices, and `0x00b8 CrossTabObject` has no cell-margin flag — the byte read as one is the third byte
  of the grid width, so `ShowCellMargins` comes from the `0x00d6` grid cells.
- **Six `Skip` runs were variable-length structures** and are decoded as such. Each was the right length only because
  every corpus report stores the variable parts empty.
- **Thirteen more types moved to field tables**, led by the chart definition and the whole cross-tab family. The chart's
  pie and 3-D families become conditional *fields*, the chart subtype is read as the narrowing integer it is (so a
  subtype at or above 256 keeps its high bits), and a cross-tab level's bound field reference is read at its stated
  position.
- **The last positional readings retire**, including the font (`0x0008`) — the one type read with no table at all — and
  the object position, format wrappers, cross-tab measure binding and chart data field. Two were rules, not addresses: a
  wrapper's slots are the references the record states in slot order rather than whatever parsed as an `@`-name, and a
  chart's data field is the summarized field at its stated position rather than the first plausible string.
- **A parameter's detail record (`0x007a`)** was read by hunting for landmarks — a `0xFF` run, an `ff ff` pair, the
  literal `crobj` — and every landmark turns out to be a field. Two were not where they read.
- **The report root record (`0x0064`)** carries a length-prefixed document name, so every field after it moves by that
  name's length. `EnableSaveDataWithReport` was read at the empty form's offset, landing inside the name, so the flag
  was the low bit of a letter of the subreport's file name. Nine subreport records were affected, in both directions.
- **A running total (`0x0080`) contains its summary rather than preceding it**, so it is told from a summary definition
  by containment rather than adjacency; each of its two conditions is a kind followed by what that kind names.
- **The saved-data catalog, and one report's stored rows come back.** A header-shaped run of field data was framed as a
  nested record, cutting real field bytes; on one report it cut the very first field, so the record width read as `0`,
  the batch cipher IV keyed on it, and the report claimed no saved data while its own descriptor said otherwise.
- **A parameter's saved current value (`0x0031`)** was read at computed offsets, one of them wrong, so every record
  failed its own entry decode and fell through to the prompting-form document. The saved values now read in a record
  vocabulary of their own (`Dialect::ReportParameters`), and a stored date default is written as `Date(YYYY,MM,DD)`.
- **Text objects read as sequences**: the opener names its paragraph count and field-heading flag, a literal run is text
  plus character spacing (so an empty run reads as the stored value it is), and a field run states its embedded
  reference as one composite, closing three latent defects in the scan it replaces.
- Likewise the field object, blob-field wrapper, picture object, chart object, formula, designer connection, guideline,
  the seven band markers, the saved-data descriptor, the field-heading link, the subreport re-import descriptor, the
  cross-tab custom-member collection, specified-order group values, font colour and font conditional formats, the group
  and sort records, the field-pool census, persisted formula variables, and a new `CrossTabGridCell` table.
- **The database and print-options records** read declaratively too, so a table's alias, qualified name and SQL command
  text, a connection's driver DLL and a logon property's value are read at their stated positions rather than found by
  scanning. The page DEVMODE is read as the `dmFields` mask says.
- **Two records are renamed for what they are**: `0x0081` is a SQL Expression field (it was named after the engine
  translation unit hosting its reader) and `0x0160` is the document's option bag (it defines no field).
- **A formula field's stored value type and width are no longer discarded** when its body references a dropped column.
  The decoder was reproducing the engine's load-time recompute, which is gated on live datasource binding and so not
  reproducible from the file; that model stays in `rpt_formula::string_max_bytes`.
- **A chart with no axes reports the gridline modes its record stores** — discarding them for Pie, Doughnut, Gauge,
  Gantt, Funnel and Histogram was a derivation in the reader. Ten of forty-seven corpus chart records move.
  `ChartElementFont::size_pt` is gone: a chart's per-element point size is not a `Contents` fact.

**Elsewhere**

- **The reader's internal layers are named for what they do.** The invented vocabulary — "project/raise", "substrate",
  "DOM", "tiling" — is gone from module paths, identifiers and prose, while names that come from the format or the SDK
  are unchanged. Decode baselines, Page IR baselines and PDF output are byte-identical.
- **A chart category label that does not fit its slot is rotated 45° instead of thinned away**, the axis thinning only
  once the rotated labels collide. Our old rule thinned horizontally against a fixed width, which both over-thinned and
  never rotated. Moves seven fixtures' Page IR.
- **Chart subtitles and footnotes draw on every chart family** — the caption bands were reserved only on the shared 2-D
  dispatch, so eight families silently dropped a stored subtitle and footnote.
- **The PDF backend shapes every text run itself**, with `harfrust`, rather than calling krilla's single-face, BIDI-less
  `draw_text` wrapper that returns no advance and made a justified line shape each word twice. It retires the
  `RUSTSEC-2026-0206` waiver rather than renewing it, and **krilla moves 0.6.0 → 0.8.2** with no source changes. Output
  is byte-identical across all eight PDF baselines and all 64 Page IR baselines.
- **Release archives ship stripped binaries with debug symbols as a separate asset**; dropping the sidecar beside the
  binary restores names and source lines. `rpt` 18.45 MB → 3.12 MB, `rpt-render` 72.77 MB → 16.06 MB.
- Test-harness hardening: `json_dump_is_the_model_and_only_the_model` checks a named `DERIVED_KEYS` list, the `rpt-cli`
  fixture-gated suites no longer skip on an empty corpus, `rpt dump --glob` accepts a directory and sweeps recursively,
  and the PDF-artifact suite documents that `RPT_REQUIRE_QPDF` is set nowhere, so its "opens in a real reader" layer
  runs only where `qpdf` happens to be present.
- `rpt-render --db` against a `sqlite://` URL hands the record-selection formula to the query builder as the PostgreSQL
  path does; the SQL is unchanged, but the run now warns that selection is applied after the fetch.
- `rpt_text::FaceRun` and `rpt_model::first_brace_ref` are public — the first was the unnameable return type of a public
  method, the second was copied in two crates.
- The test suite runs on Windows in CI as well as Linux, and text files check out with LF line endings everywhere — the
  baselines are compared as exact strings, so a CRLF checkout failed all of them at once.

### Fixed

**Decode.** Most were invisible on the committed corpus — a fixed offset and a stated field sequence coincide until a
string, a repeat or a schema version moves — so unless an entry says otherwise, no decoded value changes today and what
is fixed is a reading that would have been wrong on a report we do not have.

- **All ten format-record wrappers and five of the format records themselves read from field tables.** Transcribing the
  five corrected a string field's swapped reading order and line-spacing type, a border's line width (a whole word, not
  the byte its low half looks like) and its corner ellipse.
- **Fourteen more types read from field tables.** `0x0071 NamedValue` has **one shape, not two** — a definition with no
  name stores an *empty* name — and two were filed under a wrong premise, `0x0078` being a special-variable field
  definition rather than an object-format record.
- **The six query-engine session types read as their reader writes them**, and every field gained at a version is read
  only from that version on, so a report written by an older engine no longer decodes at shifted offsets.
- **The record tree reads a record whose header states no schema word** — a never-revised type is written four bytes
  narrower — recovering 393 query-engine index records that used to read as opaque blobs.
- **A `QESession` stream no longer frames its own field data as nested records**: requiring the observed schema prefix
  removes all 1,029 header-shaped runs that had become child records. Two compensations that had grown around the damage
  go with them, so an `nvarchar(max)` column reports the stored `-1`, not a fabricated 131070.
- **Empty records are read as records** — the framing layer omits the length field entirely for a record with no
  content, and the engine uses that form as an end marker and inside area pairs, sections, page setup and object names.
- **The two bytes after a record type are the `schema` word, big-endian**, not a little-endian "subtype".
- **Emitting a record at a version reproduces that version's layout.** The write path resolved a schema-selected field's
  *width* against the version but its *presence* from the row, so an older-version record still carried every field a
  later version added. All 37 version-gated declarations are now exercised at each version they name.
- **Three decoded values were wrong** in the last two format records to be tabled: `visible_records_per_page` was read
  as the single byte its low half occupies (a limit of 300 read as 44); an area's `suppress` came from the underlay flag
  eleven values later, a word only a *section* sets; and `0x00fc`'s values were addressed past a "fixed 15-byte header"
  that does not exist, so a non-empty tool-tip moved the hyperlink target, rotation, CSS class and link type with it.
- **`ObjectFormat::text_rotation` no longer reads a fixed offset inside a variable-length region** — the angle follows
  the leaf's `HyperlinkText`, so every object with a real hyperlink decoded its rotation out of the URL's own
  characters; three corpus objects reported the ASCII of `"tt"` and `"nt"`.
- **Four more are now read as stored**: a string summary reported its length in characters and saturated at 255; a blob
  column reported `65535` instead of `-1`; a save-time metadata value failing a clean-text check was reported empty
  though it is framed by its own length; and the string after a date-time order enum was read at a fixed offset.
- **The summary record's percentage fields are read after its second operand**, not a fixed distance from its first.
- **The chart / cross-tab binding collector no longer finds an object's name by scanning its record** — a scan picks a
  different string on 274 of 6058 records.
- **Hierarchical grouping is decoded again**: its block sits inside the group record's fixed trailer, not past it.
- **A line drawn upward across a section boundary keeps its stored second-corner Y and end section**; three corpus lines
  previously decoded as ending at `0`, in their own section.
- Also read as stored: a subreport link's second field handle; one labeled value per riser in a chart's data-value
  record, and the group-axis title's slant; the report's canned formatting style and `ReportKind`; and a pie or doughnut
  chart's `SliceDetachment`.
- **The saved-data batch directory is walked by byte position, not by record width** — `saved_record_count` returned
  1000 for a report storing 2645, because a packed record is compacted per batch to that batch's own per-column maxima,
  so `item_size` is not constant. A truncated decode is now visible too, the count coming from the directory so
  `rows.len() < record_count` is the signal.
- **`rpt dump` names and reads every record in the vocabulary of the stream it came from** — it resolved every name from
  the report definition's registry, so a `QESession` record was labelled after whatever `Contents` record shares its
  number. It also stops blaming the field table for a record it never read.
- **`rpt saved` and `rpt dump --saved` no longer blame a batch class the reader never reached** for a report whose rows
  live in a subreport.
- **Reports decode on Windows again.** Streams are addressed by OLE path, read with the host's path rules — and
  `Path::components` spells the container root `/` on Unix but `\` on Windows. The root survived there, so every
  top-level stream looked nested and classified as `Other`; with `Contents` unrecognised nothing decoded at all, and
  `rpt inspect` reported zero records for every file. Subreport grouping misread the same path the same way. The root is
  now dropped structurally and both separators split, so an OLE path classifies identically everywhere. Reported by
  @jporeilly ([#1](https://github.com/MrSrsen/rpt-rs/pull/1)).

**Tests.**

- **Sixteen tests and two CI gates could pass without executing their checks**, each repair proved by breaking the thing
  it guards. Twelve resolved a committed fixture with `if absent { skip }`, so a renamed directory turned the file into
  a green no-op — including `patch_gate`, the clearance gate on the only write path that can corrupt a `.rpt`, which had
  never run at all. Five corpus sweeps carried no floor or one far below reach; the font-source tests passed on a PDF
  embedding no font; and `public_rpts()` returned an empty vector when rooted wrongly, which every caller read as
  "nothing to check".
- **The L1 decode net covers every committed report, not just the fixture tree.** The synthetic Meridian corpus had
  page-IR baselines only, though it is the sole carrier of hierarchical grouping, formula variables, subreport links and
  Basic-syntax formulas. `json_baseline` also walks each baseline back to its report, so an orphan fails. The
  field-table parity sweep now decodes a subreport's own stream before walking it.
- **Two decode assertions could pass on a coincidence** — the chart data-labels test asserted only the negative case,
  and no corpus report declared a `Time` column. Both now have purpose-built fixtures.

**Render.**

- **A report rendered from saved data is no longer re-filtered by its record-selection formula.** A saved batch is the
  rowset as it stood *after* selection ran, so re-evaluating could only lose rows the engine displays; what does apply
  is the separate saved-data selection formula. `RowSource` gains `already_selected()`. Two corpus reports rendered
  mostly-empty pages and now reproduce the engine's own export to within 1% of its placed text.
- **An offline render of a saved `DateTime` keeps its time of day.** The batch packs two `u32` halves into one 8-byte
  scalar and the pipeline read the whole serial as a bare Julian day, so a `9:15` row rendered as `12:00`. The gap hid
  behind 8515 of 8542 saved cells sitting at midnight.
- **A half-open range parameter no longer drops every row** — a `Null` bound is now an **open** end, where the evaluator
  had no representation for one and `rpt-render` built a range only when both ends coerced.
- **Special fields render their value instead of their own name.** A text object carries its references as runs, each
  holding the engine's placeholder rendering, and the renderer substituted on the flattened string — so a special
  printed `Printed Date: PrintDate`. Text objects now resolve run by run, and an embedded `Page N of M` is patched with
  the final count without re-evaluating the object's formulas, so their `WhilePrintingRecords` side effects fire once.
- **A parameter embedded in a text object renders its value** — `{?Param}` resolved to null on the reasoning that a
  higher layer would supply it, and none did.
- **`GroupName` resolves the group level it names**, both as a placed field and inside a formula body, where it had
  reached the evaluator as an argument-less print-state special that no context answered, failing the whole formula.
- **The 103 `cr*` format constants evaluate to the number their property stores** (`crBold` = `1`, `crLandscape` = `2`,
  `crOnePointLine` = `20` twips), where each previously carried a per-name code of its own that matched nothing. The
  point of these constants is that `if … then crBold else crRegular` on a conditional format agrees with picking the
  option statically, which requires the value to be the property's own ordinal — so every comparison or `ToText` against
  one was wrong before. Three names whose enumeration is still unidentified — `crNoLeadingDay`, `crShortLeadingDay`,
  `crLongLeadingDay` — now report as unimplemented rather than evaluate to a number nothing backs.
- **A percentage summary renders as a percentage, not its raw aggregate** — `PercentOf<Op>` fell back to a field-only
  match, so the group's subtotal printed where the engine prints its share (`106,235.83` instead of `3.68%`).
- **Date/time special fields resolve from the render's as-of instant**, having been deferred to an orchestrator that
  never ran the step, so a `CurrentDate` formula worked while the `PrintDate` beside it did not. A render with no as-of
  renders them blank rather than reading the host clock. `ModificationDate`, `ModificationTime` and `FileCreationDate`
  render the file's timestamps, split **in UTC** so a rendered date is never an artifact of the rendering machine.
- **An explicit date/time field renders what it stores rather than what the locale says.** The renderer ignored all five
  stored separators and ordered the tokens by the render locale, so a field authored `dd-MMM-yyyy` rendered
  `14/Nov/2023` on an en-US host and `14.Nov.2023` on a German one, and a month-day-year field rendered `26/05/2001`
  under a day-month-year locale — not a rearrangement but a different day. The stored time format is honoured too
  (clock base, element styles, designator text and placement, both separators), and a `DateOnly` field stops appending
  `12:00:00AM` to every date on the page. Two rules the bytes do not carry are reproduced: the "no leading zero" hour
  still occupies two cells, and a dropped element takes the separator after it. A system-default field still takes
  order and separator from the locale.
- **A numeric field stored as `DefaultAlign` renders right-aligned** — the engine resolves that default at paint time
  from the value type, and applies it to field objects only, where we rendered every such field flush left.
- **A positive number reserves the character cells its negative form would occupy** — a leading cell for the minus
  (` 1,012`), a trailing cell for the closing bracket (`$53.90 `). A cell a currency symbol already fills is not padded.
  `NumberFormat::reserve_sign` is off by default, so `ToText` is unaffected. Verified token-for-token over ~15,000
  numeric tokens.
- **A tab in a text object advances the pen instead of drawing a `.notdef` box.** Tabs resolve before the Page IR — a
  tabbed line is split and each segment placed at the stop its tab advanced to, every **0.25 inch (360 twips)** from the
  line's left edge — so no control character reaches a shaper.
- **An "Underlay Following Sections" band ends at its companion section**, so a footer no longer prints on top of the
  watermark it should sit below. The engine never closed the span, so on a report with a 7781-twip watermark the group
  footer, its aging table and the report footer all landed 7781 twips too high.
- **A date group's name and chart category label print at the group's own granularity** — both were derived from the
  bucketed key alone, which cannot say which period produced it, so a monthly group printed `1/1/2024` where the engine
  prints `1/2024`.
- **Chart and cross-tab labels are no longer elided with an ellipsis** — the engine draws over-long text in full and
  lets the box clip it.
- **A chart's value axis picks the engine's ticks.** The auto scale divided the data max by 8 on a 1/2/5×10ⁿ ladder
  where the engine divides by **9** on a ladder including **4**, and labels were abbreviated (`1000.0k`) where the
  engine never abbreviates.
- **Area, line, stock and radar charts reserve the legend band the engine reserves** — these families draw every mark in
  one colour, so the engine legends the *series*, not the categories. Drawing none gave the plot the whole width, which
  is not cosmetic: the category axis thins by fit, so the wider plot kept 25 labels where the engine keeps 13.
- **Chart text renders in its authored weight and slant**, at the size the chart's height implies rather than a
  hardcoded table. Element faces were also read one element out of step, and the byte read as the title's point size is
  not one — it reads 17 for a 14 pt, a 20 pt and a 24 pt title alike, which made four fixtures draw their title at
  27.75–33.75 pt against the engine's 13.5 pt.
- **3-D charts draw the engine's room** — three **slabs** rather than three flat planes in invented greys over a floor
  grid, filled by face orientation, hairline-outlined, with the value gridlines carried around each wall's near end-cap.
  The two wall greys were swapped, and the room now takes its size from the chart box rather than the label frame, so it
  no longer shrinks and drifts as the label bands grow.
- **3-D chart cameras are measured against the engine, not reconstructed from preset names**, for eleven of the sixteen
  `ChartViewAngle` presets — including `Standard`, now known to be a flat wide box seen from 19.3°/47.4° rather than a
  cube from 36.1°/42.1°. The remaining five keep their name-based reconstruction. The engine **stretches** the room per
  axis rather than scaling it uniformly.
- **The 3-D room belongs to `Riser3D` and `Surface3D` alone.** A depth-effect Area chart was routed through it and now
  draws the flat frame plus one cast face per crest segment; and a 3-D chart draws **no legend**, where we drew a swatch
  per category — its series are its depth axis, already labelled on the floor.
- **A page-total run keeps its character spacing in its reported advance**, so rigid spacing re-anchors a centre- or
  right-aligned footer to the width it is actually drawn at.
- `ObjectFormat::vertical_alignment` is now known to be right for all three values — bottom stores as `8`, which no
  report in any corpus used, so only top and centre had ever been exercised.
- Smaller repairs: the `rpt-render` binary's `db-postgres` / `db-sqlite` features compile cleanly again;
  `special_value_type` and `SpecialFieldType::value_type` no longer disagree; and the JSON dump carries both stored
  numeric-format slots and the stored heading alignment, which the decoder had previously destroyed.

**Documentation.**

- **The format documentation describes a record's content as a sequence rather than a layout** — a straight-line run of
  typed reads whose positions move with a string's length, a repeat's count and the schema version. The old fixed-layout
  description is what invited positional decoders, and would have invited the same mistake in the writer.
- **The documentation is re-read against the tree**, and record coverage re-measured on the committed corpus alone:
  **154 reports, 55,829 outermost records, 106,950 across the record trees, 2 still `Unknown`**. Nine claims were wrong
  rather than stale. Maps, alone among the undecoded families, do occur — one corpus report carries all six map records,
  so decoding them wants the work rather than a report to author.
- **The support matrix says what its three partial streams exclude.** The substantive find: `PromptManager` inflates to
  a *sequence* of `<CRMetaObjects>` documents and only the first is decrypted, so the prompt-group documents behind
  cascading prompts, edit masks, the discrete-vs-range flags and the pick-list mirror are entirely unread.
- **The README no longer claims clean-room development, and it names the mark's owner.** *Clean-room* is a term of art
  meaning the implementer never examines the original, which is not how this project was built; the accurate
  description is an independent implementation, reverse-engineered from the file format. The licence section states the
  two licences that are not MPL-2.0, which closes the `NOTICE` question: MPL-2.0 has no equivalent of Apache-2.0 §4(d).
- **The two test-layer descriptions in `docs/` are reconciled** — `building.md` owns the L1–L4b layer map and
  `09-testing-parity.md` owns what each corpus is, and the testing page gained the L4b section it was missing.
- **The `rpt_reader::provenance` module is gone** — six empty public modules holding a hand-maintained reverse index
  from a model type to the record it decodes from, a relation already stated forward in the block catalog and at each
  decoder. Going through it claim by claim showed the module's own warning — that a copy this far from the parsing code
  has nothing to keep it honest — applied to its record numbers too.

## [0.3.0]

This release replaces the RptToXml-compatible XML export with a plain, exhaustive **JSON** dump of the decoded model as
the decode regression surface, finishes the stored-vs-derived separation, adds a lossless KDL projection and a `.rpt`
anonymizer, pushes the render pipeline through typography, subreport flow, charts, and live-database work, and closes a
workspace-wide audit of **error surfacing** so that every failure says what went wrong, where, and what to do next —
including the ones the pipeline used to swallow silently.

### Breaking changes

The upgrade checklist; each item is expanded in the sections below.

- `From<std::io::Error> for rpt_reader::Error` removed — a bare `?` on an I/O call no longer compiles. Use
  `rpt_reader::IoError::at(op, path, source)`, or `::new` where there genuinely is no path.
- `rpt_reader::Error::Record`, `rpt_reader::RecordError`, and `ProjectErrorKind::UnknownRecord` deleted (zero
  construction sites) — use `Rpt::decode_coverage()` to detect an incomplete decode.
- `rpt_query::build_query{,_in,_full,_for_report}` return `Result<_, QueryError>`, not `Option`.
- `rpt_json::export_json` returns `DecodeCoverage`, not `()`; its `Error::Io` split into `Write { path }` /
  `Serialize { input }`.
- `ReportDocument::export_{pdf,html}_to_disk` return `rpt_reader::Result<()>`, not `std::io::Result<()>`.
- `rpt_render::render_with` is infallible (`-> PagedDocument`); `RenderError` moved to the CLI; the `db-postgres` /
  `db-sqlite` features are gone.
- `rpt_pages::DiagnosticKind` is now `#[non_exhaustive]` — an exhaustive `match` needs a wildcard arm.
- `rpt_data::DbError::{Connect,Query}` are struct variants carrying the data source and statement; `no_table()` →
  `no_query(reason)`.
- `metafile::Error` variants carry `offset` / `record`.
- `rpt_model::TableLink.join_type` is replaced by `join_kind` + `operator`.
- `rpt_model::Report` no longer carries `records` / `record_inventory`, and `record_count`, `distinct_record_types`,
  `count_of` are gone — use `Rpt::record_dom()` / `Rpt::inventory()`.
- `Sqlite/PostgresSource::fetch` take a unified argument list.
- `rpt patch` refuses a record type that is not cleared for editing — pass `--force` for the previous behaviour.
- `rpt xml-dump`, the `rpt-xml` crate, the XML baselines, and the export `--locale` flag are gone — use `rpt json-dump`.
- `docs/` was renumbered; existing links to the old numbering do not resolve.

### Added

- **`rpt json-dump <file.rpt> [out.json]` — the decode regression surface.** Emits an exhaustive, deterministic
  `{ "model": <Report> }` document: the full serde serialization of the decoded model, every field including defaults,
  the whole subreport tree, sorted-key maps, two-space indent, byte-identical across runs. It carries **stored facts
  only** — nothing inferred, recomputed, or reconstructed — so a diff against a committed baseline always means the
  decoder's reading of the file changed. Backed by the new reader-side library crate **`rpt-json`** (`export_json`),
  callable in-process instead of by spawning the CLI. The committed baselines (94 fixtures) live under
  `tests/fixtures/baselines/json/` and are checked by `cargo test -p rpt-cli --test json_baseline`
  (`RPT_BLESS=1` re-blesses).
- **`rpt-kdl` crate + `rpt kdl <file.rpt> [out.kdl]`.** Exports the semantic model to a [KDL](https://kdl.dev) document
  (`rpt_kdl::to_document` / `to_kdl_string`): construct kinds as node names, the identifying name as the first argument,
  scalars as `key=value`, nested structs/`Vec`s as child nodes — *sparse* (non-default values only), geometry in twips,
  colours as `#rrggbb`, enums as kebab-case tokens, formula/text bodies as multi-line strings. Binary payloads never
  enter the KDL: pictures emit a `source="…"` reference with bytes returned out-of-band by `rpt_kdl::assets`. The
  mapping is **lossless and compiler-enforced** — every struct is destructured without `..` and every enum matched
  without a `_` wildcard, so a new `rpt-model` field fails to compile until the exporter handles it. It emits the whole
  stored surface: the record-level sort, report options, saved printer, table columns, field-pool metadata, the full
  parameter surface (edit mask, report name, current/initial values with range bounds, display/sort/discrete-or-range
  kinds, prompt group, dynamic LOV binding), persisted formula variables, running-total condition formulas, hierarchical
  group values, the full chart definition, and cross-tab axis options. Depends only on `rpt-model` + `kdl`. The CLI
  subcommand writes sidecar picture files, or prints to stdout.
- **`rpt anonymize <file.rpt> [out.rpt]` — strip authoring metadata.** Removes the identity a report carries: the OLE
  `SummaryInformation` author and last-saver (blanked), and a re-imported subreport's stored source path (reduced to its
  bare file name, *not* blanked, because it is the only evidence the subreport was imported at all — emptying it would
  silently turn `SubreportObject.IsImported` false). The database connection's stored path is deliberately left alone:
  it is a live datasource locator, not authoring metadata. **Every edit is same-length** — a value's length prefix is
  untouched and only its characters are overwritten, then NUL-padded to the original width — so no record length, no
  enclosing record length, no property offset and no section size moves, and the decoded model is unchanged apart from
  those fields. A report with nothing to remove round-trips byte-identically. `--dry-run` reports what would go without
  writing; `--json` machine-reads the removals. Exposed as `Rpt::anonymize` / `rpt_reader::io::AnonymizeReport`; this is
  what keeps the committed fixture corpus publishable.
- **`rpt dump --stream DataSourceManager`.** The `DataSourceManager` saved-data catalog stream's logical payload
  (decrypted + inflated) is now exposed through the reader's `streams()` / `logical_bytes()` surface, so `rpt dump`
  can read its QE-dialect record tree and dump its records (structure `0x2d`, field header `0x41`, batch entry `0x6d`).
  `rpt streams` reports the stream's decoded logical byte count instead of labelling it opaque.
- **`rpt tree` shows decoded field-value summaries.** For recognized records — the field-format leaves
  (numeric/string/date/time/date-time/boolean/common), group-area options (`0x88`), and summary/running-total
  definitions (`0x7e`) — each node shows a concise decoded summary (`DecimalPlaces=2 Negative=Bracketed …`) in place of
  the raw byte preview; `--json` carries the same as a `decoded` object. Backed by the new public
  `rpt_reader::annotate::summarize` API.
- **SQL tracking comments on every generated query.** A query the render path issues is prefixed with a sanitized
  single-line block comment identifying the report and scope (`/* rpt-rs report="orders.rpt" scope=main */ SELECT …`),
  so a statement surfacing in the database's own logs is traceable back to the report that issued it. Built by
  `rpt_query::SqlQuery::with_comment`, which flattens control characters and breaks up `*/` / `/*` so the comment can
  neither terminate early nor nest; an empty comment is a no-op. Both drivers accept it.
- **`metafile` crate.** A Windows-metafile vector parser (EMF today; WMF/EMF+ planned) that resolves a metafile's own
  coordinate machinery into backend-agnostic primitives through a `MetafileSink` visitor, so the pipeline can draw
  OLE-embedded EMF pictures as vectors. Zero dependencies, no GDI, WASM-safe, and free of `.rpt` concepts — publishable
  on its own (MIT OR Apache-2.0).
- **`meridian-seed` + the synthetic render-test corpus.** A deterministic generator for the "Meridian Global Logistics"
  render-test database, emitting portable Postgres/SQLite SQL from one fixed-seed PRNG (~39 tables, ~40k rows in the
  committed `small` tier; `--tier`, `--dialect`, `--out`, `--ddl-only`). Per-field distributions are realistic and date
  envelopes internally consistent; the committed seed lives at `tests/meridian/sql/meridian.sql`, the human-readable
  schema at `apps/meridian-seed/schema.sql`. Alongside it: a corpus of own reports over that database with HTML render
  baselines, and a typography fixture set with Page-IR/HTML baselines.
- **Formula constants: `cr*` enum constants and relative date ranges are now evaluable.** The 117 `cr*` enum constants
  (alignment / font style / line style / negative & currency format / calendar, e.g. `crBold`, `crLeftAligned`,
  `crDashedLine`) and the 27 relative-date-range constants (`YearToDate`, `MonthToDate`, `Last7Days`, `LastFullMonth`,
  `AllDatesToToday`, `Aged0To30Days`, the calendar quarters/halves, …) now evaluate instead of failing with an
  unsupported-builtin error. The `cr*` constants resolve to a bare number; the date ranges resolve to an inclusive date
  `To` range computed against the context's `CurrentDate`/`Today`. Conditional-format formulas returning a `cr*`
  constant and record-selection formulas using a date range no longer surface as unsupported-formula diagnostics.
- **Date/time formula specials in the render pipeline.** Formulas reading `CurrentDate`, `Today`, `CurrentDateTime`, or
  `CurrentTime` now resolve during a render instead of evaluating to null — so a date-relative formula (e.g. an aging
  bucket that buckets rows by days from `CurrentDate`) produces its real value. A single "as-of" instant is captured
  once per render (`RenderOptions::as_of`, defaulting to the system clock at render start via `default_as_of`, or set
  explicitly for a reproducible render), threaded through both the record pipeline and the layout pass so every
  evaluation context — including crosstab pivots and subreports — shares the same fixed value. `Today` is recognized as
  the engine's alias for `CurrentDate`. The render core stays clock-free and WASM-safe: the clock is read only at the
  entry point (the `rpt-render` facade / CLI), and a WASM build falls back to the Unix epoch unless the host supplies an
  `as_of`.
- **Paragraph typography in text objects.** A text object's per-paragraph formatting is now honored end to end instead
  of being flattened to one object-level font at single spacing:
    - **Per-paragraph font.** Each paragraph renders in its own run font — a multi-paragraph object mixing point sizes
      (e.g. a 12pt paragraph followed by a 20pt one) now draws each paragraph at its own size and wraps it with the
      right metrics, rather than rendering every paragraph at the object's base size.
    - **Line spacing.** The paragraph line spacing (`LineSpacing` / `LineSpacingType`) is now decoded (the `0x00c0`
      paragraph leaf: type byte 17, a 16.16 multiplier — or exact twip pitch — at bytes 18-21) into a new
      `IndentAndSpacingFormat.line_spacing` model field, and applied by the layout engine: single / one-and-a-half /
      double spacing grows the inter-line gap, and exact spacing pins the line pitch to its twip value. Can-grow band
      heights account for the taller lines.
    - **Justified alignment.** Justified text now stretches to both edges by spreading the inter-word slack across every
      wrapped line except a paragraph's last (which stays flush-left), instead of rendering identically to left-aligned.
      The layout marks which lines justify; all four backends flush both edges (SVG `textLength`, HTML
      `text-align-last:justify`, PDF `Tw`/word-by-word, raster per-gap spread).
    - **Text rotation.** The stored text rotation angle (`TextRotationAngle`, `0x00fc` bytes 20-21, degrees) is now
      decoded and applied: a 90°/270° text object lays its wrapped lines out as vertical columns (reading up for 90°,
      down for 270°) and all four backends (SVG, HTML, PDF, raster) rotate the runs, matching the native engine's
      vertical text.
- **Chart text: subtitles, footnotes, and per-element fonts.** A chart's decoded subtitle (drawn under the title) and
  footnote (drawn at the chart bottom) are now rendered for every chart family; previously both were decoded but never
  drawn. Chart text also renders in the font stored per text element instead of one hardcoded Arial: the layout engine
  resolves each title/subtitle/footnote against the decoded `ChartDefinition.element_fonts` run, preferring the stored
  face name (when it is a real override, not the default Arial) and stored point size, and otherwise falls back to the
  engine's per-element default table (title Arial 14 bold, subtitle Arial 10, footnote Arial 8 bold-italic,
  axis/data/series titles Arial 8 bold, legend/data labels Arial 7). Charts that store defaults render byte-identically.
- **3-D ("Faked3DRegular") pie rendering.** A pie chart whose stored subtype requests the depth effect now draws a
  tilted-ellipse face with an extruded, shaded crust along its front rim instead of the flat 2-D disc, matching the
  engine's faked-3D pie. Flat pies are unchanged.
- **Biweekly chart category axis.** A chart whose category axis is a date field grouped biweekly now buckets on
  fortnight-aligned two-week boundaries instead of falling back to weekly grouping.
- **Aspect-fit picture rendering.** Picture objects and DB blob-field images now letterbox — the raster is scaled
  uniformly to the largest size that fits its object box, preserving the source pixel aspect ratio, and centered, with
  the surrounding space left empty — matching the native engine instead of non-uniformly stretching to fill the box. The
  Page IR's `ImageOp` carries a new `fit: ImageFit { Fill, Contain }` (defaulting to `Fill`, so existing Page-IR dumps
  deserialize unchanged); layout emits `Contain` for pictures and blob fields, and all four backends honor it (HTML
  `background-size:contain`, SVG `preserveAspectRatio="xMidYMid meet"`, PDF/raster compute the centered fitted sub-rect
  from the decoded pixel dimensions).
- **Raster backend JPEG/GIF pictures.** The `rpt-render-raster` backend now decodes JPEG (via the pure-Rust
  `zune-jpeg`) and GIF first frames (via the pure-Rust `gif` crate) in addition to PNG and BMP, so those picture formats
  composite as pixels instead of falling back to the placeholder outline. Both decoders are WASM-safe (no C bindings),
  matching the render core's portability rule.
- **Cross-page inline-subreport flow.** An inline subreport taller than a whole page is now split across parent pages at
  row boundaries instead of overflowing off the page bottom. The child is still formatted exactly once (its
  `Shared`/`Global` writes fire once); the split is pure geometry over the cached box-local ops, so a subreport with an
  internal forced page break places each of its pages on its own parent page. A subreport that fits on a page keeps the
  byte-identical atomic placement (moving whole to the next page when the space left is too small).
- **A large set of previously-unmapped stored fields is now decoded**, and is carried by the JSON and KDL exports:
    - *Fields & formats:* value types for formula / special / group-name / running-total / SQL-expression references;
      the numeric & currency leaf (negative format, currency-symbol format, symbol text and placement, thousands
      separator, suppress-if-zero); the string leaf (word-wrap, maximum line count, text interpretation); the date /
      time / date-time display members (date order, hour/minute/second, date-time order) and separator; object vertical
      alignment.
    - *Objects:* the object hyperlink **type**, read from its stored `CrHyperlinkTypeEnum` byte (see *Changed*); a box's
      rounded-corner ellipse (width/height, the stored basis for rounded boxes and ovals); picture geometry (original
      size, X/Y scaling, cropping); box/line end-section spanning; section codes; paragraph indentation and
      field-heading reading order; subreport links, the imported flag and the re-import flag; the group-name special
      field in SDK form (`GroupName ({field})`).
    - *Data definition:* **custom-function declarations** (`DataDefinition.custom_functions`), split out of the formula
      pool by body shape — a body opening with the `Function` keyword can only be a custom function, never a report
      formula; SQL-Expression field definitions; parameter range kind and range current value; command-table /
      stored-procedure parameters; formula null-treatment; the group long-period date-grouping condition (weekly …
      annually) and the six boolean group conditions; the summary operation parameter, percentage-summary flag, and a
      two-field summary's secondary field; Top N / Bottom N `DiscardOthers`.
    - *Report & database:* report options (convert-database-null-to-default and its sibling, verify-on-every-print);
      database table qualified names; a table link's join kind **and** comparison operator as independent values (see
      *Changed*); cross-tab grid style flags, grand-total colors, and per-level suppress-subtotal / suppress-label;
      chart data/category field references (in summary form), 3-D view angle, layout type, and the full legend-position
      set (`Left` / `Right` / `BottomCenter` / `Custom`, all four now corpus-confirmed); summary-info provenance
      (revision number, last-saved-by, saved printer name).
- **`rpt formulas <file.rpt>` — list and check every formula, without rendering.** Prints one line per formula marked
  `ok` / `warn` / `ERROR` with findings indented beneath, so you can see what was covered rather than only how many.
  Covers formula fields, record- and group-selection formulas, and conditional-format formulas wherever they hang — on a
  section, an object's format, its border, or a field/text object's font colour — through every subreport. Reads the
  `.rpt` alone; exits 1 on any error, so it works as a CI gate. `--source` quotes each formula's body under its line,
  and a formula field the report declares but left blank is listed as `empty` rather than silently omitted. `--json`
  always carries the source, kind, syntax, and size.
- **Incomplete decodes are reported instead of passing silently.** `rpt json-dump`, `rpt kdl`, `rpt-render`, and
  `rpt streams` now say when a report did not fully decode — previously an export missing content looked identical to a
  faithful one, because projection returns defaults for records it does not recognize rather than failing. New
  `Rpt::decode_coverage()` exposes the figures (unrecognized records and their types, bytes covered by no record,
  per-stream decode errors); `rpt streams` gains them in `--json`. `--strict` on the two export commands turns the
  warning into exit 1 for CI — the document is still written, so only the exit status changes.
- **The write path refuses uncleared record edits; `rpt patch --force` overrides.** Patching a record type that is not
  on the cleared-for-editing allow-list now fails with nothing written, because the writer's mechanical checks cannot
  catch a leaf carrying its own offset table or checksum being overwritten into a file that re-decodes cleanly but is
  semantically corrupt. The list holds one entry today (`0x0142 SubreportReimportInfo`), so most existing `rpt patch`
  invocations need `--force` (`EditPolicy::Forced`) — the right flag when writing an invalid record is the point, the
  wrong one for a report you intend to keep.
- **Diagnostics carry kind, severity, and location end to end.** `rpt_pages::Diagnostic` gained a
  `DiagnosticLocation { page, area, section, record_index, span }` (all optional, never fabricated) and `describe()`;
  `DiagnosticKind` gained the data-pipeline kinds and is now `#[non_exhaustive]`. Formula diagnostics carry the failing
  sub-expression's byte range and per-row failures the record index. `rpt_data::DatasetOptions` / `build_dataset_opts`
  is the general pipeline entry point and the only way to attach a `DiagnosticSink` alongside parameters, link filters,
  and an as-of instant. See [Rendering › Diagnostics](docs/12-rendering.md#diagnostics).
- **Errors say which file, and print their cause once.** `rpt_reader::IoError` carries the operation and path, so the
  commonest failure names itself (``cannot read `/nope/missing.rpt`: No such file or directory``);
  `rpt_reader::error_chain` renders a full `source()` chain and is shared by both binaries. `Error::NotAReport`
  diagnoses an input that is not a report at all — no OLE2 signature (with a sniff of what it actually is), a compound
  file with no `Contents` stream (listing what it does carry), a truncated container, or a `Contents` that will not
  decrypt — in place of `Invalid CFB file (wrong
  magic number)`.
- **Database failures name the source, the statement, and the next step.** `DbError::hint()` returns the failing SQL
  and, for a missing table or column, a pointer at `rpt sql <file>`; the error names the data source, which is what
  tells a multi-`RPT_DB_URL_<SERVER>` report which connection is wrong. `rpt_query::QueryError` replaces a reasonless
  `None`, and
  `SqlQuery::not_pushed` reports the selection conjuncts that could not be pushed into `WHERE` and so run locally after
  the fetch — the query reads more rows than the report shows, now warned at normal verbosity rather than only under
  `-v`.
- **Silent type coercions are reported.** A cell that will not parse as its column's declared type still falls back to
  text or null, but `RowSource::coercions()` now reports it once per column with the affected row count and an example —
  values kept as text sort, group, and summarize as text, which is otherwise invisible.
- **Test and CI infrastructure.** A deterministic fuzz target over `crystal-formula` (parse → compile → run → validate
  over generated input, both syntaxes, plus deep nesting), a CI guard for error-handling anti-patterns clippy cannot
  express, and `clippy::missing_errors_doc` / `missing_panics_doc` at workspace level — every public fallible API now
  documents the error variants a caller can match on.

### Changed

- **The export surface is JSON + KDL, and it is stored-facts-only.** The XML exporter and the derived analytics are gone
  (see *Removed*); the exhaustive `rpt-json` dump and the sparse `rpt-kdl` projection replace them.
- **Workspace restructured into `crates/` + `apps/` (24 members).** The three binaries moved out of the library tree
  into `apps/` — `apps/rpt-cli` (the `rpt` binary), `apps/rpt-render-cli` (`rpt-render`), and the dev-only
  `apps/meridian-seed`. No `[[bin]]` remains under `crates/` and no library logic remains under `apps/`: an app parses
  arguments, resolves inputs, and calls a library.
- **A table link's join kind and comparison operator are decoded independently** (breaking model change). The file
  stores the link's outer-ness and its comparison separately, so `TableLink.join_type: TableJoinType` is replaced by
  `join_kind: TableJoinKind` (inner / left / right / full outer) + `operator: TableLinkOperator` (`=`, `<>`, `<`, `<=`,
  `>`, `>=`); both are one-hot bit codes, not ordinals. The SDK's single `TableJoinType` cannot express "left outer
  **and** `>`", so it is now a lossy fold a consumer performs, not the decoded shape. `rpt-query` generates the ON
  condition from the operator and the join keyword from the kind, so a left-outer inequality join renders correctly; a
  full outer join has no counterpart and still degrades to an inner join.
- **A field renders its own stored currency symbol.** The currency symbol is stored *per field*, so two fields in one
  report can carry two different currencies: an explicit (non-system-default) field now uses its stored symbol text
  (`€`, `Kč`, `kr `, including any spacing baked into it) and its NoSymbol/Fixed/Floating choice, instead of always
  taking the locale's symbol. `NoSymbol` drops to a plain number; a system-default field still resolves the symbol from
  the render locale, which is what the engine does. Symbol *placement* still follows the locale pending the stored
  position byte.
- **An object's hyperlink type is read from the file, not guessed from the target.** The stored `CrHyperlinkTypeEnum`
  ordinal in the `0x00fc` ObjectFormat leaf is now located by walking past `ToolTipText` and `CssClass`, so a non-empty
  tooltip cannot shift it, and it decides whether a hyperlink exists (code `6`, `Undefined`, is the engine's "no
  hyperlink" sentinel). Previously the kind was inferred from a `mailto:` prefix and presence was decided by a non-empty
  target — which dropped every real hyperlink kind that legitimately carries an empty target (a field-value website, a
  report-part drill-down).
- **Subreport fetches are memoized per render.** The layout engine asks for a subreport's rows once per *instance*, and
  because a parent link is applied in memory after the fetch (never in the `WHERE`), every instance of one subreport
  issued the identical query — an O (instances × rows) blow-up on a report with many groups. The `rpt-render` CLI's live
  scope now memoizes each distinct fetch, so those repeats collapse to a single round-trip and a shared, refcounted
  rowset.
- **The raw record substrate left the format-neutral model.** `rpt_model::Report` no longer carries `records` (a full
  second copy of the record tree, every leaf byte cloned into the model on each open) or `record_inventory`, and the raw
  `Node`/`Unknown`/`Value`/`RecordTag`/`RecordTypeCount` types moved out of `rpt-model` into the `rpt` reader
  (`rpt_reader::raw`). The reader now projects them **on demand** from the substrate it already owns:
  `Rpt::record_dom()`,
  `Rpt::inventory()`, and `Rpt::subreport_record_doms()`. This makes the neutral model actually neutral — the whole
  render/data/db pipeline (which links `rpt-model`, not `rpt`) no longer drags in the `.rpt` record-type registry, and
  no report pays the per-open cost of duplicating its record tree. `rpt tree` output is unchanged.
  (`Report::record_count`/`distinct_record_types`/`count_of` are gone; derive them from `Rpt::inventory()`.)
- **`rpt-render` facade error model honesty.** The library's `render_with` is now infallible (`-> PagedDocument`
  instead of `Result<_, RenderError>`) — its built-in datasources (saved data, a materialized `RowSource`, a pre-built
  `Dataset`) cannot fail, and a live fetch fails before the render, in the caller. The stringly-typed `RenderError`
  (which the library never actually constructed) moved to the `rpt-render` CLI where every failure is raised; its `Db`
  and `Io` variants now carry the underlying `DbError`/`std::io::Error` as a real `source()` instead of flattening it
  into a message, so CLI errors print the full cause chain (e.g. `cannot write "…": No such file or directory (os error
  2)`). As a result the `rpt-render` library no longer links the native DB-driver crates at all (its `db-postgres`/
  `db-sqlite` features are gone), keeping the render core fully DB-free. The bring-your-own-layout
  `render_dataset_with` now takes the same `locale`/`scope`/`as_of` reproducibility knobs `RenderOptions` carries,
  instead of silently defaulting them.
- **Unified live-DB driver skeleton.** `rpt-db-postgres` and `rpt-db-sqlite` now share one error type and one
  constructor shape instead of duplicating them. The shared, driver-agnostic `DbError` lives in `rpt-data` (generic over
  each driver's own error type, so the crate stays WASM-safe with no DB-driver dependency), and both drivers alias it.
  Both `SqliteSource::fetch` and `PostgresSource::fetch` now take the same arguments (`url`/`conn_str`, `database`,
  `sql_exprs`, `selection`, `params`, `comment`): SQLite gains the `selection`/`params`
  push-down parameters and a `fetch_for_report` pruning constructor, and Postgres gains query-log `comment` support.
  Existing default (no-selection) query output is unchanged. The `rpt-render` CLI's SQLite path now builds the SQL once
  (via a new `SqliteConn`) and runs that exact query, so the logged SQL is by construction the SQL executed.
- **`rpt` CLI flags are scoped to their subcommand.** The argument parser is now table-driven: each subcommand declares
  exactly the flags it accepts, so a flag meant for another command (e.g. `rpt inspect f.rpt --probe u32`) is now a
  clean usage error (`unknown option '--probe' for 'inspect'`, exit 2) instead of being silently ignored. All valid
  invocations, flags, and output are unchanged.
- **`rpt` CLI usage errors are attributed correctly and exit with code 2.** A malformed flag or argument value (e.g.
  `rpt sql report.rpt --dialect bogus`, an invalid `dump` option value, a bad `patch` argument) is now reported as a
  plain usage message and exits with code `2`, instead of being laundered through an I/O error that printed an `io:`
  prefix and exited `1`. Reader and output errors are unchanged (still exit `1`).
- **A stream encrypted under a foreign key is diagnosed as a key problem.** Crystal never emits one, but an application
  embedding its own copy of the encryption library can round-trip reports under its own key, and such a payload simply will not decrypt with the
  built-in key. That case is now reported as an encryption-key failure rather than as a bogus zlib error. The
  `Contents` header's `useFixed` flag is confirmed inert — clearing it changes nothing about how the stream decodes, and
  the reader deliberately does not branch on it, matching the engine.
- **Linked subreports execute per instance.** An inline subreport is rendered against a per-instance dataset filtered by
  its parent-row link values: a parameter-routed link binds the parent field into the subreport's parameter (so its
  record-selection formula, and the live-DB `WHERE` push-down, keep only the linked subset) and a direct field link
  applies a structural equality filter. Previously every subreport instance rendered the same unfiltered dataset. This
  also propagates `Shared` variables accumulated inside a subreport back to the main report (a main-report grand total
  reading a subreport-accumulated `Shared` variable now resolves). New `rpt_data::build_dataset_with` /
  `rpt_data::FieldFilter`.
- **On-demand subreports render their placeholder caption instead of executing.** A subreport flagged `EnableOnDemand`
  is a click-to-expand link the native engine never runs in a static export; the layout now emits just its caption (the
  subreport name) and skips the query/dataset entirely, instead of executing it inline and drawing its clipped bands.
- **A subreport taller than its placeholder box grows the enclosing band instead of clipping.** An inline subreport is
  now formatted once ahead of pagination (its `Shared`/`Global` writes fire exactly once); the containing band grows to
  fit its full height and the existing checkpoint pagination flows the enlarged band across pages, so a subreport's
  detail rows are no longer truncated to the first page of its box.
- **Stored display formats are honored at render.** Numeric/date/time fields apply their stored thousands-separator,
  suppress-if-zero, per-field currency symbol and placement, and date/time separator instead of the locale's.
- **Formula null-treatment is honored during evaluation.** `crTreatNullAsDefaultValue` replaces a null field with its
  type's default and continues; the default `crTreatNullAsException` propagates the null.
- **Top N / Bottom N group sorts apply their limit and DiscardOthers** — groups are ranked by their summary and cut to
  N, the rest discarded or collapsed into a single "Others" group.
- **Date groups bucket by their stored granularity** (daily … annually, plus the order-sensitive boolean conditions)
  rather than by raw value. The boolean conditions break on a transition, on the row before it, or on the row after it,
  depending on the condition family; an unrecognized condition ordinal falls back to raw-value grouping and raises a
  diagnostic.
- **Chart/cross-tab temporal category labels are driven per period by an intentional style** rather than a
  monthly-vs-everything catch-all: each `ChartCategoryPeriod` maps to an explicit label style — annual reads `YYYY`,
  monthly `M/YYYY`, quarterly/semi-annually roll to their start month `M/YYYY`, and the day-granular periods
  (daily/weekly/semi-monthly) `M/d/YYYY`. Monthly (`M/YYYY`) and weekly (`M/d/YYYY`) are unchanged (confirmed); the
  previously catch-all annual/quarterly/semi-annual labels now read in their own style. A raw (un-grouped) date
  cross-tab column now shows the field's default date format instead of the compact style.
- **Two-field summaries compute** — WeightedAverage / Correlation / Covariance now resolve from the decoded secondary
  field instead of an empty value.
- **Layout controls are applied when placing objects** — center/bottom vertical alignment, paragraph indentation and
  right-to-left reading order, pictures at their authored scale (with cropping), and boxes/lines spanning to their end
  section.
- **Cross-tab grid style and grand totals render** — grid lines follow the stored show-grid option; grand totals are
  computed, filled with the stored colors, and suppressed when set.
- **Rounded box corners render** (border radius) in the HTML and PDF backends, driven by the box's newly decoded corner
  ellipse.
- **Field headings render** as text runs with their stored label, font, and colour, instead of being dropped.
- **Pictures render across all four backends, embedded once and referenced by content hash.** HTML, PDF, SVG, and raster
  all draw report pictures (BMP/PNG/JPEG/GIF), each distinct image embedded a single time. Binary columns are fetched as
  real bytes (a new `Value::Bytes`, no `::text` cast), so a blob-bound picture — including a per-row database blob —
  receives its true image bytes end-to-end.
- **Per-glyph Unicode font fallback** with a bundled open symbol/emoji font replaces `.notdef` boxes for glyphs the
  primary font lacks.
- **Report-footer grand totals render** — row-independent field kinds (summary / special / group-name)
  now resolve from the print state even in a row-less band.
- **`rpt-pages` Page IR** documents one additive-stable policy (new `#[serde(default)]` fields and
  `DrawOp` variants are non-breaking; renames/removals gated) and moves `serde_json` behind a default-on
  `json` feature so a consumer of the IR types can drop it.
- **The committed test corpus was reshaped.** The third-party `ajryan` reports were dropped (their licensing and content
  made them unpublishable as a regression corpus) in favour of reports authored for this project, the synthetic Meridian
  corpus, and a typography fixture set. Every committed fixture is `rpt anonymize`-clean, enforced by a test.
- **The `docs/` set was renumbered and rewritten** so it reads front to back in one order (format `01`–`04`, semantic
  model `05`, saved data `06`, block catalog `07`, support matrix `08`, endianness `09`, codebase map `10`, usage `11`,
  rendering `12`, render examples `13`). Existing links to the old numbering no longer resolve.
- **The render CLI reports the data pipeline's diagnostics.** Previously only layout/render diagnostics reached the
  warning channel. Data-pipeline failures now arrive too, errors print through a channel `-q` cannot suppress, and
  identical repeats collapse (`… [record 0] — and 606 more like it`) so a per-row failure cannot bury the summary.
- **Supplying a parameter the report does not declare is an error, not a warning.** The value cannot reach the render,
  so the output is for different criteria than asked for. It survives `-q` (which previously hid it from scripted runs)
  and suggests the nearest declared name: `parameter "order_amt_rang" is not declared by the report … did you mean
  "Order_Amt_Range"?`. The render still proceeds on the report's own defaults.

### Fixed

- **Formula / parameter / running-total / SQL-expression / special fields honor their stored format when rendered.**
  A placed field object carries a bound `value_type` only for database and summary fields; for every other kind the type
  lives on the field *definition*, so the render pipeline saw `Unknown` and formatted the value as a bare
  string/number — a formula field with a stored 2-decimal, currency, or date format leaf silently lost that format. The
  layout engine now resolves each object's **effective** value type (via the shared
  `rpt_model::field_object_value_type`) before picking the display format, so such a field renders with its own stored
  numeric/currency/date format (e.g. `$100.50`) instead of `100.50`.
- **Charts stay robust under degenerate and extreme data.** Non-finite (NaN/±Inf) plotted values are folded to zero at
  aggregation and clamped through a shared value→axis fraction in every axis/3-D renderer, so a divide-by-zero formula
  or an extreme magnitude can no longer produce runaway (i32-saturating) bar/riser geometry or an integer-overflow panic
  in the 3-D label placement; the value scale ignores non-finite values, and empty / all-zero / negative-only /
  high-cardinality (1000+ mark) series render without panicking.
- **A chart title no longer decodes to a garbage point size.** The stored title-size slot is shared with adjacent layout
  state and is not written by every authoring tool, so it could decode as a 2–4 pt or 200 pt+ title. A decoded size
  outside the range a real title occupies is now treated as "not an override" and falls back to the engine default.
- **A group-by formula fetches the database fields it references.** When a report grouped by a formula
  (`{@Key} = ToText({t.id},0) & " - " & {t.name}`), the live-DB query collected fields only from placed objects,
  selection formulas, running totals, and subreport links — never from a group's condition/sort formula. A field
  referenced *only* inside the group formula was left out of the `SELECT`, so the group key couldn't distinguish rows
  and every value collapsed into one group (e.g. a Top-N subreport showing one item instead of five, at a fraction of
  its true height). `rpt-query` now also walks each group's condition and sort formulas for their referenced fields.
- **Box and border background fills are no longer forced to white.** The border/format record's background colour was
  discarded by a bogus "white" sentinel on the fill's (inert) alpha byte, so every filled box decoded as white — a
  navy/rust column-heading bar rendered white-on-white (invisible). The fill colour is now always read from its RGB
  bytes, matching the engine (a genuinely auto/white box is unaffected).
- **Cross-tab dimensions bucket by their grouping period, and all measures render.** A cross-tab column bound to a date
  grouped monthly previously produced one column per raw date (an exploded, mislaid grid with wrong totals) instead of
  one per month; the dimension's stored grouping period (decoded from its grid-group leaf) is now applied. A cross-tab
  with several measures drew only the first; every declared measure now renders, stacked per cell.
- **Group header/footer bands sort by their decoded nesting level.** A 3-level nested group's areas (and their group
  associations) could come out in the wrong order when the group areas were stored, or named (`nameHeader`/
  `customeridHeader`, no trailing digit), out of nesting order; the level now comes from the per-area section-code
  record rather than binary-appearance order or an area-name digit.
- **The live-Postgres fetch streams instead of buffering the whole result set.** `rpt-db-postgres` now reads rows
  through a server-side portal (`query_raw`) rather than `query`, which materialized the entire joined result into a
  driver-side `Vec` before the pipeline saw a single row. On a large joined result this held two full copies of the
  result at once (the raw driver `Vec` plus the typed rows built from it)
  and could exhaust memory during the fetch phase. Driver-side memory is now O (1) in the row count — measured at ~60 MB
  flat for 5M wide rows versus ~1.46 GB (and growing linearly) for the buffered path — so a report whose live result
  runs to tens of millions of rows no longer doubles its peak footprint just to fetch. (The pipeline still materializes
  the selected rows into the in-memory `Dataset` for sort/group/summary, so a genuinely enormous result set can still be
  memory-bound there; that is a separate, larger streaming-layout effort.)
- **A cross-tab's decomposed cell objects are no longer drawn on top of its grid.** The decoder surfaces a cross-tab's
  internal cell/label/summary objects flat in the section, geometrically inside the cross-tab box; the layout now
  suppresses any object whose top-left sits within a cross-tab's bounds (the native grid already draws the whole pivot),
  so a cross-tab's grand-total labels and cells render once instead of doubled/tripled.
- **A tall subreport in the Report Header now flows across pages instead of clipping.** Cross-page inline-subreport flow
  previously engaged only for detail/group bands; a subreport in the report header (emitted at page-top) rendered
  clipped to page 1. The report header is now emitted through the same flow path, so a report-header subreport taller
  than a page splits across continuation pages (each repeating the page header) before the main body begins. The common
  case — a report header shorter than a page, with no overflowing subreport — is unchanged.
- **Scatter and bubble charts plot their points for group-scoped summary bindings.** An XY-scatter or bubble chart whose
  data bindings are group summaries (`Sum({field}, {group})`) — including a formula binding — now plots one point per
  category group (x/y/size = each binding's per-group value) instead of logging "no plottable points". An ungrouped
  point scatter falls back to one point per detail row, evaluating each binding formula-aware in the row context.
- **Section-level conditional formatting is now evaluated at render time.** A section's own `BackgroundColor`
  condition formula (e.g. `if {@IsUnderperformer} then RGB(255,220,220) else crWhite`) is resolved per record and tints
  the whole band, overriding the static section fill; a `Section_Visibility` condition suppresses the band per record
  like the static suppress flag. Previously only object-level conditional formats were honoured.
- **`ToText(<date/time>, "picture")` now formats the value instead of erroring.** The two-argument date/time overload of
  `ToText` (e.g. `ToText(CDate({?StatementDate}), "dd-MMM-yyyy")`, `ToText({date}, "yyyy-MM-dd")`) previously returned
  `EvalError::Unsupported`, which blanked the whole enclosing string concatenation. It now renders the Crystal picture
  tokens `d`/`dd`/`ddd`/`dddd`, `M`/`MM`/`MMM`/`MMMM`, `yy`/`yyyy`, `H`/`HH`/`h`/`hh`, `m`/`mm`, `s`/`ss`, `t`/`tt`,
  with separators and quoted runs passed through literally (month/weekday names + AM/PM from en-US). A datetime picture
  may mix date and time tokens in one string. Backed by a new `rpt_format_value::format_picture` helper.
- **The layout engine honours "Suppress If Blank Section", so a blank section reserves no vertical space.** A section
  flagged `EnableSuppressIfBlank` whose objects all resolve to no visible output (empty text/fields, no drawn border or
  visible fill) is now dropped during pagination instead of occupying its designed height. A report with a conditional
  group-footer note band (empty for most groups) previously accumulated that wasted height into one extra page; it now
  paginates to the same page count as the native engine.
- **The join builder follows link direction, so a table with redundant links is not reached backwards.** A stored
  `TableLink` is directional (`source.field = target.field` joins the target as a lookup onto the source). When a table
  was reachable by more than one link (a link-graph cycle), `rpt-query` could attach it by reversing a
  dimension-to-dimension link instead of via its own fact link — joining one dimension row to many fact rows, a
  cartesian blow-up. `join_order` now prefers forward links, then reversed links to a pure-source (fact) table, and only
  uses a reversed link into a dimension to break a deadlock — reaching each lookup from its fact.
- **Summaries over a formula field aggregate the evaluated value.** A declared summary whose summarized field is a
  `{@formula}` (e.g. `Sum of {@LineTotal}`) folded straight off the raw fetched row — which never holds a formula
  value — so every group and grand total was `Null`/blank. The fold now evaluates the formula per record before
  aggregating, matching a summary over a raw database field.
- **A placed summary field resolves by operation, not just field.** When one field carried several summaries (e.g. `Sum`
  and `Average` of it), the object showed the first summary of that field regardless of its operation (an `Avg` object
  rendered the `Sum`). Summary resolution now keys on `(operation, field, group scope)`.
- **A 1-argument (grand-total) summary resolves to the report total from any band.** `Sum({field})` with no group
  operand is the report grand total, but in a group/detail band it resolved to the innermost group's subtotal. It now
  always resolves against the grand total; the 2-argument form (`Sum({field}, {group})`) still resolves to the named
  group.
- **Summary functions inside a formula body resolve to the report's summaries.** A summary function over a reference —
  `Count({field}, {group})`, `Sum({field})` — evaluated to `Null` because only the array-literal aggregate form was
  supported. The formula engine now resolves such a call to the corresponding computed group/grand-total summary (the
  way the engine treats it — a reference to an existing subtotal), and a formula using one is classified
  `WhilePrintingRecords` so it runs after the summaries are computed.
- **Group header/footer areas are classified by their band marker, not their name.** A group band is named after its
  group field (RAS authoring writes e.g. `nameHeader`/`customeridHeader`, not `GroupHeaderArea1`), so the name-prefix
  heuristic silently classified those areas as `ReportHeader`. The area kind now comes from the authoritative
  band-marker record (`0x8d`–`0x99`) that parents each section, so a RAS-authored group lays out as `GroupHeader`/
  `GroupFooter` and its header/footer bands (and every field/summary in them) render. Reports whose area names are
  standard are unaffected (the marker yields the same kind).
- **Non-detail bands resolve their fields against the correct record.** A field/formula object in a report header,
  report footer, group footer, or page header/footer was evaluated with no current record, so database-field references
  (and formulas reading them) collapsed to `Null` and rendered blank. Each band now evaluates against Crystal's record
  context: the report's first record (report header), its last (report footer), the group's first/last record (group
  header/footer), and the page-boundary record (page header/footer). Detail bands and summary values are unchanged.
- **Record selection now resolves report parameters** (`build_dataset_with_params`): the selection and grouping formulas
  evaluate `{?Param}` against the supplied values, so a parameter-filtered report keeps the rows the parameters select
  instead of dropping every row. Previously the selection ran before parameters were attached, so a
  `{?Param}` reference failed to resolve and every record was dropped fail-open (an empty report). Unset declared
  parameters resolve to `Null`, so `HasValue({?Param})` reports false — matching the engine's treatment of an optional
  parameter left unset.
- **A declared parameter left unset at render binds to its stored default/current value.** When `rpt-render` ran a
  parameterized report with no `--param` override, the selection formula saw the parameter as unbound and skipped its
  filter, so every row rendered (e.g. an `IN`-list default selecting nothing). The CLI now binds each unsupplied
  parameter to its stored current value (else its default value (s)), so defaults feed the record selection exactly as
  when the engine runs a report accepting defaults; a parameter with no stored value still binds `Null`.
- **System-default integer fields group thousands** (`1,002`, not `1002`), matching the engine, which applies the same
  grouped number format to an integer field as to a decimal one; only the decimal places drop to zero.
- **The en-US system-default short date renders unpadded** (`M/d/yyyy`, e.g. `5/2/2023`), matching the engine, instead
  of zero-padding month and day (`05/02/2023`). A new `Locale::short_date_leading_zero` flag drives the system-default
  short-date pattern: false for en-US, true for the locales whose Windows short date pads (`dd/MM/yyyy` — en-GB, de-DE,
  fr-FR, es-ES, it-IT). Explicit stored date formats are unaffected.
- **Bar charts dispatch to the bar renderer explicitly** instead of the unsupported-type fallback, so a `Bar` chart no
  longer logs a spurious "chart type Bar is not yet supported" warning (the visual output was already a bar chart).
- **The gantt chart plots every datable record**, matching the engine. A hardcoded 60-row cap silently truncated a
  longer detail set (e.g. 109 project rows drawn as 60); the cap is now a high defensive guard (2000) that only trims a
  pathological set, with row-label thinning keeping dense charts legible.
- **"Hide (Drill-Down OK)" areas render no bands.** An area flagged `hide_for_drill_down` (the SDK's
  Hide-for-drill-down) was laid out normally, so a report that hides its detail to show a per-group summary dumped every
  detail row. The layout now contributes no bands for a hidden area (its structural bookkeeping — group level, footer
  pairing key, group format — is preserved), matching the engine's normal (non-drill-down) view.
- **An empty dataset renders the group-band skeleton instead of a blank page.** A report that defines groups but
  produced none (no records passed selection) dropped its whole body — the static content authored into the group
  header/footer (letterhead, column labels, totals captions) never rendered, so the page came out blank. The layout now
  emits the group headers (outermost→innermost) and footers (innermost→outermost) once for an empty grouped dataset,
  resolving field references against a synthetic empty row; no detail band is emitted.
- **Group selection applies a group-scoped summary at the level its group argument names.** A `GroupSelectionFormula`
  built from an inline summary — `DistinctCount({field}, {group}) > 1` — was fail-open (every group kept) because the
  safety gate rejected the summarized field and the filter only ran at the innermost leaf. The filter now allows
  references consumed by a group-scoped aggregation, resolves the summary from the group's computed subtotals, and
  prunes at the group level the summary's group argument names (matched on the full field name), dropping whole groups
  the way the engine does.
- **PDF text no longer inherits a leaked stroke** (doubled/haloed glyphs): a run drawn right after a line left krilla's
  stroke active, filling *and* stroking the glyphs; text now clears the stroke first.
- **`TotalPageCount` / `PageNofM` resolve the true final page count**, patched into every placed run once pagination
  completes (re-anchoring right/centre-aligned footers) instead of showing pages-so-far; the field also includes its
  `Page ` / ` of ` literals ("Page 1 of 37").
- **Conditional object visibility is evaluated** under its stored reserved name (was looked up under the wrong key), so
  per-row features like zebra backgrounds render.
- **Group-footer bands map to the correct group level by pairing each footer to its header by name** (the area name with
  its `Header`/`Footer` band token removed — `nameHeader`↔`nameFooter`), robust to whatever order the file presents
  footers in; a blind innermost-first reversal misattributed footers (and their subtotals) to the wrong nesting level,
  so an outer-group footer fired at every inner-group break. Falls back to the reversal when the names don't pair
  cleanly. **Group-scoped summaries disambiguate shared short names** by matching the full field name.
- **Lines and boxes read their stroke from the border record** (only thickness lives on the shape), so border-defined
  styles — including vertical column dividers — render instead of being dropped.
- **Row-background boxes underlay content and grow with the band**, covering a can-grow row's full height.
- **The PDF backend draws underline and strikethrough**, matching the other backends (HTML also gained strikethrough;
  existing output unchanged).
- **Pictures draw at their op-box size** (aspect/fit) and **detail text uses its stored font size** — fixing images
  drawn too large and mis-sized table text.
- **Conditional-format colours resolve to the correct hue**, and **page-header bands no longer render as black bars**.
- **A Postgres boolean bound to a String field reads as `1`/`0`** (psqlODBC `BoolsAsChar`), fixing a
  `{field} <> "1"` comparison broken by the live-DB path text-casting it to `true`/`false`.
- **Thin lines (~10 twips) render** (e.g. a footer divider), and **a table alias no longer decodes as its base name**.
- **The formula parser rejected valid Crystal, and the affected field rendered a wrong value.** A parenthesised group is
  a statement **sequence** with an optional trailing separator, not a single expression — how loop bodies and
  multi-statement branches are written (`Then (c := c + 1;) Else (c := c;)`). The parser failed at the first `;`, then
  compiled and evaluated the recovery AST, so a corpus report silently produced a wrong value.
- **A deeply-nested formula overflowed the stack and aborted the process.** `SIGABRT` cannot be caught, unlike a panic,
  so a pathological formula took down any host embedding the engine. Expression nesting is now capped at 128 with a
  diagnostic — far above anything a real formula reaches.
- **`codec::decode_contents` panicked on a length read from the stream.** The payload offset comes from the header
  record's own declared length, which on a damaged or non-report stream can point past the end. Now a `CodecError`.
- **A render that drops every row now says why.** `rpt-data`'s `DiagnosticSink` was attached by nothing outside its own
  tests, so every fail-open site produced silently wrong output — three corpus reports rendered zero rows from non-empty
  saved data and exited 0. A sink is now attached on every render path: the facade, `rpt-layout`'s subreport datasets,
  and the CLI. A selection that *failed* on every row is an error; one that cleanly *excluded* every row is a warning
  pointing at the report's parameters.
- **Failures that were previously swallowed are now reported.** Formula parse errors (once per formula at compile time,
  not per row, naming the formula and span — every one of the four compile sites bound them to `_`, so a syntax error
  evaluated from a partial parse with nothing said); EMF parse failures with their reason, distinguishing damaged data
  from a gap in this parser; a cross-tab cell's evaluation failure, which could blank a whole column silently; and the
  real reason no query could be built, in place of a synthesized "report has no database table".
- **Cause chains printed the same cause twice.** `connection failed: error connecting to server: error connecting to
  server: Connection refused`, and the same for a missing table. A variant that carries a `source` no longer
  interpolates it — applied to `rpt_reader::Error::Io`, `DbError::{Connect,Query}`, and `rpt_json::Error`, and
  documented as a convention in `rpt_reader::error`.
- **Panic sites in the formula engine and HTML backend hardened.** Formula text comes from an arbitrary `.rpt` and the
  engine is meant to be embeddable in an LSP, validator, or WASM sandbox, where a panic crashes the host. Seven internal
  invariants became `debug_assert!` plus a graceful release path — the new `EvalError::Internal` where a `Result`
  exists, a skipped op or object where none does.

### Removed

- **The dead error surface.** `rpt_reader::Error::Record`, `rpt_reader::RecordError`, and
  `ProjectErrorKind::UnknownRecord` are gone — publicly exported and documented, but with zero construction sites,
  because the `raise` layer is infallible by design and returns defaults for anything it cannot interpret.
  `Rpt::decode_coverage()` is how a decode gap is reported now.
  `Error::Project` survives behind the write-path clearance gate, with `UnclearedRecordEdit` as its remaining kind.
- **The blanket `From<std::io::Error> for rpt_reader::Error`.** A contextless `?` on an I/O call no longer compiles,
  which is the point: it made losing the path the path of least resistance. Use
  `rpt_reader::IoError::at(op, path, source)`.
- **The XML export surface.** `rpt xml-dump` (and its `--full` record-tree mode), the `rpt-xml` crate, the
  RptToXml-compatible serializers, and the committed XML baselines are gone; `rpt json-dump` and the JSON baselines
  replace them as the decode regression surface, and `rpt tree` remains for the record DOM. The XML shape existed to be
  diffed against RptToXml's XML — which could not reach large parts of the model (charts, cross-tabs, maps, OLAP) and
  whose element/attribute vocabulary forced the exporter to reshape, rename, and re-serialize decoded values. A plain
  serde dump of the model is a strictly better regression surface and costs nothing to keep exhaustive. With it go the
  XML-only behaviours: the RptToXml-shaped `RecordSelectionFormula` re-serializer, the
  `SavedData/@RecordCount` element (the count is still available as `Rpt::saved_record_count`, and `rpt saved` prints
  it), and the `--locale` flag that selected a host locale for the export's runtime-resolved display formats.
- **The derived analytics, and with them the last derived values in any export.** `Field.UseCount`, parameter usage,
  `<SummaryFields>`, the locale-resolved effective field format, and the derived preferred view no longer exist in the
  export path. They were reconstructions of engine bookkeeping that is not in the file: `UseCount` is a live reference
  count in the running engine and was never fully correct, and the effective display format needs a locale and formula
  return types, i.e. a consumer's context. The dump is now a pure projection of the decode, which is the only reason a
  baseline diff can be read as "the decoder's reading of this file changed". Consumers that need a derivation compute it
  themselves; `rpt_model::analysis` holds the dependency-free ones more than one consumer needs (e.g.
  `field_object_value_type`).
- **Saved-data cipher brute-forcing.** The saved-data batch key is known, so the IV/key-search path was removed from the
  reader.

## [0.2.0]

This release turns `rpt-rs` from a reader/exporter into a full reporting engine: a complete render pipeline (data →
layout → Page IR → HTML/SVG/PDF/PNG), a formula evaluator, live-database support, saved-data decoding, a byte-faithful
writer, and the test corpus to validate it all against the native Crystal engine.

### Added

- **End-to-end report rendering.** A new render & data pipeline built purely on the decoded model:
    - **`rpt-data`** — the record pipeline: `RowSource` → record selection → sort → grouping → summaries → running
      totals, with the formula evaluation context (`Global`/`Shared` variables, per-record cache, evaluation-time
      scheduling). Two-field summaries (`WeightedAverage`, `Correlation`, `Covariance`) resolve to an empty/unavailable
      value rather than a plausible-but-wrong one until the second field is decoded.
    - **`rpt-layout`** — the layout & pagination engine: places every object at its twip position, paginates
      band-by-band, and honours the section-break controls (New Page Before/After, Keep Group Together, Print at Bottom
      of Page, Reset Page Number After, Underlay Following Sections). Resolves each field's display format from the
      locale + its stored format spec.
    - **`rpt-pages`** — the backend-agnostic Page IR: `Rect` / `Ellipse` / `Line` / `Text` / `Polygon` / `Image`
      draw-ops in twips, solid/gradient/hatch fills, rotated text runs, image assets, checkpoints, and fidelity
      diagnostics. `serde`-serializable — the frozen contract between layout and backends.
    - **Four output backends** — `rpt-render-html` (self-contained XHTML, images inlined), `rpt-render-svg` (one file
      per page), `rpt-render-pdf` (krilla with real font-subset embedding, plus a dependency-free fallback writer), and
      `rpt-render-raster` (tiny-skia → PNG per page).
    - **`rpt-text`** — the real text stack (cosmic-text): font metrics, Unicode/CJK line breaking, bidi, and font
      fallback behind a swappable `TextLayout` trait, with bundled Liberation fonts and a dependency-free
      `ApproxLayout` for deterministic output.
    - **`rpt-render`** — the orchestration facade: `ReportDocument` (load → inspect → `to_pdf`/`to_html`/…) and an
      options-driven `render_with(report, RenderOptions)` threading the datasource (`Saved`/`Rows`/`Dataset`),
      parameters, locale, and subreport scope, with typed `RenderError`s.
    - **The `rpt-render` CLI** — renders a report to HTML / PDF / SVG / PNG from its saved data or a live database
      (`--db`), with `--param`, `--locale`, format inference from the output extension, and stdout piping.
    - The pipeline up to the Page IR plus the HTML and SVG backends compile to **WebAssembly**; a CI job guards the
      boundary.

- **Chart rendering — 16 chart types as native vector draw-ops** (no rasterization): bar (clustered / stacked / percent,
  multi-series), line, area, pie, doughnut, 3-D riser, 3-D surface, 3-D area, scatter, bubble, stock (hi-lo / OHLC),
  histogram, radar, gauge, funnel, and numeric-axis, plus a bar fallback (with a diagnostic) for types without a
  dedicated renderer. Matches the native engine's defaults: the full 20-colour palette, axis titles, tick density,
  compact temporal category labels with period bucketing (weekly/monthly/quarterly/…), label thinning on dense axes,
  family-dependent legend rules, and a perspective 3-D scene (corner room, floor grid, painter-sorted risers). Per-axis
  gridline modes are decoded from the chart styling record.

- **Cross-tab rendering.** A cross-tab pivots the dataset by row × column dimensions with an aggregate measure per cell
  and draws a native grid (cell rects, grid lines, headers, grand totals). Each dimension buckets by its stored grouping
  period — a date column grouped monthly produces one column per month (`M/YYYY`), not one per raw date — and every
  declared measure is rendered, stacked per cell (e.g. a Sum over a DistinctCount), each formatted for its operation.
  Current cut: one row dimension × one column dimension.

- **Live database support.** `rpt-query` generates the joined `SELECT` from the report's table/link graph — projecting
  only the tables and columns the report actually uses (matching the native engine's SQL and avoiding accidental
  cartesian joins) and pushing the translatable record-selection subset into `WHERE`. `rpt-db-postgres`
  and `rpt-db-sqlite` implement `RowSource` (native-only, isolated behind the trait so the portable core links no
  driver). Connection URLs are read only from the environment (`RPT_DB_URL`, `DATABASE_URL`,
  `RPT_DB_URL_<SERVER>`), never from flags.

- **Formula evaluation.** The `crystal-formula` engine gained a full evaluator: a bytecode VM (with a tree-walking
  reference evaluator), the builtin library across every family — string, math, conversion, date/time (incl.
  `DatePart` week-numbering modes), financial (`Pmt`/`FV`/`PV`/`NPV`/`IRR`/`Rate`/`DDB`/`SLN`/`SYD`), statistical
  (sample + population), and numeral (`ToWords`, `Roman`) — plus loop `Exit`, textual `#Month d, yyyy#` date literals,
  and a **semantic validation pass** (`validate`/`validate_str`): unknown/misspelled builtins with suggestions, arity
  and operator-type errors, and unknown field/parameter/formula references, as spanned, severity-tagged diagnostics.

- **Saved-data decoding.** A report saved with data now yields its schema (column names + types) and cached rows
  (`Report::saved_data`, the `rpt saved` subcommand) — both the external-memo and inline packed batch layouts decode,
  including the per-batch encryption and the memo heap.

- **`.rpt` writer (byte-faithful).** The decode pipeline is invertible at the record-substrate level:
  `Rpt::reencode` re-serializes, deflates, encrypts, and rewrites a valid `.rpt` that re-opens byte-identically at the
  logical level, and `Rpt::patch_record_leaf` overwrites a same-size region of a decoded record's leaf. Exposed as the
  `rpt reencode` / `rpt patch` subcommands. There is no model→records lowering yet.

- **New decoders in the reader:** charts (bindings, analytic layout, data-value labels, gridline modes), cross-tabs
  (dimensions, grid formats), object hyperlinks, hierarchical grouping, formula variables (name/type/scope), typed field
  sub-formats (number / currency / date / time / boolean / string masks), subreport re-import metadata, save metadata,
  designer state (rulers, guidelines), and the field-manager census. Every record type observed in the corpus is now
  named in the registry. `rpt` decode errors carry structured context (stream, byte offset, record type) via dedicated
  error types instead of message strings.

- **CLI additions.** `rpt inspect` shows each chart's data binding ("show value (s)" / "on change of");
  `rpt tree` prints the colorized decoded record DOM; `rpt streams` reports per-stream decode coverage;
  `rpt dump` is a byte-layout workbench (annotated hex, string scan, scalar probe, minimal-pair diff);
  `rpt saved` prints the decoded saved rows; `rpt sql` lists every SQL a report can run against its database (the
  generated join query + stored SQL Commands + SQL Expression fields, recursively through subreports, with
  connection/table provenance and a `--dialect` selector); `Report::objects()` / `objects_mut()` iterate all report
  objects.

- **Render-parity test corpus and infrastructure.** A committed corpus of 36 reports over one synthetic
  "parking" database, each with an XML decode baseline and an HTML render baseline, validated out-of-band against the
  native Crystal engine; golden-file tests for the Page IR and every backend; `docker-compose.yml` and a
  `Makefile` for the fixture database; DB-gated CI regression tests.

- **Documentation.** New guides: rendering (`docs/12-rendering.md`), a compile-verified render cookbook
  (`docs/13-render-examples.md`), and the five-part formula-engine set (architecture/VM, language reference, builtins,
  validation). GitHub-native Mermaid diagrams throughout, a rewritten README (status section, quick start, example
  render), and a docs↔code audit bringing the block catalog, support matrix, and saved-data docs in line with the code.

### Changed

- **Workspace restructured into a 20-crate, two-layer workspace** with compiler-enforced boundaries:
    - The semantic model moved out of `rpt` into the standalone, pure-data **`rpt-model`** crate (no I/O, WASM-safe,
      optional `serde`); `rpt` re-exports it as `rpt_reader::model`. The whole render/data pipeline depends on
      `rpt-model`, not the decoder, so the render stack links no CFB/inflate. Byte-level provenance notes live in the
      documentation-only
      `rpt_reader::provenance` module.
    - The formula language moved into the standalone **`crystal-formula`** crate (depends only on
      `rpt-format-value`), reusable without the binary reader (LSP, WASM sandbox, validator).
    - The `rpt-engine` crate was dissolved (its derived analytics now live in `rpt-cli`'s private
      `export::analysis`), and the `rpt-to-xml` binary was folded into `rpt-cli` as the **`rpt xml-dump`**
      subcommand — one `rpt` binary for all inspection and export. XML output is byte-identical.
- **Naming:** standalone "SAP" was dropped in favor of "Crystal Reports" across the README, docs, CLI help, and crate
  metadata.

### Removed

- The standalone **`rpt-to-xml`** binary (now `rpt xml-dump`) and the **`rpt-engine`** crate (dissolved into
  `rpt-cli` / `crystal-formula`), per the restructuring above.

## [0.1.0]

### Added

- **Saved data (stored rows).** Decodes the cached rows a report carries when saved with data (`SavedRecordsStream` +
  `MemoValuesStream`) and exports them as a `<SavedData>` element. See [`docs/06-saved-data.md`](docs/06-saved-data.md).
- **Formula syntax.** Reports each formula field's authoring dialect (`Syntax` — `crFormulaSyntaxCrystal` or
  `crFormulaSyntaxBasic`).
- **SQL-expression fields.** Decodes `{%name}` SQL-expression field references.
- **Dynamic parameters.** Recognises dynamic (list-of-values) parameters and reports their editing flags accordingly.
- **Top N / Bottom N group sorts.** Decodes group summary sorts and renders their summary sort expression and direction.
- **Percentage summaries.** Decodes percentage summaries (`PercentOfSum (…)`, etc.).
- **Running-total conditions.** Decodes running-total reset and evaluation conditions (`OnChangeOfField` / `OnFormula`).
- **Cross-section boxes.** Resolves a box that spans into a later section, reporting its end section and bottom edge.
- **Dynamic image locations.** Decodes a picture object's dynamic graphic-location formula, and its `EnableCanGrow`
  flag.
- **Subreport on-demand flag.** Decodes a subreport's `EnableOnDemand` flag.

### Fixed

- **Subreport parameter report name.** A parameter defined inside a subreport now reports that subreport's name as its
  `ReportName` (previously always empty); main-report parameters remain empty, as the engine emits.
- **Basic-syntax formulas.** A formula authored in Basic syntax now reports `Syntax="crFormulaSyntaxBasic"` (read from
  the formula record's stored dialect flag) instead of defaulting every formula to Crystal syntax.
- **Table aliases with spaces.** Aliases whose table name contains spaces (which Crystal substitutes with underscores)
  now match correctly, fixing the alias and the field long-names and formula forms derived from it.
- **Range parameter current values.** A range (non-discrete) current value now sets `HasCurrentValue`.
- **Summary result types.** `Maximum` / `Minimum` summaries report the summarized field's own type; a Currency running
  total reports a Number result, matching the engine.
- **Negative line heights.** A line drawn bottom-to-top reports its height as a magnitude.
- **Cross-tab keep-together.** A cross-tab no longer inherits the object-level keep-together flag.
- **Field use counts.** Corrects use-count totals for summary-sorted groups.

## [0.0.0]

The initial release: a pure-Rust reader for SAP Crystal Reports `.rpt` files, with no dependency on the SAP runtime, a
database connection, or any Windows component.

### Added

- **Direct `.rpt` decoding.** Opens the CFB/OLE2 compound file, decrypts the report streams (AES-128 in CFB mode, fixed
  key, per-stream IV) with a self-contained pure-Rust cipher, inflates the zlib payload, and tiles it into the record
  stream.
- **Recursive record tree.** Resolves the per-record content mask to build the full nested record tree, and recurses
  into subreports (`Subdocument N` storages).
- **Lossless record substrate.** Every record is preserved verbatim, including types not yet modelled, so reading never
  loses data.
- **Typed report model.** Projects records into a structured model: summary info; report and print options (paper size,
  orientation, margins, page rectangle); database (connections, tables, command/SQL tables, fields, joins); data
  definition (parameters with types and default/current values, formulas, groups, sort fields, summaries, running
  totals, record/group selection formulas); and report definition (areas, sections, and report objects with placement,
  fonts, borders, colors, and conditional formatting).
- **Subreport links.** Decodes how values pass between a report and its subreports.
- **Derived analytics (`rpt-engine`).** Computes values the engine derives rather than stores — including field use
  counts — backed by a Crystal formula lexer, parser, and reference/type analysis.
- **`rpt-to-xml` exporter.** Serializes a report to a structured XML document, with a `--full` mode that also dumps the
  complete decoded record tree.
- **`rpt` command-line inspector.** A read-only CLI with `inspect`, `inputs`, `streams`, and `strings` subcommands and a
  `--json` flag for machine-readable output.
- **Docker image.** A multistage build producing a minimal (~14 MB) image containing only the statically linked
  binaries.
- **Release workflow.** On a version tag, publishes cross-platform binaries (Linux, macOS, Windows) to a GitHub Release
  and pushes the Docker image to the GitHub Container Registry.
- **Documentation.** A guide to the `.rpt` format and the library under [`docs/`](docs/).
