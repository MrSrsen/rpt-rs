# Meridian Global Logistics — database schema

The complete table and field catalog for the synthetic MGL database. This is the contract report authors bind against;
it matches the generated DDL in `apps/meridian-seed/schema.sql` and the seed in `tests/meridian/sql/meridian.sql`.

**Conventions.** Portable ANSI types drive both PostgreSQL (perf / Crystal oracle) and SQLite (fast CI) from one seeder.
The only per-engine differences are: `BYTEA`↔`BLOB` (image/logo/photo columns) and `BOOLEAN`↔`INTEGER`. Primary keys are
`*_id` integers — `BIGINT` on the high-volume child tables (`order_line`, `invoice_line`, `shipment_leg`,
`tracking_event`, `shipment_charge`). Text keys such as `currency_code`/`iso2`/`iso3` are `VARCHAR` (shown below as their
generated widths). `→ table` marks a foreign key; *(null)* marks a nullable column (reports use these to exercise
`IsNull`/`HasValue`). All data is deterministic (fixed-seed).

> **Authoritative source:** `apps/meridian-seed/schema.sql` is the generated DDL of record; this table matches it.
> Where a precise width/precision matters for a report, check `schema.sql`.

Four layers: **reference** (small, fixed lookups) · **master** (entities) · **fact** (transactions, the volume) ·
**market/projects/KPI**.

---

## Relationships at a glance

- **Geo hierarchy:** `region` → `country` → `province` → `city` → (`facility`, `customer`, `supplier` locate here).
- **Product hierarchy:** `product_category` self-references (`parent_id`) → `product` → `order_line`.
- **Org hierarchy:** `department` self-references; `employee.manager_id` self-references (the org tree).
- **Order→fulfilment chain:** `sales_order` → `order_line`; `sales_order` → `invoice` → `invoice_line` → `payment`;
  `sales_order` → `shipment` → `shipment_leg` → `tracking_event`, with `shipment_charge` hanging off `shipment`.
- **Fleet:** `carrier` → `vehicle`; both referenced by `shipment_leg`.
- **Market/KPI:** `fuel_price`, `exchange_rate` (time series); `project` → `project_task`; `carrier_scorecard` → `carrier`.

---

## 1. Reference / lookup tables

### `region`
| Field | Type | Description |
| ----- | ---- | ----------- |
| region_id | INTEGER PK | Surrogate key. |
| code | VARCHAR(8) | AMER, EMEA, APAC, LATAM, MEAF. |
| name | VARCHAR(64) | Display name. Top of the geo hierarchy; top group level in R07. |

### `country`
| Field | Type | Description |
| ----- | ---- | ----------- |
| country_id | INTEGER PK | Surrogate key. |
| region_id | INTEGER → region | Owning region. |
| iso2 | CHAR(2) | ISO 3166-1 alpha-2 (real codes). |
| iso3 | CHAR(3) | ISO 3166-1 alpha-3. |
| name | VARCHAR(80) | Country name. |
| currency_code | CHAR(3) → currency | Country's default currency. |

### `province`
| Field | Type | Description |
| ----- | ---- | ----------- |
| province_id | INTEGER PK | Surrogate key. |
| country_id | INTEGER → country | Owning country. |
| code | VARCHAR(8) | State/province code. |
| name | VARCHAR(80) | Province name. |

### `city`
| Field | Type | Description |
| ----- | ---- | ----------- |
| city_id | INTEGER PK | Surrogate key. |
| province_id | INTEGER → province | Owning province. |
| name | VARCHAR(80) | City name. |
| latitude | NUMERIC(9,6) | For map charts. |
| longitude | NUMERIC(9,6) | For map charts. |
| population | INTEGER | Bubble-size / weighting fodder. |

### `currency`
| Field | Type | Description |
| ----- | ---- | ----------- |
| currency_code | CHAR(3) PK | ISO 4217 (USD, EUR, GBP, JPY, …). |
| name | VARCHAR(48) | Currency name. |
| symbol | VARCHAR(4) | Display symbol. |
| decimal_places | INTEGER | Minor-unit digits (JPY=0, most=2) — currency formatting. |

