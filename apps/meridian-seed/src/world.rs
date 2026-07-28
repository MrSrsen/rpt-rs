//! The generation state and orchestration.
//!
//! A single [`World`] threads one deterministic RNG stream through every table
//! in a fixed order, recording just enough per-row index information to satisfy
//! foreign keys downstream. Reference tables are effectively hand-authored (a
//! 1:1 projection of [`crate::pools`]); master, fact and market tables are
//! generated. `build` returns the tables in FK-safe emit order.

use crate::calendar::ymd;
use crate::png::solid_png;
use crate::pools;
use crate::rng::Rng;
use crate::sql::{col, fk, fk_nul, fk_text, Table, Ty, Val};
use std::collections::BTreeMap;

/// The single fixed PRNG seed. Changing it re-rolls the entire corpus.
pub(crate) const SEED: u64 = 0x4d_45_52_49_44_49_41_4e; // "MERIDIAN"

/// Requested corpus size.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Tier {
    /// Committed CI tier (`tracking_event` ≈ 10k).
    Small,
    /// Perf tier (`tracking_event` ≈ 1M); not committed.
    Large,
}

impl Tier {
    /// Parse the `--tier` argument.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "small" => Some(Self::Small),
            "large" => Some(Self::Large),
            _ => None,
        }
    }

    /// The count multiplier applied to master/fact entity counts.
    fn factor(self) -> usize {
        match self {
            Tier::Small => 1,
            Tier::Large => 100,
        }
    }
}

/// Resolved per-table target counts for a tier.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sizing {
    pub(crate) facilities: usize,
    pub(crate) carriers: usize,
    pub(crate) vehicles: usize,
    pub(crate) suppliers: usize,
    pub(crate) products: usize,
    pub(crate) employees: usize,
    pub(crate) customers: usize,
    pub(crate) orders: usize,
    pub(crate) shipments: usize,
}

impl Sizing {
    /// Counts for a tier (base × [`Tier::factor`] on the scaled tables).
    pub(crate) fn for_tier(tier: Tier) -> Self {
        let f = tier.factor();
        Self {
            facilities: 120 * f,
            carriers: 40 * f,
            vehicles: 200 * f,
            suppliers: 120 * f,
            products: 800 * f,
            employees: 200 * f,
            customers: 400 * f,
            orders: 1500 * f,
            shipments: 1000 * f,
        }
    }
}

// --- per-row index records (only the fields downstream FKs need) ------------

