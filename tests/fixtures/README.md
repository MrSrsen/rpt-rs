# Test fixtures

Public Crystal Reports `.rpt` files and their baselines, used by three regression tests: the JSON decode
baseline (`apps/rpt-cli/tests/json_baseline.rs`), the data-driven HTML render baseline
(`crates/rpt-render/tests/postgres_fixtures.rs`), and the data-free typography render baseline
(`crates/rpt-render/tests/typography_baselines.rs`).

- `reports/` — the `.rpt` fixtures.
- `baselines/json/` — the committed JSON decode baselines (`<group>/<name>.json`), the full serde
  serialization of the decoded model.
- `baselines/html/` — the committed HTML render baselines (`<group>/<name>.html`), one per report seeded
  from `sql/<group>/`.
- `baselines/page-ir/` — the committed normalized Page-IR baselines (`<group>/<name>.json`), the
  structural layout contract (op kinds, twip positions, text, resolved font).
- `sql/` — SQL migrations (schema + synthetic seed) for the data-driven render test; see `sql/README.md`.

## The `typography/` group

Five synthetic, **data-free** reports (no tables, groups, formulas or datasource — just static
`TextObject`s in the Report Header), each isolating one font/text axis so a render change shows up
against exactly one variable:

| Fixture                  | Exercises                                                             |
| ------------------------ | --------------------------------------------------------------------- |
| `font_sizes`             | 17 sizes, 6–72 pt                                                     |
| `font_faces`             | the common Windows faces (Arial, Calibri, Georgia, Consolas, …)       |
| `font_styles`            | bold / italic / underline / strikethrough and all 10 combinations     |
| `text_color_align`       | 9 colors, the four alignments, background fills                      |
| `paragraph_typography`   | line spacing, first-line/left/right indent, character spacing         |

Because they bind no datasource these render with no database, so unlike `postgres_fixtures.rs` the
typography harness runs on every checkout. They were authored programmatically; the known
divergences from the format's intended behaviour (text rotation not applied, justified alignment not
stretched, `LineSpacing` and `CharacterSpacing` unmodelled) are tracked as their own tickets.
**The baselines freeze our current behaviour** — a baseline
moving when one of those lands is the expected signal, so re-bless it then:

```sh
RPT_BLESS=1 cargo test -p rpt-render --test typography_baselines
```

Text geometry comes from whichever faces the render resolved, so the harness renders through
`FontProvider::bundled()` — the compiled-in Liberation/DejaVu set, never the host's installed library
(also the pipeline default). The baselines therefore mean the same thing on every machine, and a bare
runner with no fonts cannot read as a layout regression.

## The JSON decode baseline

The test runs `rpt json-dump` over every fixture and compares the output against the committed baseline. The dump
needs no sandbox — it takes no locale, `source_path` is a stored field rather than the invocation path, and nothing
else in the output depends on where the report sits — so it is byte-identical on every machine.

Regenerate the baselines after an intentional decode change:

```sh
RPT_BLESS=1 cargo test -p rpt-cli --test json_baseline
```

## Sources and attribution

These are publicly available sample reports. All rights remain with their respective authors; they are included here
only as test fixtures.

| Prefix           | Source                                                                                                   |
| ---------------- | -------------------------------------------------------------------------------------------------------- |
| `worrall_*`      | [worrallbrian/crystal_reports](https://github.com/worrallbrian/crystal_reports)                          |
| `benbrahim777_*` | [benbrahim777/Crystal-Reports](https://github.com/benbrahim777/Crystal-Reports) — Xtreme sample reports. |