### `industry`
| Field | Type | Description |
| ----- | ---- | ----------- |
| industry_id | INTEGER PK | Surrogate key. |
| name | VARCHAR(64) | Customer segment (Retail, Automotive, Pharma, …). Grouping dimension. |

### `product_category`
| Field | Type | Description |
| ----- | ---- | ----------- |
| category_id | INTEGER PK | Surrogate key. |
| parent_id | INTEGER → product_category *(null)* | Self-referencing hierarchy (Division→Group→Class). NULL at top. |
| name | VARCHAR(80) | Category name. |
| depth | INTEGER | 1/2/3 — hierarchy level (convenience for grouping). |

### `unit_of_measure`
| Field | Type | Description |
| ----- | ---- | ----------- |
| uom_id | INTEGER PK | Surrogate key. |
| code | VARCHAR(8) | EA, KG, PLT, CTN, … |
| name | VARCHAR(32) | Full name. |

### `incoterm`
| Field | Type | Description |
| ----- | ---- | ----------- |
| incoterm_id | INTEGER PK | Surrogate key. |
| code | VARCHAR(4) | FOB, CIF, DDP, EXW, … |
| name | VARCHAR(48) | Full term. |

### `shipment_mode`
| Field | Type | Description |
| ----- | ---- | ----------- |
| mode_id | INTEGER PK | Surrogate key. |
| code | VARCHAR(8) | SEA, AIR, ROAD, RAIL. |
| name | VARCHAR(32) | Display name. Pie/doughnut split (R12). |

### `service_level`
| Field | Type | Description |
| ----- | ---- | ----------- |
| level_id | INTEGER PK | Surrogate key. |
| code | VARCHAR(8) | ECON, STD, EXP. |
| name | VARCHAR(20) | Economy / Standard / Express / **Priority** (4 levels). Group level in R07. |
| target_transit_days | INTEGER | SLA target — the gauge target in R09. |

### `order_status` / `shipment_status` / `payment_status`
Three parallel status lookups (same shape). **Note the PK column is `id`** (the fact tables reference it via their own
`status_id` column, e.g. `sales_order.status_id → order_status.id`).
| Field | Type | Description |
| ----- | ---- | ----------- |
| id | INTEGER PK | Surrogate key (referenced by `*.status_id`). |
| code | VARCHAR(10) | Machine code (e.g. QUOTED, PICKED, DELIVERED / OPEN, PAID). |
| name | VARCHAR(30) | Display label — `Switch`/`Select Case` formulas, funnel stages. |
| sort_order | INTEGER | Pipeline order (funnel/stage sequencing). |

### `fuel_type`
| Field | Type | Description |
| ----- | ---- | ----------- |
| fuel_id | INTEGER PK | Surrogate key. |
| code | VARCHAR(8) | DIESEL, JET, BUNKER, LNG. |
| name | VARCHAR(32) | Display name. |

### `charge_type`
| Field | Type | Description |
| ----- | ---- | ----------- |
| charge_id | INTEGER PK | Surrogate key. |
| code | VARCHAR(16) | FREIGHT, FUEL, CUSTOMS, INSURANCE, HANDLING. |
| name | VARCHAR(48) | Display name. Stacked-bar series (R14). |

---

## 2. Master data

### `facility`
| Field | Type | Description |
| ----- | ---- | ----------- |
| facility_id | INTEGER PK | Surrogate key. |
| city_id | INTEGER → city | Location (geo group level). |
| type | VARCHAR(16) | WAREHOUSE / HUB / PORT / TERMINAL. |
| code | VARCHAR(16) | Short code. |
| name | VARCHAR(80) | Facility name. |
| capacity_m3 | NUMERIC(12,2) | Storage/throughput capacity. |
| opened_date | DATE | Commissioning date. |
| image | BYTEA/BLOB | Placeholder image — PictureObject / underlay (R01). |

