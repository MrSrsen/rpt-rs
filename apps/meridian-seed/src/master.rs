//! Master-data generators: the low-volume operational entities that fact rows
//! reference. All build on [`World`]'s shared helpers and record index rows for
//! downstream foreign keys.

use crate::calendar::{last_day, ymd};
use crate::pools;
use crate::sql::{col, fk, fk_nul, fk_text, nul, Table, Ty, Val};
use crate::world::{CarrierIx, CustomerIx, FacilityIx, ProductIx, World};

/// The fixed organisation tree: `(name, parent_index_1_based_or_0)`.
const DEPARTMENTS: &[(&str, i64)] = &[
    ("Meridian Global Logistics", 0),
    ("Meridian Freight", 1),
    ("Meridian Trade", 1),
    ("Meridian Capital Projects", 1),
    ("Finance", 1),
    ("Human Resources", 1),
    ("Information Technology", 1),
    ("Ocean & Air Operations", 2),
    ("Road & Rail Operations", 2),
    ("Fleet Management", 2),
    ("Sales", 3),
    ("Procurement", 3),
    ("Customer Service", 3),
    ("Project Delivery", 4),
    ("Accounts Receivable", 5),
];

const DEPT_SALES: i64 = 11;
const DEPT_PROJECT: i64 = 14;

impl World {
    /// Uppercase ASCII letters of length `n`.
    fn letters(&mut self, n: usize) -> String {
        (0..n)
            .map(|_| (b'A' + self.rng.int(0, 25) as u8) as char)
            .collect()
    }

    /// A synthetic company name from the given head/tail pools.
    fn company(&mut self, heads: &[&'static str], tails: &[&'static str]) -> String {
        format!("{} {}", self.rng.pick(heads), self.rng.pick(tails))
    }

    /// A country row at random (used to co-locate region + currency).
    fn any_country(&mut self) -> crate::world::CountryIx {
        *self.rng.pick(&self.countries.clone())
    }

    pub(crate) fn gen_department(&mut self) -> Table {
        let mut t = Table::new(
            "department",
            vec![
                col("department_id", Ty::Int),
                fk_nul("parent_id", "department", "department_id"),
                col("name", Ty::Varchar(60)),
            ],
        );
        for (i, (name, parent)) in DEPARTMENTS.iter().enumerate() {
            let id = i as i64 + 1;
            self.departments.push(id);
            t.push(vec![
                Val::Int(id),
                if *parent == 0 {
                    Val::Null
                } else {
                    Val::Int(*parent)
                },
                Val::Text((*name).into()),
            ]);
        }
        t
    }

    pub(crate) fn gen_employee(&mut self) -> Table {
        let mut t = Table::new(
            "employee",
            vec![
                col("employee_id", Ty::Int),
                fk("department_id", "department", "department_id"),
                fk_nul("manager_id", "employee", "employee_id"),
                col("first_name", Ty::Varchar(30)),
                col("last_name", Ty::Varchar(30)),
                col("title", Ty::Varchar(40)),
                col("hire_date", Ty::Date),
                col("salary", Ty::Numeric(12, 2)),
                fk_text("currency_code", "currency", "currency_code", 3),
                fk("region_id", "region", "region_id"),
                nul("photo", Ty::Blob),
                col("is_active", Ty::Bool),
            ],
        );
        let count = self.sizing.employees;
        for i in 0..count {
            let id = i as i64 + 1;
            // Roughly a third are sales reps, a smaller share project managers.
            let (dept, title) = if self.rng.chance(0.30) {
                self.sales_reps.push(id);
                (
                    DEPT_SALES,
                    if self.rng.chance(0.5) {
                        "Account Executive"
                    } else {
                        "Sales Representative"
                    },
                )
            } else if self.rng.chance(0.12) {
                self.project_managers.push(id);
                (DEPT_PROJECT, "Project Manager")
            } else {
                (
                    *self.rng.pick(&self.departments.clone()),
                    *self.rng.pick(pools::TITLES),
                )
            };
            let country = self.any_country();
            // Manager is any earlier employee (self-ref FK points to a lower id).
            let manager = if id > 1 && self.rng.chance(0.85) {
                Val::Int(self.rng.int(1, id - 1))
            } else {
                Val::Null
            };
            let (first, last) = self.person();
            // Hire before the transactional window opens (2021-01-01) so an
            // employee always predates any order/project they touch.
            let hire =
                self.rng
                    .int(ymd(2010, 1, 1) as i64, ymd(2020, 12, 31) as i64) as i32;
            let salary = self.dec(32_000.0, 185_000.0, 2);
            let photo = if self.rng.chance(0.8) {
                self.blob("employee")
            } else {
                Val::Null
            };
            t.push(vec![
                Val::Int(id),
                Val::Int(dept),
                manager,
                Val::Text(first.into()),
                Val::Text(last.into()),
                Val::Text(title.into()),
                Val::Date(hire),
                salary,
                Val::Text(country.ccy.into()),
                Val::Int(country.region_id),
                photo,
                Val::Bool(self.rng.chance(0.92)),
            ]);
        }
        // Guarantee at least one sales rep and project manager exist.
        if self.sales_reps.is_empty() {
            self.sales_reps.push(1);
        }
        if self.project_managers.is_empty() {
            self.project_managers.push(1);
        }
        t
    }