#[derive(Debug, Clone, Copy)]
pub(crate) struct CountryIx {
    pub(crate) id: i64,
    pub(crate) region_id: i64,
    pub(crate) ccy: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CityIx {
    pub(crate) id: i64,
    pub(crate) region_id: i64,
    pub(crate) ccy: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FacilityIx {
    pub(crate) id: i64,
    pub(crate) city_id: i64,
    pub(crate) region_id: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CarrierIx {
    pub(crate) id: i64,
    pub(crate) mode_id: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProductIx {
    pub(crate) id: i64,
    pub(crate) price_cents: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CustomerIx {
    pub(crate) id: i64,
    pub(crate) city_id: i64,
    pub(crate) ccy: &'static str,
    pub(crate) sales_rep_id: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OrderIx {
    pub(crate) id: i64,
    pub(crate) customer_id: i64,
    pub(crate) ccy: &'static str,
    pub(crate) order_day: i32,
    pub(crate) status_id: i64,
    pub(crate) total_cents: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OrderLineIx {
    pub(crate) id: i64,
    pub(crate) amount_cents: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct InvoiceIx {
    pub(crate) id: i64,
    pub(crate) ccy: &'static str,
    pub(crate) gross_cents: i64,
    pub(crate) invoice_day: i32,
    pub(crate) status_id: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ShipmentIx {
    pub(crate) id: i64,
    pub(crate) carrier_id: i64,
    pub(crate) origin_facility: i64,
    pub(crate) dest_facility: i64,
    pub(crate) ccy: &'static str,
    pub(crate) pickup_day: i32,
    /// Upper bound of the shipment window: `actual_delivery` if delivered, else
    /// `planned_delivery`. Legs and tracking events stay within `[pickup, end]`.
    pub(crate) end_day: i32,
    /// Whether the shipment has an `actual_delivery` (drives the terminal event).
    pub(crate) delivered: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LegIx {
    pub(crate) id: i64,
    pub(crate) from_facility: i64,
    pub(crate) to_facility: i64,
    pub(crate) arrive_day: i32,
}

/// Generation state.
#[derive(Debug)]
pub(crate) struct World {
    pub(crate) rng: Rng,
    pub(crate) sizing: Sizing,
    pub(crate) blobs: BTreeMap<&'static str, Vec<u8>>,

    pub(crate) currencies: Vec<&'static str>,
    pub(crate) countries: Vec<CountryIx>,
    pub(crate) cities: Vec<CityIx>,
    pub(crate) cities_by_region: BTreeMap<i64, Vec<CityIx>>,
    pub(crate) leaf_categories: Vec<i64>,
    pub(crate) uoms: Vec<i64>,

    pub(crate) facilities: Vec<FacilityIx>,
    pub(crate) carriers: Vec<CarrierIx>,
    pub(crate) vehicles_by_carrier: BTreeMap<i64, Vec<i64>>,
    pub(crate) suppliers: Vec<i64>,
    pub(crate) products: Vec<ProductIx>,
    pub(crate) departments: Vec<i64>,
    pub(crate) sales_reps: Vec<i64>,
    pub(crate) project_managers: Vec<i64>,
    pub(crate) customers: Vec<CustomerIx>,
    /// Cumulative Pareto spend weights over `customers`, for weighted sampling.
    pub(crate) customer_cum: Vec<f64>,

    pub(crate) orders: Vec<OrderIx>,
    pub(crate) order_lines_by_order: BTreeMap<i64, Vec<OrderLineIx>>,
    pub(crate) invoices: Vec<InvoiceIx>,
    pub(crate) shipments: Vec<ShipmentIx>,
    pub(crate) legs_by_shipment: BTreeMap<i64, Vec<LegIx>>,

    /// Scratch: provinces awaiting city generation, as `(province_id, country_id)`.
    pub(crate) pending_provinces: Vec<(i64, i64)>,
}

impl World {
    /// Build the whole corpus for a tier, returning tables in FK-safe order.
    pub(crate) fn build(tier: Tier) -> Vec<Table> {
        let mut blobs = BTreeMap::new();
        for (kind, color) in pools::BLOB_COLORS {
            blobs.insert(*kind, solid_png(4, *color));
        }
        let mut w = World {
            rng: Rng::from_seed(SEED),
            sizing: Sizing::for_tier(tier),
            blobs,
            currencies: Vec::new(),
            countries: Vec::new(),
            cities: Vec::new(),
            cities_by_region: BTreeMap::new(),
            leaf_categories: Vec::new(),
            uoms: Vec::new(),
            facilities: Vec::new(),
            carriers: Vec::new(),
            vehicles_by_carrier: BTreeMap::new(),
            suppliers: Vec::new(),
            products: Vec::new(),
            departments: Vec::new(),
            sales_reps: Vec::new(),
            project_managers: Vec::new(),
            customers: Vec::new(),
            customer_cum: Vec::new(),
            orders: Vec::new(),
            order_lines_by_order: BTreeMap::new(),
            invoices: Vec::new(),
            shipments: Vec::new(),
            legs_by_shipment: BTreeMap::new(),
            pending_provinces: Vec::new(),
        };

        // Reference / lookup, then master — each generator runs in this exact
        // order (the RNG stream depends on it).
        let mut tables = vec![
            w.gen_region(),
            w.gen_currency(),
            w.gen_country(),
            w.gen_province(),
            w.gen_city(),
            w.gen_industry(),
            w.gen_product_category(),
            w.gen_unit_of_measure(),
            w.gen_incoterm(),
            w.gen_shipment_mode(),
            w.gen_service_level(),
            w.gen_status_table("order_status", pools::ORDER_STATUSES),
            w.gen_status_table("shipment_status", pools::SHIPMENT_STATUSES),
            w.gen_status_table("payment_status", pools::PAYMENT_STATUSES),
            w.gen_fuel_type(),
            w.gen_charge_type(),
            w.gen_department(),
            w.gen_employee(),
            w.gen_facility(),
            w.gen_carrier(),
            w.gen_vehicle(),
            w.gen_supplier(),
            w.gen_product(),
            w.gen_customer(),
            w.gen_customer_contact(),
        ];

        // Fact.
        let (sales_order, order_line) = w.gen_orders();
        tables.push(sales_order);
        tables.push(order_line);
        let (invoice, invoice_line) = w.gen_invoices();
        tables.push(invoice);
        tables.push(invoice_line);
        tables.push(w.gen_payment());
        tables.push(w.gen_shipment());
        tables.push(w.gen_shipment_leg());
        tables.push(w.gen_tracking_event());
        tables.push(w.gen_shipment_charge());

        // Market / projects / KPI.
        tables.push(w.gen_fuel_price());
        tables.push(w.gen_exchange_rate());
        let (project, project_task) = w.gen_projects();
        tables.push(project);
        tables.push(project_task);
        tables.push(w.gen_carrier_scorecard());

        tables
    }

    // --- shared value helpers ----------------------------------------------

    /// Right-skewed money as integer cents: a log-normal amount around `median`.
    pub(crate) fn money_skewed(&mut self, median: f64, sigma: f64) -> i64 {
        (self.rng.lognormal(median, sigma).max(0.01) * 100.0).round() as i64
    }

    /// A right-skewed `NUMERIC` value: a log-normal around `median` at `scale`.
    pub(crate) fn dec_skewed(&mut self, median: f64, sigma: f64, scale: u8) -> Val {
        let mul = 10f64.powi(i32::from(scale));
        Val::Dec(
            (self.rng.lognormal(median, sigma).max(0.0) * mul).round() as i64,
            scale,
        )
    }

    /// A `NUMERIC` value drawn from a normal clamped to `[lo, hi]`.
    pub(crate) fn dec_normal(&mut self, mean: f64, sd: f64, lo: f64, hi: f64, scale: u8) -> Val {
        let mul = 10f64.powi(i32::from(scale));
        Val::Dec(
            (self.rng.bounded_normal(mean, sd, lo, hi) * mul).round() as i64,
            scale,
        )
    }

    /// A `NUMERIC` value: a real in `[lo, hi)` rounded to `scale` decimals.
    pub(crate) fn dec(&mut self, lo: f64, hi: f64, scale: u8) -> Val {
        let mul = 10f64.powi(i32::from(scale));
        Val::Dec((self.rng.real(lo, hi) * mul).round() as i64, scale)
    }

    /// A random second-of-day for timestamps.
    pub(crate) fn secs(&mut self) -> u32 {
        self.rng.int(0, 86_399) as u32
    }

    /// A synthetic person name: `(first, last)`.
    pub(crate) fn person(&mut self) -> (&'static str, &'static str) {
        (
            *self.rng.pick(pools::FIRST_NAMES),
            *self.rng.pick(pools::LAST_NAMES),
        )
    }

    /// A blob for an entity kind (all entities of a kind share one swatch).
    pub(crate) fn blob(&self, kind: &str) -> Val {
        Val::Blob(self.blobs[kind].clone())
    }

    /// A city drawn with a realistic regional skew (weighted categorical over
    /// regions, then uniform within the region).
    pub(crate) fn weighted_city(&mut self) -> CityIx {
        // AMER, EMEA, APAC, LATAM, MEAF.
        const REGION_WEIGHTS: [u32; 5] = [28, 30, 25, 9, 8];
        let region = self.rng.weighted(&REGION_WEIGHTS) as i64 + 1;
        match self.cities_by_region.get(&region) {
            Some(cs) if !cs.is_empty() => *self.rng.pick(&cs.clone()),
            _ => *self.rng.pick(&self.cities.clone()),
        }
    }

    /// A customer drawn in proportion to its Pareto spend weight, so a few
    /// strategic accounts dominate order volume (an 80/20 revenue curve).
    pub(crate) fn weighted_customer(&mut self) -> CustomerIx {
        let total = self.customer_cum.last().copied().unwrap_or(0.0);
        if total <= 0.0 {
            return *self.rng.pick(&self.customers.clone());
        }
        let r = self.rng.real(0.0, total);
        let idx = self.customer_cum.partition_point(|&c| c < r);
        self.customers[idx.min(self.customers.len() - 1)]
    }

    /// An order/shipment date weighted toward Q4 (the peak-season surge).
    pub(crate) fn business_day(&mut self) -> i32 {
        const MONTH_WEIGHT: [u32; 12] = [6, 6, 8, 8, 9, 9, 8, 8, 10, 14, 16, 14];
        let year = 2021 + self.rng.int(0, 2) as i32;
        let total: u32 = MONTH_WEIGHT.iter().sum();
        let mut pick = self.rng.int(0, i64::from(total) - 1) as u32;
        let mut month = 1u32;
        for (i, wt) in MONTH_WEIGHT.iter().enumerate() {
            if pick < *wt {
                month = i as u32 + 1;
                break;
            }
            pick -= *wt;
        }
        let day = self.rng.int(1, 28) as u32;
        ymd(year, month, day)
    }

    // --- reference tables --------------------------------------------------

    fn gen_region(&mut self) -> Table {
        let mut t = Table::new(
            "region",
            vec![
                col("region_id", Ty::Int),
                col("code", Ty::Varchar(8)),
                col("name", Ty::Varchar(40)),
            ],
        );
        for (i, (code, name)) in pools::REGIONS.iter().enumerate() {
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Text((*code).into()),
                Val::Text((*name).into()),
            ]);
        }
        t
    }

    fn gen_currency(&mut self) -> Table {
        let mut t = Table::new(
            "currency",
            vec![
                col("currency_code", Ty::Varchar(3)),
                col("name", Ty::Varchar(40)),
                col("symbol", Ty::Varchar(8)),
                col("decimal_places", Ty::Int),
            ],
        );
        for (code, name, symbol, dp) in pools::CURRENCIES {
            self.currencies.push(code);
            t.push(vec![
                Val::Text((*code).into()),
                Val::Text((*name).into()),
                Val::Text((*symbol).into()),
                Val::Int(i64::from(*dp)),
            ]);
        }
        t
    }

    fn gen_country(&mut self) -> Table {
        let mut t = Table::new(
            "country",
            vec![
                col("country_id", Ty::Int),
                fk("region_id", "region", "region_id"),
                col("iso2", Ty::Varchar(2)),
                col("iso3", Ty::Varchar(3)),
                col("name", Ty::Varchar(60)),
                fk_text("currency_code", "currency", "currency_code", 3),
            ],
        );
        let region_id = |code: &str| {
            pools::REGIONS
                .iter()
                .position(|(c, _)| *c == code)
                .map(|i| i as i64 + 1)
                .unwrap_or(1)
        };
        for (i, (iso2, iso3, name, region, ccy)) in pools::COUNTRIES.iter().enumerate() {
            let rid = region_id(region);
            self.countries.push(CountryIx {
                id: i as i64 + 1,
                region_id: rid,
                ccy,
            });
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Int(rid),
                Val::Text((*iso2).into()),
                Val::Text((*iso3).into()),
                Val::Text((*name).into()),
                Val::Text((*ccy).into()),
            ]);
        }
        t
    }

    fn gen_province(&mut self) -> Table {
        let mut t = Table::new(
            "province",
            vec![
                col("province_id", Ty::Int),
                fk("country_id", "country", "country_id"),
                col("code", Ty::Varchar(6)),
                col("name", Ty::Varchar(60)),
            ],
        );
        let mut pid = 0i64;
        let countries = self.countries.clone();
        for c in &countries {
            let n = self.rng.int(4, 7);
            for _ in 0..n {
                pid += 1;
                let adj = *self.rng.pick(pools::PROVINCE_ADJ);
                let noun = *self.rng.pick(pools::PROVINCE_NOUN);
                let code = format!("P{pid:03}");
                self.provinces_push(pid, c.id);
                t.push(vec![
                    Val::Int(pid),
                    Val::Int(c.id),
                    Val::Text(code),
                    Val::Text(format!("{adj} {noun}")),
                ]);
            }
        }
        t
    }

    // Provinces feed city generation; kept in a scratch map on the world.
    fn provinces_push(&mut self, province_id: i64, country_id: i64) {
        self.pending_provinces.push((province_id, country_id));
    }

    fn gen_city(&mut self) -> Table {
        let mut t = Table::new(
            "city",
            vec![
                col("city_id", Ty::Int),
                fk("province_id", "province", "province_id"),
                col("name", Ty::Varchar(60)),
                col("latitude", Ty::Numeric(9, 6)),
                col("longitude", Ty::Numeric(9, 6)),
                col("population", Ty::Int),
            ],
        );
        let mut cid = 0i64;
        let provinces = std::mem::take(&mut self.pending_provinces);
        for (province_id, country_id) in provinces {
            let region_id = self
                .countries
                .iter()
                .find(|c| c.id == country_id)
                .map(|c| c.region_id)
                .unwrap_or(1);
            let ccy = self
                .countries
                .iter()
                .find(|c| c.id == country_id)
                .map(|c| c.ccy)
                .unwrap_or("USD");
            let (clat, clon) = region_center(region_id);
            let n = self.rng.int(3, 7);
            for _ in 0..n {
                cid += 1;
                let head = *self.rng.pick(pools::CITY_HEADS);
                let body = *self.rng.pick(pools::CITY_BODIES);
                let lat = self.dec(clat - 6.0, clat + 6.0, 6);
                let lon = self.dec(clon - 8.0, clon + 8.0, 6);
                let pop = self.rng.int(15_000, 4_000_000);
                let city = CityIx {
                    id: cid,
                    region_id,
                    ccy,
                };
                self.cities.push(city);
                self.cities_by_region
                    .entry(region_id)
                    .or_default()
                    .push(city);
                t.push(vec![
                    Val::Int(cid),
                    Val::Int(province_id),
                    Val::Text(format!("{head}{body}")),
                    lat,
                    lon,
                    Val::Int(pop),
                ]);
            }
        }
        t
    }

    fn gen_industry(&mut self) -> Table {
        let mut t = Table::new(
            "industry",
            vec![col("industry_id", Ty::Int), col("name", Ty::Varchar(60))],
        );
        for (i, name) in pools::INDUSTRIES.iter().enumerate() {
            t.push(vec![Val::Int(i as i64 + 1), Val::Text((*name).into())]);
        }
        t
    }

    fn gen_product_category(&mut self) -> Table {
        let mut t = Table::new(
            "product_category",
            vec![
                col("category_id", Ty::Int),
                fk_nul("parent_id", "product_category", "category_id"),
                col("name", Ty::Varchar(60)),
                col("depth", Ty::Int),
            ],
        );
        let mut id = 0i64;
        // Depth 1: divisions.
        let mut division_ids = Vec::new();
        for name in pools::CATEGORY_DIVISIONS {
            id += 1;
            division_ids.push(id);
            t.push(vec![
                Val::Int(id),
                Val::Null,
                Val::Text((*name).into()),
                Val::Int(1),
            ]);
        }
        // Depth 2: groups under divisions.
        let mut group_ids = Vec::new();
        for &div in &division_ids {
            let n = self.rng.int(2, 3);
            for _ in 0..n {
                id += 1;
                group_ids.push(id);
                let g = *self.rng.pick(pools::CATEGORY_GROUPS);
                t.push(vec![
                    Val::Int(id),
                    Val::Int(div),
                    Val::Text(g.into()),
                    Val::Int(2),
                ]);
            }
        }
        // Depth 3: classes under groups (the leaves products bind to).
        for &grp in &group_ids {
            let n = self.rng.int(1, 3);
            for _ in 0..n {
                id += 1;
                self.leaf_categories.push(id);
                let q = *self.rng.pick(pools::PRODUCT_QUALIFIERS);
                let noun = *self.rng.pick(pools::PRODUCT_NOUNS);
                t.push(vec![
                    Val::Int(id),
                    Val::Int(grp),
                    Val::Text(format!("{q} {noun}s")),
                    Val::Int(3),
                ]);
            }
        }
        t
    }

    fn gen_unit_of_measure(&mut self) -> Table {
        let mut t = Table::new(
            "unit_of_measure",
            vec![
                col("uom_id", Ty::Int),
                col("code", Ty::Varchar(6)),
                col("name", Ty::Varchar(40)),
            ],
        );
        for (i, (code, name)) in pools::UNITS.iter().enumerate() {
            self.uoms.push(i as i64 + 1);
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Text((*code).into()),
                Val::Text((*name).into()),
            ]);
        }
        t
    }

    fn gen_incoterm(&mut self) -> Table {
        let mut t = Table::new(
            "incoterm",
            vec![
                col("incoterm_id", Ty::Int),
                col("code", Ty::Varchar(4)),
                col("name", Ty::Varchar(40)),
            ],
        );
        for (i, (code, name)) in pools::INCOTERMS.iter().enumerate() {
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Text((*code).into()),
                Val::Text((*name).into()),
            ]);
        }
        t
    }

    fn gen_shipment_mode(&mut self) -> Table {
        let mut t = Table::new(
            "shipment_mode",
            vec![
                col("mode_id", Ty::Int),
                col("code", Ty::Varchar(6)),
                col("name", Ty::Varchar(20)),
            ],
        );
        for (i, (code, name)) in pools::MODES.iter().enumerate() {
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Text((*code).into()),
                Val::Text((*name).into()),
            ]);
        }
        t
    }

