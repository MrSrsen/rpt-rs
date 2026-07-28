# Meridian reports

Hand-authored Crystal Reports (`.rpt`) for the synthetic **Meridian Global Logistics** universe,
organized by division into subdirectories (`sales/`, `freight/`, `executive/`, `projects/`, `probes/`).
This is a **self-contained** corpus: every report here renders against the one seed database at
[`tests/meridian/sql/meridian.sql`](../sql/meridian.sql), discovered **recursively** by the
`postgres_fixtures` harness (`crates/rpt-render/tests/postgres_fixtures.rs`).

Each report's HTML baseline mirrors its path under this directory, with `.rpt`→`.html`, at
`tests/meridian/baselines/html/<relpath>.html` (e.g. `reports/sales/customer_statement.rpt` →
`baselines/html/sales/customer_statement.html`).

The seed is generated deterministically by the [`meridian-seed`](../../../apps/meridian-seed) crate;
its schema contract is `apps/meridian-seed/schema.sql`. Author reports against those exact table/column
names so the Crystal designer's `VerifyDatabase` binds cleanly.

**Status:** reports land here as they are authored; until a report has a blessed baseline it is counted
as skipped, so CI stays green.

## For report authors

- Bind to the column names in `schema.sql` — they are stable and business-realistic.
- Give every detail an explicit sort or group so the render is order-deterministic (Postgres does not
  guarantee fetch order).
- The data has real shape to exploit: a Q4 order surge, a mid-2022 fuel-price spike, one under-performing
  carrier (`carrier_id = 1`) for Radar/Gauge stories, a Pareto customer-revenue curve for Top-N, and
  right-skewed transit times for histograms. Nullable fields exercise `IsNull`/`HasValue`.
- After authoring, establish the cross-engine oracle and bless the HTML baseline
  (`make bless-fixtures`).