    pub(crate) fn gen_facility(&mut self) -> Table {
        let mut t = Table::new(
            "facility",
            vec![
                col("facility_id", Ty::Int),
                fk("city_id", "city", "city_id"),
                col("type", Ty::Varchar(20)),
                col("code", Ty::Varchar(10)),
                col("name", Ty::Varchar(60)),
                col("capacity_m3", Ty::Numeric(12, 2)),
                col("opened_date", Ty::Date),
                nul("image", Ty::Blob),
            ],
        );
        for i in 0..self.sizing.facilities {
            let id = i as i64 + 1;
            let city = *self.rng.pick(&self.cities.clone());
            let kind = *self.rng.pick(pools::FACILITY_TYPES);
            let head = *self.rng.pick(pools::COMPANY_HEADS);
            let cap = self.dec(2_000.0, 120_000.0, 2);
            let opened = self.rng.int(ymd(2005, 1, 1) as i64, last_day() as i64) as i32;
            let image = if self.rng.chance(0.7) {
                self.blob("facility")
            } else {
                Val::Null
            };
            self.facilities.push(FacilityIx {
                id,
                city_id: city.id,
                region_id: city.region_id,
            });
            t.push(vec![
                Val::Int(id),
                Val::Int(city.id),
                Val::Text(kind.into()),
                Val::Text(format!("FAC{id:05}")),
                Val::Text(format!("{head} {kind}")),
                cap,
                Val::Date(opened),
                image,
            ]);
        }
        t
    }

    pub(crate) fn gen_carrier(&mut self) -> Table {
        let mut t = Table::new(
            "carrier",
            vec![
                col("carrier_id", Ty::Int),
                col("name", Ty::Varchar(60)),
                fk("primary_mode_id", "shipment_mode", "mode_id"),
                col("scac", Ty::Varchar(4)),
                col("is_own_fleet", Ty::Bool),
                nul("logo", Ty::Blob),
            ],
        );
        // Realistic modal share: Road > Sea > Air > Rail (MODES = Sea/Air/Road/Rail).
        const MODE_WEIGHTS: [u32; 4] = [30, 15, 45, 10];
        for i in 0..self.sizing.carriers {
            let id = i as i64 + 1;
            let mode_id = self.rng.weighted(&MODE_WEIGHTS) as i64 + 1;
            // The first carrier is the narrative under-performer; make it own-fleet
            // so it shows up in the fleet reports as well.
            let own_fleet = i == 0 || self.rng.chance(0.25);
            let name = self.company(pools::CARRIER_HEADS, pools::CARRIER_TAILS);
            let scac = self.letters(4);
            let logo = if self.rng.chance(0.75) {
                self.blob("carrier")
            } else {
                Val::Null
            };
            self.carriers.push(CarrierIx { id, mode_id });
            t.push(vec![
                Val::Int(id),
                Val::Text(name),
                Val::Int(mode_id),
                Val::Text(scac),
                Val::Bool(own_fleet),
                logo,
            ]);
        }
        t
    }