### `carrier`
| Field | Type | Description |
| ----- | ---- | ----------- |
| carrier_id | INTEGER PK | Surrogate key. |
| name | VARCHAR(80) | Carrier name. |
| primary_mode_id | INTEGER → shipment_mode | Main transport mode. |
| scac | VARCHAR(8) | Standard carrier alpha code. |
| is_own_fleet | BOOLEAN | MGL-owned vs contracted. |
| logo | BYTEA/BLOB | Placeholder logo — Blob/Picture. |

### `vehicle`
| Field | Type | Description |
| ----- | ---- | ----------- |
| vehicle_id | INTEGER PK | Surrogate key. |
| carrier_id | INTEGER → carrier | Operating carrier. |
| type | VARCHAR(16) | TRUCK / VESSEL / AIRCRAFT / RAILCAR. |
| registration | VARCHAR(16) | Reg/tail/IMO number. |
| capacity_kg | NUMERIC(12,2) | Weight capacity. |
| capacity_m3 | NUMERIC(12,2) | Volume capacity. |
| model_year | INTEGER | Build year. |

### `supplier`
| Field | Type | Description |
| ----- | ---- | ----------- |
| supplier_id | INTEGER PK | Surrogate key. |
| city_id | INTEGER → city | Location. |
| name | VARCHAR(80) | Supplier name. |
| rating | NUMERIC(3,1) | 0–5 quality rating. |
| since_date | DATE | Onboarded date. |
| logo | BYTEA/BLOB | Placeholder logo. |

### `product`
| Field | Type | Description |
| ----- | ---- | ----------- |
| product_id | INTEGER PK | Surrogate key. |
| category_id | INTEGER → product_category | Catalog category (hierarchy). |
| supplier_id | INTEGER → supplier | Source supplier. |
| uom_id | INTEGER → unit_of_measure | Selling unit. |
| sku | VARCHAR(24) | Stock-keeping unit. |
| name | VARCHAR(120) | Product name. |
| description | VARCHAR(400) | Long text — can-grow field (R02). |
| unit_price | NUMERIC(12,2) | List price. |
| currency_code | CHAR(3) → currency | Price currency. |
| weight_kg | NUMERIC(10,3) | Unit weight. |
| volume_m3 | NUMERIC(10,4) | Unit volume. |
| hs_code | VARCHAR(12) | Harmonized-system customs code. |
| is_hazardous | BOOLEAN | Dangerous-goods flag. |
| is_active | BOOLEAN | Catalog-active flag. |
| image | BYTEA/BLOB | Placeholder image — PictureObject (R02). |

### `department`
| Field | Type | Description |
| ----- | ---- | ----------- |
| department_id | INTEGER PK | Surrogate key. |
| parent_id | INTEGER → department *(null)* | Self-referencing org hierarchy. |
| name | VARCHAR(64) | Department name. |

### `employee`
| Field | Type | Description |
| ----- | ---- | ----------- |
| employee_id | INTEGER PK | Surrogate key. |
| department_id | INTEGER → department | Home department. |
| manager_id | INTEGER → employee *(null)* | Manager (self-ref org tree — hierarchical grouping, R19). |
| first_name | VARCHAR(48) | Given name (synthetic). |
| last_name | VARCHAR(48) | Family name (synthetic). |
| title | VARCHAR(64) | Job title. |
| hire_date | DATE | Start date. |
| salary | NUMERIC(12,2) | Annual salary. |
| currency_code | CHAR(3) → currency | Salary currency. |
| region_id | INTEGER → region | Operating region. |
| photo | BYTEA/BLOB | Placeholder photo — Blob/Picture (R19). |
| is_active | BOOLEAN | Employment flag. |

