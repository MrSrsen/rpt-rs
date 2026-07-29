# meridian-seed

Deterministic seed-database generator for the synthetic **Meridian Global Logistics (MGL)** render-test
corpus. It builds a fictional third-party-logistics universe — freight, trade, and capital projects —
and emits portable SQL that loads identically into **PostgreSQL** (perf path) and **SQLite**
(zero-process CI path). Nothing it produces is real: no PII, safe to commit.

The schema (tables, columns, FKs) and the generation model live in this crate's source; see `schema.sql` for the
generated DDL. Reports are authored by hand against this schema, so the column names and types here are
authoritative and stable.

## Usage

```sh
# Committed small-tier Postgres seed (what the render harness loads):
meridian-seed --tier small --dialect postgres --out tests/meridian/sql/meridian.sql

# SQLite variant of the same data:
meridian-seed --tier small --dialect sqlite --out meridian.sqlite.sql

# Large (perf) tier — ~1M tracking events, not committed:
meridian-seed --tier large --dialect postgres --out /tmp/meridian_large.sql

# DDL only (the human-readable schema contract):
meridian-seed --ddl-only --dialect postgres --out apps/meridian-seed/schema.sql
```

| Flag | Values | Default | Meaning |
| ---- | ------ | ------- | ------- |
| `--tier` | `small` \| `large` | `small` | Corpus size (see below). |
| `--dialect` | `postgres` \| `sqlite` | `postgres` | Target SQL dialect. |
| `--out` | path | stdout | Output file. |
| `--ddl-only` | — | off | Emit `CREATE TABLE`s only, no data. |

Output is DDL (`DROP`/`CREATE TABLE` in FK order) followed by batched multi-row `INSERT`s in FK-safe
order, so it loads with foreign keys enforced.

## Determinism guarantees

Determinism is the whole point — a blessed render baseline is only meaningful if the data behind it never
moves.

- A single named constant `SEED` (in `src/world.rs`) seeds one **`rand_chacha::ChaCha8Rng`** stream. The
  ChaCha8 keystream is stable across platforms and `rand_chacha` versions.
- Every quantity is a pure function of that stream, drawn in a **fixed generation order** (tables, then
  rows, then fields). No system entropy, no wall-clock, no `HashMap` iteration order (`Vec`/`BTreeMap`
  only). All dates derive from fixed epoch constants, never `now()`.
- The distribution helpers (`normal`/`lognormal`/`poisson`/`pareto`/`weighted` in `src/rng.rs`) are
  hand-rolled on the raw 64-bit output rather than delegating to `rand`'s distributions, whose algorithms
  are not guaranteed stable across releases.
- Result: **running the generator twice with the same arguments produces byte-identical output.**

```sh
meridian-seed --tier small --dialect postgres --out a.sql
meridian-seed --tier small --dialect postgres --out b.sql
diff a.sql b.sql        # identical
```

## Tiers (sizing)

`large` is a pure ×100 scale-up of the master/fact entity counts from the same seed (reference/geography
and the daily/monthly time series stay fixed); it stays deterministic and FK-consistent.

| Table (committed `small`) | ~rows | | Table | ~rows |
| ------------------------- | ----- |-| ----- | ----- |
| `tracking_event`          | 10k   | | `shipment_leg` | 2.5k |
| `order_line`              | 4.8k  | | `shipment_charge` | 2.8k |
| `sales_order`             | 1.5k  | | `fuel_price` | 2.9k (4 fuels × 730 d) |
| `invoice` / `invoice_line`| 1.3k / 1.8k | | `exchange_rate` | 0.6k (24 ccy × 24 mo) |
| `payment`                 | 1.1k  | | `carrier_scorecard` | 0.96k (40 × 24 mo) |
| `shipment`                | 1.0k  | | `customer` / `product` | 400 / 800 |
| `city` / `province`       | 1.0k / 214 | | `facility` / `supplier` | 120 / 120 |

The committed small-tier Postgres file is ~2.8 MB.

## Realistic distributions

Business data is skewed and heavy-tailed; each quantity is drawn from a distribution matching its shape
(all from the fixed seed):

- **Pareto** — customer spend weight (a few strategic accounts dominate order volume → meaningful Top-N;
  top 20% of customers hold ~68% of revenue). Also sets each customer's tier.
- **Log-normal** — `product.unit_price`, `order_line` amounts, `shipment` weight/volume/`freight_cost`,
  leg cost/distance, and shipment **transit time** (right-skewed so the transit histogram has a real tail).
- **Poisson** — counts: lines per order, events per shipment, legs per shipment.
- **Seasonal** — order/shipment dates weighted toward Q4 (the peak-season surge).
- **Autocorrelated random walk** — `fuel_price` OHLC and `exchange_rate` (serially correlated), with a
  deliberate mid-2022 fuel-price spike. OHLC invariant `low ≤ open,close ≤ high` holds per row.
- **Bounded normal** — `carrier_scorecard` metrics and tracking `temperature_c`, with **carrier 1** a
  deliberate under-performer/outlier (drives the Radar/Gauge narrative).
- **Weighted categorical** — carrier mode (Road > Sea > Air > Rail), service level, and customer region.

Nullable fields (`shipment.actual_delivery`, `product.description`, `tracking_event.temperature_c`, …)
are NULL at a realistic rate so reports exercise `IsNull`/`HasValue`.

## Blobs

Every `image`/`logo`/`photo` column carries a tiny procedurally-generated solid-color PNG (one swatch
per entity kind, encoded by `src/png.rs` with no compression dependency) — enough to exercise
Picture/Blob rendering. The bytes are a valid, decodable PNG (signature `89 50 4E 47 …`).

## Module map

| File | Responsibility |
| ---- | -------------- |
| `main.rs` | CLI parsing + orchestration. |
| `rng.rs` | The ChaCha8 stream and all distribution helpers. |
| `calendar.rs` | Day-count ↔ civil-date arithmetic (Hinnant), date/timestamp formatting. |
| `sql.rs` | Portable SQL model: dialects, column types, tables, values, the emitter. |
| `pools.rs` | Fixed reference data (real ISO codes) and synthetic name fragments. |
| `png.rs` | Deterministic placeholder-PNG encoder. |
| `world.rs` | Generation state, sizing, shared value helpers, reference tables. |
| `master.rs` / `fact.rs` / `market.rs` | The master, fact, and market/KPI generators. |

## Regenerating the committed fixtures

```sh
cargo build -p meridian-seed --release
target/release/meridian-seed --tier small --dialect postgres \
    --out tests/meridian/sql/meridian.sql
target/release/meridian-seed --ddl-only --dialect postgres \
    --out apps/meridian-seed/schema.sql
```