    pub(crate) fn gen_vehicle(&mut self) -> Table {
        let mut t = Table::new(
            "vehicle",
            vec![
                col("vehicle_id", Ty::Int),
                fk("carrier_id", "carrier", "carrier_id"),
                col("type", Ty::Varchar(20)),
                col("registration", Ty::Varchar(12)),
                col("capacity_kg", Ty::Numeric(12, 2)),
                col("capacity_m3", Ty::Numeric(12, 2)),
                col("model_year", Ty::Int),
            ],
        );
        for i in 0..self.sizing.vehicles {
            let id = i as i64 + 1;
            let carrier = *self.rng.pick(&self.carriers.clone());
            let kind = *self.rng.pick(pools::VEHICLE_TYPES);
            let reg = format!("{}-{:04}", self.letters(2), self.rng.int(0, 9999));
            let cap_kg = self.dec(1_000.0, 40_000.0, 2);
            let cap_m3 = self.dec(10.0, 3_000.0, 2);
            let year = self.rng.int(2008, 2023);
            self.vehicles_by_carrier
                .entry(carrier.id)
                .or_default()
                .push(id);
            t.push(vec![
                Val::Int(id),
                Val::Int(carrier.id),
                Val::Text(kind.into()),
                Val::Text(reg),
                cap_kg,
                cap_m3,
                Val::Int(year),
            ]);
        }
        t
    }

    pub(crate) fn gen_supplier(&mut self) -> Table {
        let mut t = Table::new(
            "supplier",
            vec![
                col("supplier_id", Ty::Int),
                fk("city_id", "city", "city_id"),
                col("name", Ty::Varchar(60)),
                col("rating", Ty::Numeric(3, 2)),
                col("since_date", Ty::Date),
                nul("logo", Ty::Blob),
            ],
        );
        for i in 0..self.sizing.suppliers {
            let id = i as i64 + 1;
            let city = *self.rng.pick(&self.cities.clone());
            let name = self.company(pools::COMPANY_HEADS, pools::COMPANY_TAILS);
            let rating = self.dec(1.0, 5.0, 2);
            let since = self.rng.int(ymd(2008, 1, 1) as i64, last_day() as i64) as i32;
            let logo = if self.rng.chance(0.6) {
                self.blob("supplier")
            } else {
                Val::Null
            };
            self.suppliers.push(id);
            t.push(vec![
                Val::Int(id),
                Val::Int(city.id),
                Val::Text(name),
                rating,
                Val::Date(since),
                logo,
            ]);
        }
        t
    }

    pub(crate) fn gen_product(&mut self) -> Table {
        let mut t = Table::new(
            "product",
            vec![
                col("product_id", Ty::Int),
                fk("category_id", "product_category", "category_id"),
                fk("supplier_id", "supplier", "supplier_id"),
                fk("uom_id", "unit_of_measure", "uom_id"),
                col("sku", Ty::Varchar(20)),
                col("name", Ty::Varchar(80)),
                nul("description", Ty::Varchar(400)),
                col("unit_price", Ty::Numeric(12, 2)),
                fk_text("currency_code", "currency", "currency_code", 3),
                col("weight_kg", Ty::Numeric(12, 3)),
                col("volume_m3", Ty::Numeric(12, 4)),
                nul("hs_code", Ty::Varchar(12)),
                col("is_hazardous", Ty::Bool),
                col("is_active", Ty::Bool),
                nul("image", Ty::Blob),
            ],
        );
        for i in 0..self.sizing.products {
            let id = i as i64 + 1;
            let category = *self.rng.pick(&self.leaf_categories.clone());
            let supplier = *self.rng.pick(&self.suppliers.clone());
            let uom = *self.rng.pick(&self.uoms.clone());
            let qual = *self.rng.pick(pools::PRODUCT_QUALIFIERS);
            let noun = *self.rng.pick(pools::PRODUCT_NOUNS);
            let ccy = *self.rng.pick(&self.currencies.clone());
            // Prices are right-skewed: many cheap parts, a long tail of costly ones.
            let price = self.money_skewed(60.0, 1.0).clamp(200, 5_000_000);
            let weight = self.dec_skewed(6.0, 1.1, 3);
            let volume = self.dec_skewed(0.05, 1.0, 4);
            let description = if self.rng.chance(0.7) {
                Val::Text(format!(
                    "{qual} {noun} — supplied on {} terms, palletised for freight.",
                    self.rng.pick(pools::INCOTERMS).0
                ))
            } else {
                Val::Null
            };
            let hs_code = if self.rng.chance(0.85) {
                Val::Text(format!(
                    "{:02}{:02}.{:02}",
                    self.rng.int(1, 97),
                    self.rng.int(1, 99),
                    self.rng.int(0, 99)
                ))
            } else {
                Val::Null
            };
            let image = if self.rng.chance(0.65) {
                self.blob("product")
            } else {
                Val::Null
            };
            self.products.push(ProductIx {
                id,
                price_cents: price,
            });
            t.push(vec![
                Val::Int(id),
                Val::Int(category),
                Val::Int(supplier),
                Val::Int(uom),
                Val::Text(format!("SKU-{id:06}")),
                Val::Text(format!("{qual} {noun} {}", 100 + (id % 900))),
                description,
                Val::Dec(price, 2),
                Val::Text(ccy.into()),
                weight,
                volume,
                hs_code,
                Val::Bool(self.rng.chance(0.08)),
                Val::Bool(self.rng.chance(0.9)),
                image,
            ]);
        }
        t
    }

