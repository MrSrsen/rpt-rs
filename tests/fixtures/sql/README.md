# SQL render fixtures — data-driven render testing

> **DEPRECATED.** This is the legacy render-fixture corpus and harness. New synthetic-corpus work lives
> in the self-contained [`tests/meridian/`](../../meridian/) corpus (one seed at
> `tests/meridian/sql/meridian.sql`, recursive report discovery). This directory is slated for removal
> once the corpus migration completes; do not add new fixtures here.

Each fixture seeds a database with **synthetic** rows so a report can be rendered *with data* and
checked against a committed baseline. **PostgreSQL is the single DB technology** for render testing,
so the only variable under test is rendering, not the datasource.

- **`cargo test`** (`crates/rpt-render/tests/postgres_fixtures.rs`) — seeds the migration into a
  PostgreSQL server, renders the full pipeline (`rpt-db-postgres` → `rpt-data` → `rpt-layout` → Page IR)
  with the deterministic `ApproxLayout`, and compares the normalized Page IR against a committed baseline
  (`tests/fixtures/baselines/page-ir/<group>/<name>.json`). The server is taken from `RPT_DB_URL` (else
  `DATABASE_URL`); when neither is set the test **skips**, so a DB-less `cargo test` stays green.
Because the rows come from PostgreSQL byte-identically on every run, this is a render test **with data** —
the piece the stripped saved-data corpus can't provide (group summaries / chart series / running totals
are empty without live rows). Our datasource re-types every column against the report's declared field
types (not the DB's), so the render is backend-independent: a given set of rows produces the same Page IR
regardless of which engine served them.

## Running the corpus

Provision PostgreSQL with docker compose (see `docker-compose.yml` / `Makefile` at the repo root):

```sh
docker compose up -d --wait                                   # start (blocks until healthy)
export RPT_DB_URL=postgres://rpt:rpt@localhost:55432/rptfixtures
cargo test -p rpt-render --test postgres_fixtures
docker compose down                                           # stop + discard (ephemeral)
```

Or the one-shot Makefile target: `make test-fixtures-clean` (up → test → down). CI runs the same test
against a `postgres:16-alpine` service.

## Fixture layout

Fixtures are grouped one directory deep by report set; each baseline mirrors that `<group>/<name>`
path. A seed drives its reports in one of two cardinalities:

| File                                                   | What                                                              |
| ------------------------------------------------------ | ---------------------------------------------------------------- |
| `sql/<group>/<name>.sql`                               | **Per-report (1:1)** seed for `reports/<group>/<name>.rpt`.      |
| `sql/<group>/<group>.sql`                              | **Group-shared (1:N)** seed for *every* report under `reports/<group>/` (e.g. `parking/parking.sql`). |
| `baselines/page-ir/<group>/<name>.json`                | Committed Page IR baseline the render is compared against (blessed). |

The group-shared seed is the render-parity corpus model: one synthetic database (e.g. the `parking`
domain), many reports authored against it. A per-report seed still wins over the group-shared one for
that report; a group with no reports yet contributes no fixtures.

A fixture runs only when its report is present under `tests/fixtures/reports/`. A seed whose report
is absent is skipped, so a checkout always runs the full committed set and stays green.

## Page IR baselines

The test compares the **whole normalized Page IR** against the committed baseline. Rendering uses the
dependency-free `ApproxLayout` (no system fonts) so the IR is **byte-deterministic across hosts**. What makes a
baseline *correct* (not just stable) is review of the diff at the time it is blessed. Regenerate after an
intentional render change:

```sh
RPT_BLESS=1 cargo test -p rpt-render --test postgres_fixtures    # or: make bless-fixtures
```

## Authoring a fixture

1. Get the report's tables/sources: `rpt-render <report>.rpt --list-sources`, and per-column
   names+types from the QESession decode (`r.database.tables[].data_fields`).
2. Write portable ANSI DDL: `CREATE TABLE name (col INTEGER/VARCHAR(n)/DECIMAL(p,s)/DATE/…, …)`. Column
   names + types **must** match the report's stored bindings, or the Crystal engine's `VerifyDatabase`
   rejects the refresh. Keep it portable (no `SERIAL`/vendor types) so the same seed can drive future
   DB backends.
3. **Give every detail an explicit sort or group.** PostgreSQL does not guarantee row order without
   `ORDER BY`, so a report that lists rows in raw fetch order is non-deterministic. Report-level
   sorts/groups make the output stable.
4. **Satisfy the report's `RecordSelectionFormula`.** The pipeline applies it per row, so seed rows
   that match (e.g. `worrall/USStatesWithAbbreviations` selects `country_id = 2`). Check it with the
   record-selection dump before seeding.
5. Bless the baseline (`make bless-fixtures`), reading the diff before you do.