### `customer`
| Field | Type | Description |
| ----- | ---- | ----------- |
| customer_id | INTEGER PK | Surrogate key. |
| city_id | INTEGER → city | Billing location (→ geo hierarchy). |
| industry_id | INTEGER → industry | Segment. |
| name | VARCHAR(120) | Account name (synthetic). |
| account_code | VARCHAR(16) | Customer number. |
| credit_limit | NUMERIC(14,2) | Credit ceiling. |
| currency_code | CHAR(3) → currency | Billing currency. |
| since_date | DATE | Relationship start. |
| sales_rep_id | INTEGER → employee | Account owner. |
| tier | VARCHAR(16) | STRATEGIC / KEY / STANDARD / SMALL (Pareto-driven). |
| is_active | BOOLEAN | Active flag. |
| logo | BYTEA/BLOB | Placeholder logo — underlay (R01). |

### `customer_contact`
| Field | Type | Description |
| ----- | ---- | ----------- |
| contact_id | INTEGER PK | Surrogate key. |
| customer_id | INTEGER → customer | Owning account (subreport link, R01). |
| name | VARCHAR(96) | Contact name (synthetic). |
| email | VARCHAR(120) | Email (synthetic @meridian-demo). |
| phone | VARCHAR(32) | Phone (synthetic). |
| role | VARCHAR(48) | Contact role. |

---

## 3. Fact / transactional (the volume & performance surface)

Row counts: **small (committed/CI) → large (perf)**.

### `sales_order`  · ~1.5k → ~300k
| Field | Type | Description |
| ----- | ---- | ----------- |
| order_id | INTEGER PK | Surrogate key. |
| customer_id | INTEGER → customer | Buyer. |
| order_date | DATE | Placed date (seasonal, Q4-surged). |
| required_date | DATE | Requested delivery. |
| status_id | INTEGER → order_status | Lifecycle status (funnel). |
| currency_code | CHAR(3) → currency | Order currency. |
| sales_rep_id | INTEGER → employee | Owning rep. |
| ship_to_city_id | INTEGER → city | Destination city. |
| incoterm_id | INTEGER → incoterm | Delivery terms. |
| total_amount | NUMERIC(14,2) | Order value (Pareto across customers). |

### `order_line`  · ~5k → ~1M
| Field | Type | Description |
| ----- | ---- | ----------- |
| order_line_id | INTEGER PK | Surrogate key. |
| order_id | INTEGER → sales_order | Parent order. |
| product_id | INTEGER → product | Line product. |
| quantity | INTEGER | Units (Poisson-ish). |
| unit_price | NUMERIC(12,2) | Price at sale. |
| discount_pct | NUMERIC(5,2) | Line discount. |
| line_amount | NUMERIC(14,2) | Extended amount (log-normal). Sum/running-total + crosstab source. |

### `invoice`  · ~1.5k → ~300k
| Field | Type | Description |
| ----- | ---- | ----------- |
| invoice_id | INTEGER PK | Surrogate key. |
| order_id | INTEGER → sales_order | Source order. |
| customer_id | INTEGER → customer | Billed customer. |
| invoice_date | DATE | Issued date. |
| due_date | DATE | Payment due — aging via DateDiff (R01, R05). |
| currency_code | CHAR(3) → currency | Invoice currency. |
| amount_net | NUMERIC(14,2) | Net amount. |
| tax_amount | NUMERIC(14,2) | Tax. |
| amount_gross | NUMERIC(14,2) | Gross total. |
| status_id | INTEGER → payment_status | OPEN / PART / PAID / OVERDUE. |

### `invoice_line`  · ~5k → ~1M
| Field | Type | Description |
| ----- | ---- | ----------- |
| invoice_line_id | INTEGER PK | Surrogate key. |
| invoice_id | INTEGER → invoice | Parent invoice. |
| order_line_id | INTEGER → order_line | Billed line. |
| amount | NUMERIC(14,2) | Line amount. |

### `payment`  · ~1.5k → ~250k
| Field | Type | Description |
| ----- | ---- | ----------- |
| payment_id | INTEGER PK | Surrogate key. |
| invoice_id | INTEGER → invoice | Paid invoice. |
| payment_date | DATE | Received date. |
| amount | NUMERIC(14,2) | Amount (may be partial). |
| method | VARCHAR(16) | WIRE / CARD / CHECK / ACH. |
| currency_code | CHAR(3) → currency | Payment currency. |