    pub(crate) fn gen_customer(&mut self) -> Table {
        let mut t = Table::new(
            "customer",
            vec![
                col("customer_id", Ty::Int),
                fk("city_id", "city", "city_id"),
                fk("industry_id", "industry", "industry_id"),
                col("name", Ty::Varchar(60)),
                col("account_code", Ty::Varchar(16)),
                col("credit_limit", Ty::Numeric(14, 2)),
                fk_text("currency_code", "currency", "currency_code", 3),
                col("since_date", Ty::Date),
                fk("sales_rep_id", "employee", "employee_id"),
                col("tier", Ty::Varchar(16)),
                col("is_active", Ty::Bool),
                nul("logo", Ty::Blob),
            ],
        );
        let industries = pools::INDUSTRIES.len() as i64;
        let mut running = 0.0f64;
        for i in 0..self.sizing.customers {
            let id = i as i64 + 1;
            let city = self.weighted_city();
            let industry = self.rng.int(1, industries);
            let name = self.company(pools::COMPANY_HEADS, pools::COMPANY_TAILS);
            let rep = *self.rng.pick(&self.sales_reps.clone());
            // A Pareto spend weight: a few accounts dominate. It drives both the
            // customer's tier and how often it is chosen for orders.
            let weight = self.rng.pareto(1.0, 1.08);
            running += weight;
            self.customer_cum.push(running);
            let tier = if weight > 12.0 {
                "Strategic"
            } else if weight > 4.0 {
                "Key"
            } else if weight > 1.7 {
                "Standard"
            } else {
                "Occasional"
            };
            // Credit scales with the spend weight (also right-skewed).
            let credit = Val::Dec(
                ((25_000.0 * weight).min(50_000_000.0) * 100.0).round() as i64,
                2,
            );
            // Onboard before the transactional window opens (2021-01-01) so a
            // customer's since_date always predates its first order.
            let since =
                self.rng
                    .int(ymd(2012, 1, 1) as i64, ymd(2020, 12, 31) as i64) as i32;
            let logo = if self.rng.chance(0.55) {
                self.blob("customer")
            } else {
                Val::Null
            };
            self.customers.push(CustomerIx {
                id,
                city_id: city.id,
                ccy: city.ccy,
                sales_rep_id: rep,
            });
            t.push(vec![
                Val::Int(id),
                Val::Int(city.id),
                Val::Int(industry),
                Val::Text(name),
                Val::Text(format!("CUST-{id:06}")),
                credit,
                Val::Text(city.ccy.into()),
                Val::Date(since),
                Val::Int(rep),
                Val::Text(tier.into()),
                Val::Bool(self.rng.chance(0.9)),
                logo,
            ]);
        }
        t
    }

    pub(crate) fn gen_customer_contact(&mut self) -> Table {
        let mut t = Table::new(
            "customer_contact",
            vec![
                col("contact_id", Ty::Int),
                fk("customer_id", "customer", "customer_id"),
                col("name", Ty::Varchar(60)),
                col("email", Ty::Varchar(80)),
                col("phone", Ty::Varchar(24)),
                col("role", Ty::Varchar(30)),
            ],
        );
        let mut cid = 0i64;
        for c in self.customers.clone() {
            let n = self.rng.int(1, 2);
            for _ in 0..n {
                cid += 1;
                let (first, last) = self.person();
                let role = *self.rng.pick(pools::CONTACT_ROLES);
                let email = format!(
                    "{}.{}@example.com",
                    first.to_ascii_lowercase(),
                    last.to_ascii_lowercase()
                );
                let phone = format!(
                    "+{} {:03}-{:04}",
                    self.rng.int(1, 99),
                    self.rng.int(100, 999),
                    self.rng.int(0, 9999)
                );
                t.push(vec![
                    Val::Int(cid),
                    Val::Int(c.id),
                    Val::Text(format!("{first} {last}")),
                    Val::Text(email),
                    Val::Text(phone),
                    Val::Text(role.into()),
                ]);
            }
        }
        t
    }
}
