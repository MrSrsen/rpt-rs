# SQL render fixtures — data-driven render testing

Each fixture seeds a database with **synthetic** rows so a report can be rendered *with data* and
checked against an oracle. **PostgreSQL is the single DB technology** for render testing: the same
portable `.sql` migration drives two consumers, so the only variable under test is rendering, not the
datasource.

- **`cargo test`** (`crates/rpt-render/tests/postgres_fixtures.rs`) — seeds the migration into a
  PostgreSQL server, renders the full pipeline (`rpt-db-postgres` → `rpt-data` → `rpt-layout` → Page IR
  → HTML) with the deterministic `ApproxLayout`, and compares the HTML against a committed baseline
  (`tests/fixtures/baselines/html/<group>/<name>.html`). The server is taken from `RPT_DB_URL` (else
  `DATABASE_URL`); when neither is set the test **skips**, so a DB-less `cargo test` stays green.
- **the out-of-band cross-engine oracle** — seeds the SAME `.sql` into a postgres container and
  renders the report through the **real Crystal engine**, then scores positioned parity — validating
  that a baseline is right.

Because both stacks read byte-identical rows from PostgreSQL, this is a render oracle **with data** —
the piece the stripped saved-data corpus can't provide (group summaries / chart series / running totals
are empty without live rows). Our datasource re-types every column against the report's declared field
types (not the DB's), so the render is backend-independent: a given set of rows produces the same HTML
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
against a `postgres:16` service.

## Fixture layout

Fixtures are grouped one directory deep by report set; each baseline mirrors that `<group>/<name>`
path (like the XML baseline harness). A seed drives its reports in one of two cardinalities:

| File                                                   | What                                                              |
| ------------------------------------------------------ | ---------------------------------------------------------------- |
| `sql/<group>/<name>.sql`                               | **Per-report (1:1)** seed for `reports/<group>/<name>.rpt`.      |
| `sql/<group>/<group>.sql`                              | **Group-shared (1:N)** seed for *every* report under `reports/<group>/` (e.g. `parking/parking.sql`). |
| `baselines/html/<group>/<name>.html`                   | Committed HTML baseline the render is compared against (blessed). |
| `sql/private/<group>/<name>.sql`                       | Fixtures for **private** reports (real schema names) — **gitignored**. |
| `baselines/html/private/<group>/<name>.html`           | Private HTML baselines — **gitignored**.                         |

The group-shared seed is the render-parity corpus model: one synthetic database (e.g. the `parking`
domain), many reports authored against it. A per-report seed still wins over the group-shared one for
that report; a group with no reports yet contributes no fixtures.

A fixture runs only when its report is present: **public** reports live in `tests/fixtures/reports/`
(committed); **private** reports in `samples/` (gitignored). Fixtures whose report is absent are
skipped, so a clean CI checkout runs the public set and stays green.

## HTML baselines

The test compares the **whole rendered HTML** against the committed baseline, exactly like the XML
exporter's baseline test (`crates/rpt-cli/tests/baseline.rs`). Rendering uses the dependency-free
`ApproxLayout` (no system fonts) so the HTML is **byte-deterministic across hosts**. What makes a
baseline *correct* (not just stable) is the out-of-band cross-engine oracle: it renders the same
seed through the real Crystal engine and reports positioned parity — so a blessed baseline is a snapshot of
an engine-verified render. Regenerate after an intentional render change:

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
   `ORDER BY`, so a report that lists rows in raw fetch order is non-deterministic (and can't reach
   cross-engine parity either). Report-level sorts/groups make the output stable.
4. **Satisfy the report's `RecordSelectionFormula`.** The pipeline applies it per row, so seed rows
   that match (e.g. `worrall/USStatesWithAbbreviations` selects `country_id = 2`). Check it with the
   record-selection dump before seeding.
5. Establish the oracle: run the cross-engine oracle for the report (confirms our render matches the
   Crystal engine at 100% positioned parity), then bless the baseline (`make bless-fixtures`).

## Command-table reports (private / SAP-B1)

Every private report — and most SAP-B1 (`ajryan_*`/`boyumit_*`) — bind a Crystal
**Command** (raw SQL passed verbatim as `(<command>) AS "alias"`). If that command is simple ANSI SQL,
seeding its **underlying** tables makes it run on PostgreSQL too. Commands that use vendor-specific SQL
(T-SQL/HANA functions, stored procs) need a command-override seam in `rpt-query` (replace the verbatim
command with a synthetic SELECT over seeded tables) before they can be fixtured. Command SQL contains
real schema, so those fixtures live under `private/` (gitignored).