### `shipment`  · ~1k → ~200k
| Field | Type | Description |
| ----- | ---- | ----------- |
| shipment_id | INTEGER PK | Surrogate key. |
| order_id | INTEGER → sales_order | Fulfilled order. |
| carrier_id | INTEGER → carrier | Lead carrier. |
| origin_facility_id | INTEGER → facility | Origin (aliased join). |
| dest_facility_id | INTEGER → facility | Destination (aliased join). |
| mode_id | INTEGER → shipment_mode | Primary mode. |
| service_level_id | INTEGER → service_level | Service tier. |
| status_id | INTEGER → shipment_status | Status. |
| booked_date | DATE | Booking date. |
| planned_pickup | DATE | Scheduled pickup. |
| actual_pickup | DATE | Real pickup. |
| planned_delivery | DATE | Scheduled delivery. |
| actual_delivery | DATE *(null)* | Real delivery (NULL if in transit — IsNull/HasValue). |
| weight_kg | NUMERIC(12,2) | Gross weight (log-normal) — scatter/bubble (R10). |
| volume_m3 | NUMERIC(12,3) | Volume — bubble size (R10). |
| chargeable_weight | NUMERIC(12,2) | Billable weight. |
| freight_cost | NUMERIC(14,2) | Cost (log-normal) — scatter (R10). |
| currency_code | CHAR(3) → currency | Cost currency. |
| distance_km | NUMERIC(10,2) | Route distance. |

Transit time = `actual_delivery − actual_pickup` (right-skewed) → histogram (R10).

### `shipment_leg`  · ~2.5k → ~500k
| Field | Type | Description |
| ----- | ---- | ----------- |
| leg_id | INTEGER PK | Surrogate key. |
| shipment_id | INTEGER → shipment | Parent shipment. |
| sequence | INTEGER | Leg order within shipment. |
| from_facility_id | INTEGER → facility | Leg origin. |
| to_facility_id | INTEGER → facility | Leg destination. |
| mode_id | INTEGER → shipment_mode | Leg mode (multi-modal). |
| carrier_id | INTEGER → carrier | Leg carrier. |
| vehicle_id | INTEGER → vehicle | Assigned vehicle. |
| planned_depart | DATE | Scheduled departure. |
| actual_depart | DATE | Real departure. |
| planned_arrive | DATE | Scheduled arrival. |
| actual_arrive | DATE | Real arrival. |
| distance_km | NUMERIC(10,2) | Leg distance. |
| leg_cost | NUMERIC(14,2) | Leg cost. |

### `tracking_event`  · ~10k → ~1M  ⚑ largest table (perf driver)
| Field | Type | Description |
| ----- | ---- | ----------- |
| event_id | BIGINT PK | Surrogate key. |
| shipment_id | INTEGER → shipment | Parent shipment. |
| leg_id | INTEGER → shipment_leg | Owning leg. |
| event_time | TIMESTAMP | When scanned (the one true TIMESTAMP column). |
| event_type | VARCHAR(30) | BOOKED/PICKED/DEPARTED/IN_TRANSIT/CUSTOMS/ARRIVED/DELIVERED/EXCEPTION. |
| facility_id | INTEGER → facility | Where (facility). |
| city_id | INTEGER → city | Where (city). |
| temperature_c | NUMERIC(5,2) *(null)* | Reefer temperature (bounded Normal; NULL for non-reefer — IsNull/HasValue). |
| notes | VARCHAR(200) | Free text — can-grow. |

Count of events per shipment is Poisson (~8–12) — this is what scales to ~1M at the large tier.

### `shipment_charge`  · ~3k → ~600k
| Field | Type | Description |
| ----- | ---- | ----------- |
| charge_id | INTEGER PK | Surrogate key. |
| shipment_id | INTEGER → shipment | Parent shipment. |
| charge_type_id | INTEGER → charge_type | Charge kind (stacked-bar series). |
| amount | NUMERIC(14,2) | Charge amount (fuel spikes mid-year). |
| currency_code | CHAR(3) → currency | Charge currency. |

