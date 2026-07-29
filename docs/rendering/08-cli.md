# The `rpt-render` CLI

The `rpt-render-cli` app (`apps/rpt-render-cli`) builds the `rpt-render` binary: it opens a report, runs the data
pipeline + layout engine, and writes the paginated result through the chosen backend. It resolves the five inputs a
render needs — the report, a datasource, a locale, parameters, and an output destination.
The [rendering guide](README.md) covers the pipeline design; this is the flag-and-contract reference.

```
rpt-render <file.rpt> [OPTIONS]

DATASOURCE (default: the report's saved data if present, else empty)
    --saved            use the report's embedded saved data
    --db               fetch rows live from the database URL(s) in the environment
    --list-sources     print the report's live sources + the env var to set for each, then exit
    --list-fonts       print the font library a render would use, then exit (needs no <file.rpt>);
                       -v lists every face, --system-fonts lists the host's library instead

PARAMETERS
    -p, --param Name=Value   repeatable; repeat a name for a multi-value parameter

LOCALE
    --locale <tag>     e.g. en-US, de-DE (default: the host locale, else en-US)

FONTS (default: the bundled faces, so the render is reproducible on any machine)
    --system-fonts     lay out and embed the host's installed faces instead

OUTPUT
    -o, --output <path>  output file; '-' or omitted writes to stdout

PDF STANDARDS (default: an ordinary PDF, claiming no standard)
    --pdfa <1b|2b|3b|1a|2a|3a>  export against a PDF/A standard
    --pdfua              export against PDF/UA-1, the accessibility standard
    --tagged             emit a structure tree, claiming no standard
    --lang <tag>         the document's natural language (no report stores one)
    --title <text>       the document's title (default: the report's summary title)
    --alt <Object>=<text>  alternate text for a picture or chart; repeatable, empty = decorative

LOGGING
    -v, --verbose      also log the SQL sent, timings, and push-down decisions
    -q, --quiet        errors only

    -h, --help         show the help (its first line names the version)
    -V, --version      show the version and exit
```

PDF is a single self-contained file, safe to pipe to stdout, and it always overwrites. There is no `--format` flag:
PDF is the only output, so `-o` just names a path and any name is accepted. The one exception is a `.html`, `.svg` or
`.png` path, which is refused — those formats existed in 0.3.0, so such a command was written against the old CLI and
its extension says what the caller expected.

## Parameters

`-p Name=Value` supplies a report parameter (list them with `rpt inputs <file>`). Each value is coerced to the
parameter's declared type. Repeat the same name to build a multi-value parameter:

```sh
rpt-render report.rpt -p AsOfDate=2026-01-31 -p Region=West -p Region=East -o out.pdf
```

## Locale

`--locale <tag>` selects the locale used for date/number formatting. Resolution precedence: an explicit `--locale`
overrides the host OS locale (`LC_ALL` / `LC_NUMERIC` / `LANG`), which overrides the `en-US` fallback. Built-in tags are
`en-US`, `en-GB`, `de-DE`, `fr-FR`, `es-ES`, and `it-IT`; an unrecognized tag formats with the `en-US` fallback (the CLI
warns). This mirrors the native engine, which reads the host locale once at process start to resolve "System Default"
formats — there is no stored per-report locale.

## Fonts

Both halves of the font stack — the metrics the layout engine measures with and the faces the PDF embeds — come from the
**bundled** Liberation/DejaVu set by default, so the same report and the same rows produce the same pages and the same
bytes on every machine. `--system-fonts` switches both halves to the host's installed library.

Which one you want is a trade: the bundled Liberation faces are metric-compatible with **Arial, Times New Roman and
Courier New and nothing else**, so a report in any other family lays out differently under the two modes. Reach for
`--system-fonts` when the report's real fonts are installed on this machine and fidelity to them matters more than
reproducibility — output then depends on the host's font library, which is exactly why it is not the default. `-v` logs
which library the run used.

```sh
# Reproducible (default): the bundled faces, whatever this machine has installed
rpt-render report.rpt -o out.pdf

# This machine's real Arial / Calibri / …
rpt-render report.rpt --system-fonts -o out.pdf
```

## Database configuration (`--db`)

When a report has no saved data (or you pass `--db`), rows come from a live database. The connection is a single URL
taken **only from the environment**, never a command-line flag, so the password never appears in `ps` output or shell
history. The URL **scheme** selects the backend:

| Scheme                                                     | Status                          |
|------------------------------------------------------------|---------------------------------|
| `postgres://` (or `postgresql://`)                         | implemented                     |
| `sqlite:///path/to/file.db` (or `sqlite::memory:`)         | implemented                     |
| `mysql://` · `mariadb://` · `mssql://` (or `sqlserver://`) | recognized, not yet implemented |

For a single-server report, set `RPT_DB_URL` (or the 12-factor `DATABASE_URL` fallback; `RPT_DB_URL` takes precedence).
A report plus its subreports can read from more than one server; each distinct server gets its own
`RPT_DB_URL_<SERVER>` variable, where `<SERVER>` is the server name upper-cased with non-alphanumerics turned to `_`.
Run `--list-sources` to print the exact variable name for each source:

```sh
# Discover what --db needs for this report
rpt-render report.rpt --list-sources

# Render from a live database (URL from the environment), verbose
RPT_DB_URL='postgres://user:pass@host:5432/dbname' rpt-render report.rpt --db -o out.pdf -v
```

## Archival output (`--pdfa`)

`--pdfa 1b|2b|3b` exports against the corresponding ISO 19005 level-B standard (`PdfOptions::conformance` in the
library). `--pdfa 1a|2a|3a` and `--pdfua` are the levels that additionally require a tagged structure tree — see
[Tagged PDF](#tagged-pdf-and-the-accessible-levels).

The level is a **checked claim**, not a flag. A document that does not meet it fails the render — every unmet
requirement listed, exit code 1, nothing written — because a file carrying a conformance claim it does not honour is
worse than no file. So the levels' differences are visible at the command line:

| Level | PDF version | Forbids                                                |
|-------|-------------|--------------------------------------------------------|
| `1b`  | 1.4         | transparency, 16-bit images                            |
| `2b`  | 1.7         | — (transparency and JPEG2000 allowed)                  |
| `3b`  | 1.7         | — (as `2b`, plus attachments this backend never emits) |

A report that paints with transparency therefore fails `--pdfa 1b` and succeeds at `--pdfa 2b`; that is the standards
differing, not a defect, and the failure says so.

Conforming output differs from ordinary output by design: rasters embed with `/Interpolate false` (PDF/A forbids
interpolation), and the file carries an output intent, XMP metadata and a creation date. The date defaults to the Unix
epoch rather than the host clock, so a `--pdfa` render stays byte-reproducible for the same reason the bundled fonts are
the default.

## Tagged PDF and the accessible levels

`PdfOptions::tagged` adds a **structure tree**: what each mark means and in what order it is read. It is what lets a
screen reader announce a report as content rather than as a soup of positioned glyphs, and what makes copy-and-paste
come out in reading order. `Conformance::{PdfA1a, PdfA2a, PdfA3a, PdfUa1}` imply it.

The tree is reconstructed in `rpt-render-pdf`'s `tagging` module from the two things every draw-op carries — its
`ObjectRef` and its place in paint order:

| Page IR                                                                                                      | Structure                                                                 |
|--------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------|
| a run of ops sharing a section name, split at each section background and each repeat of an object placement | one band occurrence → `Div`                                               |
| a placed text object's wrapped lines                                                                         | `P` of one `Span` per line                                                |
| a picture or chart (all its paths *and* its labels — one graphic)                                            | one `Figure`, which must carry alternate text                             |
| a rule, a border, a fill, a section background                                                               | an **artifact**, skipped by assistive technology and absent from the tree |

Reading order is recovered per band: runs whose vertical extents overlap by more than half the shorter are one row, rows
read top to bottom and their members left to right. Overlap rather than an equal `top`, because a larger-pointed field
in the same row starts a few twips higher. **Only the tree is reordered** — the draws stay in paint order, which is what
decides what covers what, so tagging never changes how a page looks.

Two extraction defects are fixed on the way: a wrapped line declares (as `ActualText`) the space the wrapper consumed,
and a justified line declares its own words, since the justifier advances the pen across each gap instead of drawing a
space glyph.

### Where the artifact roles come from

What decides whether a run of text is document content or the running header repeated on every page is the *band* its
section belongs to, and an op carries only its section's stored **name** — in 80 of the 154 corpus fixtures at least one
of those names is `Section3`, `TSection7` or similar and says nothing. The producer knows, so it records it in
`PagedDocument::sections`
(see [the Page IR](03-page-ir.md)) and the backend maps band to role: a page header becomes a `/Header` pagination
artifact, a page footer a `/Footer`, everything else reads. The mapping is the backend's policy — the Page IR never
learns the word
"artifact" — and a section with no entry, or a band the mapping has no rule for, reads as content, because missing
information must never delete content from the reading order.

`Semantics::artifact_sections` remains as an **override**: `Some` replaces the derived classification wholesale, for a
caller that disagrees with the document or renders bare pages (`Some` of an empty map means "classified, no furniture").
This is why a tagging level needs the whole-document entry point, `try_render_document` — the pages-only entry points
carry no dictionary.

### What the Page IR cannot supply, and why the levels are refused without it

The rest a conforming tree needs is genuinely absent from the report, so `PdfOptions::semantics` takes it from the
caller and the render is **refused**, naming each one, when it is missing:

- **A natural language**, and for PDF/UA-1 a **title**. Never guessed: a French report declared `en` is read aloud in
  the wrong voice, and a file name is not a title.
- **Alternate text** for every figure, keyed by object name. An entry present but *empty* is the HTML `alt=""`
  convention — the caller looked, the graphic is decorative, and it becomes an artifact instead of a figure.

A document that carries no sections *and* gets no override is refused on the same terms: claiming PDF/UA over an
unclassified document would assert an accessibility it does not deliver — the failure mode the checked-claim design
exists to prevent.

`PdfOptions::tagged` on its own claims no standard, so it requires none of this and builds the best tree it can.

### What the report supplies, and what the caller must

`rpt_render::semantics_of(&Report)` fills in what the *report itself* states, and deliberately no more:

| `Semantics` field   | Where it comes from                                                                                                                                                                                                                                                                                          |
|---------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `title`             | `SummaryInfo.title`, when non-empty — 24 of the 154 corpus fixtures state one. Never the file name.                                                                                                                                                                                                          |
| `alt_text`          | each picture's and chart's stored `ToolTipText`, keyed by object name, through subreports too. **Literal values only** — a tooltip can be a conditional formula instead, which needs an eval context and a row. No corpus fixture sets one, so in practice every figure needs a caller-supplied description. |
| `language`          | **nothing** — a `.rpt` does not record the language of its text, and the render locale states number and date conventions rather than language (a US report formatted `de-DE` still reads in English). The caller states it or the level is refused.                                                         |
| `artifact_sections` | **nothing** — left `None`, which defers to the document's own band dictionary.                                                                                                                                                                                                                               |

The `rpt-render` CLI is a caller of that: `--tagged`/`--pdfua`/`--pdfa 1a|2a|3a` turn tagging on, `--lang`, `--title`
and `--alt <Object>=<text>` supply or override the rest, and the report's facts are read **only for a tagged run** — so
an ordinary render's metadata does not move because a report happens to carry a summary title. A refused render prints
each unmet requirement, names the figures still undescribed, points at the flag that supplies each, and exits non-zero
having written nothing.

Measured by hand — no CI job runs this — over the fixture corpus as it stood at 104 reports, at PDF/UA-1 through the CLI
(`--pdfua --lang en-US --title …`, no `artifact_sections` override, veraPDF 1.30.2 `-f ua1`): **56 pass, 0 fail, 48
refused** for an undescribed figure. Without `--title`, 80 of those 104 are refused for want of a title instead — the
count of reports whose author left it empty. Both controls hold: an untagged render of the same report fails `ua1`, and
a document with no band dictionary and no override is refused. The corpus stands at 154 reports now, so treat the counts
as the shape of the result rather than a current census; the zero failures is the load-bearing part.

## Provenance metadata

Every render — archival or not — names the engine that wrote it: `/Producer` and `/Creator` in the info dictionary,
`pdf:Producer` and `xmp:CreatorTool` in the XMP packet, all holding `rpt-rs <version>` from the same
`[workspace.package] version` the binaries report. A PDF/A render additionally cites it as the software agent in the XMP
provenance record.

The identity is a compile-time constant, so it changes when the version changes and at no other time — nothing here
reads a clock, a host name or a path, which is what lets the same input keep producing the same bytes. A library caller
can supply its own name or write none at all through `PdfOptions::producer` (`Producer::Named(…)` /
`Producer::Anonymous`); the CLI always writes the rpt-rs identity.

---

← [Charts and cross-tabs](07-charts-crosstabs.md) · [Index](README.md) ·
**Next:** [Testing the renderer](09-testing-parity.md) →