    fn gen_service_level(&mut self) -> Table {
        let mut t = Table::new(
            "service_level",
            vec![
                col("level_id", Ty::Int),
                col("code", Ty::Varchar(6)),
                col("name", Ty::Varchar(20)),
                col("target_transit_days", Ty::Int),
            ],
        );
        for (i, (code, name, days)) in pools::SERVICE_LEVELS.iter().enumerate() {
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Text((*code).into()),
                Val::Text((*name).into()),
                Val::Int(*days),
            ]);
        }
        t
    }

    fn gen_status_table(&mut self, name: &'static str, rows: &[(&str, &str, i64)]) -> Table {
        let mut t = Table::new(
            name,
            vec![
                col("id", Ty::Int),
                col("code", Ty::Varchar(10)),
                col("name", Ty::Varchar(30)),
                col("sort_order", Ty::Int),
            ],
        );
        for (i, (code, label, sort)) in rows.iter().enumerate() {
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Text((*code).into()),
                Val::Text((*label).into()),
                Val::Int(*sort),
            ]);
        }
        t
    }

    fn gen_fuel_type(&mut self) -> Table {
        let mut t = Table::new(
            "fuel_type",
            vec![
                col("fuel_id", Ty::Int),
                col("code", Ty::Varchar(6)),
                col("name", Ty::Varchar(30)),
            ],
        );
        for (i, (code, name)) in pools::FUEL_TYPES.iter().enumerate() {
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Text((*code).into()),
                Val::Text((*name).into()),
            ]);
        }
        t
    }

    fn gen_charge_type(&mut self) -> Table {
        let mut t = Table::new(
            "charge_type",
            vec![
                col("charge_id", Ty::Int),
                col("code", Ty::Varchar(6)),
                col("name", Ty::Varchar(30)),
            ],
        );
        for (i, (code, name)) in pools::CHARGE_TYPES.iter().enumerate() {
            t.push(vec![
                Val::Int(i as i64 + 1),
                Val::Text((*code).into()),
                Val::Text((*name).into()),
            ]);
        }
        t
    }
}

/// Approximate `(latitude, longitude)` centre for a region, for city jitter.
fn region_center(region_id: i64) -> (f64, f64) {
    match region_id {
        1 => (39.0, -98.0),  // AMER
        2 => (50.0, 9.0),    // EMEA
        3 => (22.0, 114.0),  // APAC
        4 => (-15.0, -55.0), // LATAM
        5 => (25.0, 45.0),   // MEAF
        _ => (0.0, 0.0),
    }
}