---

## 4. Market, projects & KPI

### `fuel_price`  (OHLC time series → Stock chart, R13)
| Field | Type | Description |
| ----- | ---- | ----------- |
| fuel_id | INTEGER → fuel_type | Fuel. |
| price_date | DATE | Trading day. |
| open | NUMERIC(10,4) | Opening price. |
| high | NUMERIC(10,4) | Session high (≥ open, close). |
| low | NUMERIC(10,4) | Session low (≤ open, close). |
| close | NUMERIC(10,4) | Closing price. |
| volume | BIGINT | Traded volume. |
| — | PK (fuel_id, price_date) | Composite key; autocorrelated walk + mid-year spike. |

### `exchange_rate`  (daily FX)
| Field | Type | Description |
| ----- | ---- | ----------- |
| currency_code | CHAR(3) → currency | Currency. |
| rate_date | DATE | Rate day. |
| rate_to_usd | NUMERIC(14,8) | Units per USD (random walk). |
| — | PK (currency_code, rate_date) | Composite key. |

### `project`  (capital projects → Gantt parent, R17)
| Field | Type | Description |
| ----- | ---- | ----------- |
| project_id | INTEGER PK | Surrogate key. |
| name | VARCHAR(120) | Project name (e.g. "Rotterdam DC Expansion"). |
| facility_id | INTEGER → facility | Target facility. |
| region_id | INTEGER → region | Region. |
| project_manager_id | INTEGER → employee | PM. |
| start_date | DATE | Kickoff. |
| planned_end_date | DATE | Target completion. |
| actual_end_date | DATE *(null)* | Real completion (NULL if in progress). |
| budget | NUMERIC(16,2) | Budget. |
| currency_code | VARCHAR(3) → currency | Budget currency. |
| status_id | INTEGER | Status code (1=PLANNED, 2=ACTIVE, 3=DONE, 4=ON_HOLD; a bare integer, no lookup table). |

### `project_task`  (Gantt bars + dependencies, R17)
| Field | Type | Description |
| ----- | ---- | ----------- |
| task_id | INTEGER PK | Surrogate key. |
| project_id | INTEGER → project | Parent project. |
| name | VARCHAR(120) | Task name. |
| start_date | DATE | Task start (Gantt bar start). |
| end_date | DATE | Task end (Gantt bar end). |
| pct_complete | NUMERIC(5,2) | Progress 0–100. |
| predecessor_task_id | INTEGER → project_task *(null)* | Dependency. |
| assigned_to | INTEGER → employee | Owner. |
| phase | VARCHAR(48) | Phase grouping. |

### `carrier_scorecard`  (Radar + Gauge, R09)
| Field | Type | Description |
| ----- | ---- | ----------- |
| scorecard_id | INTEGER PK | Surrogate key. |
| carrier_id | INTEGER → carrier | Graded carrier. |
| period_month | DATE | Month (first-of-month). |
| on_time_pct | NUMERIC(5,2) | On-time delivery % (bounded Normal; one outlier carrier low). |
| damage_rate | NUMERIC(5,3) | Damage rate. |
| cost_index | NUMERIC(6,2) | Relative cost index. |
| capacity_utilization | NUMERIC(5,2) | Utilization %. |
| claims_count | INTEGER | Claims in the month. |

---

## 5. How the schema feeds the reports

In brief: the geo + product + org hierarchies drive deep grouping
and crosstabs; the order→shipment→tracking chain plus the deep FK graph drive the 30-table / 50-join performance report
(R07) and the ~1M-row volume; and the market/KPI tables give every non-trivial chart type a natural data source
(`fuel_price`→Stock, `carrier_scorecard`→Radar/Gauge, `project_task`→Gantt, `shipment` weight×cost×volume→Scatter/Bubble,
transit-time→Histogram).
