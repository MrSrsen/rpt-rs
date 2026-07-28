//! Market, projects and KPI generators: the OHLC / FX time series, the capital
//! projects Gantt data, and the carrier scorecards. These carry the narrative
//! hooks — a mid-2022 fuel-price spike and one under-performing carrier.

use crate::calendar::{last_day, market_window, ymd, MARKET_MONTHS, MARKET_START_YEAR};
use crate::pools;
use crate::sql::{col, fk, fk_nul, fk_text, nul, Table, Ty, Val};
use crate::world::World;

/// Project lifecycle phases (also used as `project_task.phase`).
const PHASES: &[&str] = &[
    "Design",
    "Procurement",
    "Construction",
    "Fit-out",
    "Commissioning",
];

impl World {
    pub(crate) fn gen_fuel_price(&mut self) -> Table {
        let mut t = Table::new(
            "fuel_price",
            vec![
                fk("fuel_id", "fuel_type", "fuel_id"),
                col("price_date", Ty::Date),
                col("open", Ty::Numeric(10, 4)),
                col("high", Ty::Numeric(10, 4)),
                col("low", Ty::Numeric(10, 4)),
                col("close", Ty::Numeric(10, 4)),
                col("volume", Ty::BigInt),
            ],
        )
        .with_pk(vec!["fuel_id", "price_date"]);

        let (start, end) = market_window();
        // Base price per fuel (Diesel, Jet, Bunker, LNG).
        let bases = [1.20f64, 1.55, 0.52, 8.4];
        let spike_lo = ymd(2022, 5, 1);
        let spike_hi = ymd(2022, 9, 15);
        for (fi, base) in bases.iter().enumerate() {
            let fuel_id = fi as i64 + 1;
            let mut close = *base;
            for day in start..=end {
                let open = close;
                // Mean-reverting walk toward the base, plus the mid-2022 spike.
                let revert = (*base - open) * 0.02;
                let spike = if (spike_lo..=spike_hi).contains(&day) {
                    base * 0.010
                } else {
                    0.0
                };
                let drift = self.rng.normal(0.0, base * 0.012) + revert + spike;
                close = (open + drift).max(base * 0.2);
                let hi = open.max(close) * (1.0 + self.rng.real(0.0, 0.02));
                let lo = open.min(close) * (1.0 - self.rng.real(0.0, 0.02));
                let volume = self.rng.int(50_000, 5_000_000);
                t.push(vec![
                    Val::Int(fuel_id),
                    Val::Date(day),
                    dec4(open),
                    dec4(hi),
                    dec4(lo),
                    dec4(close),
                    Val::Int(volume),
                ]);
            }
        }
        t
    }

    pub(crate) fn gen_exchange_rate(&mut self) -> Table {
        let mut t = Table::new(
            "exchange_rate",
            vec![
                fk_text("currency_code", "currency", "currency_code", 3),
                col("rate_date", Ty::Date),
                col("rate_to_usd", Ty::Numeric(14, 6)),
            ],
        )
        .with_pk(vec!["currency_code", "rate_date"]);

        for (code, ..) in pools::CURRENCIES {
            let base = base_rate(code);
            let mut rate = base;
            for m in 0..MARKET_MONTHS {
                let (y, mo) = (MARKET_START_YEAR + m / 12, m % 12 + 1);
                let date = ymd(y, mo as u32, 1);
                if *code == "USD" {
                    rate = 1.0;
                } else {
                    let revert = (base - rate) * 0.05;
                    rate = (rate + self.rng.normal(0.0, base * 0.02) + revert).max(base * 0.5);
                }
                t.push(vec![
                    Val::Text((*code).into()),
                    Val::Date(date),
                    Val::Dec((rate * 1_000_000.0).round() as i64, 6),
                ]);
            }
        }
        t
    }

    pub(crate) fn gen_projects(&mut self) -> (Table, Table) {
        let mut projects = Table::new(
            "project",
            vec![
                col("project_id", Ty::Int),
                col("name", Ty::Varchar(80)),
                fk("facility_id", "facility", "facility_id"),
                fk("region_id", "region", "region_id"),
                fk("project_manager_id", "employee", "employee_id"),
                col("start_date", Ty::Date),
                col("planned_end_date", Ty::Date),
                nul("actual_end_date", Ty::Date),
                col("budget", Ty::Numeric(16, 2)),
                fk_text("currency_code", "currency", "currency_code", 3),
                col("status_id", Ty::Int),
            ],
        );
        let mut tasks = Table::new(
            "project_task",
            vec![
                col("task_id", Ty::Int),
                fk("project_id", "project", "project_id"),
                col("name", Ty::Varchar(80)),
                col("start_date", Ty::Date),
                col("end_date", Ty::Date),
                col("pct_complete", Ty::Numeric(5, 2)),
                fk_nul("predecessor_task_id", "project_task", "task_id"),
                fk("assigned_to", "employee", "employee_id"),
                col("phase", Ty::Varchar(20)),
            ],
        );

        let count = 10 * self.sizing_factor();
        let mut task_id = 0i64;
        for i in 0..count {
            let pid = i as i64 + 1;
            let facility = *self.rng.pick(&self.facilities.clone());
            let pm = *self.rng.pick(&self.project_managers.clone());
            let start =
                self.rng
                    .int(ymd(2021, 1, 1) as i64, ymd(2023, 6, 30) as i64) as i32;
            let duration = self.rng.int(180, 900) as i32;
            let planned_end = start + duration;
            let complete = planned_end < last_day() && self.rng.chance(0.5);
            let actual_end = if complete {
                Val::Date(planned_end + self.rng.int(-30, 60) as i32)
            } else {
                Val::Null
            };
            let status = if complete {
                4
            } else if start > last_day() {
                1
            } else if self.rng.chance(0.2) {
                3
            } else {
                2
            };
            let head = *self.rng.pick(pools::COMPANY_HEADS);
            let kind = *self.rng.pick(pools::FACILITY_TYPES);
            let budget = self.dec(500_000.0, 50_000_000.0, 2);
            let ccy = *self.rng.pick(&self.currencies.clone());
            projects.push(vec![
                Val::Int(pid),
                Val::Text(format!("{head} {kind} Expansion")),
                Val::Int(facility.id),
                Val::Int(facility.region_id),
                Val::Int(pm),
                Val::Date(start),
                Val::Date(planned_end),
                actual_end,
                budget,
                Val::Text(ccy.into()),
                Val::Int(status),
            ]);

            // A short chain of dependency-linked tasks spanning the phases.
            let n = self.rng.int(8, 15);
            let mut cursor = start;
            let mut prev: Option<i64> = None;
            for k in 0..n {
                task_id += 1;
                let phase = PHASES[(k as usize) % PHASES.len()];
                let len = self.rng.int(10, 90) as i32;
                // Tasks are sub-intervals of the project window: clamp both ends
                // into [start, planned_end] so no bar overruns the project.
                let t_start = cursor.min(planned_end);
                let t_end = (t_start + len).min(planned_end);
                cursor = t_end + self.rng.int(0, 10) as i32;
                let pct = if t_end < last_day() {
                    self.dec(60.0, 100.0, 2)
                } else if t_start < last_day() {
                    self.dec(0.0, 80.0, 2)
                } else {
                    Val::Dec(0, 2)
                };
                let assignee = *self.rng.pick(&self.project_managers.clone());
                tasks.push(vec![
                    Val::Int(task_id),
                    Val::Int(pid),
                    Val::Text(format!("{phase} — task {}", k + 1)),
                    Val::Date(t_start),
                    Val::Date(t_end),
                    pct,
                    prev.map(Val::Int).unwrap_or(Val::Null),
                    Val::Int(assignee),
                    Val::Text(phase.into()),
                ]);
                prev = Some(task_id);
            }
        }
        (projects, tasks)
    }

    pub(crate) fn gen_carrier_scorecard(&mut self) -> Table {
        let mut t = Table::new(
            "carrier_scorecard",
            vec![
                col("scorecard_id", Ty::Int),
                fk("carrier_id", "carrier", "carrier_id"),
                col("period_month", Ty::Date),
                col("on_time_pct", Ty::Numeric(5, 2)),
                col("damage_rate", Ty::Numeric(6, 4)),
                col("cost_index", Ty::Numeric(6, 3)),
                col("capacity_utilization", Ty::Numeric(5, 2)),
                col("claims_count", Ty::Int),
            ],
        );
        let mut sid = 0i64;
        for c in self.carriers.clone() {
            // Carrier 1 is the narrative under-performer.
            let laggard = c.id == 1;
            for m in 0..MARKET_MONTHS {
                sid += 1;
                let (y, mo) = (MARKET_START_YEAR + m / 12, m % 12 + 1);
                let date = ymd(y, mo as u32, 1);
                // Metrics are bounded normals around a mean; the laggard carrier
                // sits well below the pack (drives the Radar/Gauge narrative).
                let on_time = if laggard {
                    self.dec_normal(68.0, 6.0, 45.0, 82.0, 2)
                } else {
                    self.dec_normal(94.0, 4.0, 78.0, 99.9, 2)
                };
                let damage = if laggard {
                    self.dec_normal(0.040, 0.010, 0.010, 0.090, 4)
                } else {
                    self.dec_normal(0.006, 0.003, 0.0, 0.020, 4)
                };
                let cost_index = if laggard {
                    self.dec_normal(1.22, 0.08, 0.90, 1.50, 3)
                } else {
                    self.dec_normal(1.02, 0.10, 0.80, 1.35, 3)
                };
                let utilization = self.dec_normal(78.0, 10.0, 45.0, 99.0, 2);
                let claims = if laggard {
                    self.rng.poisson(11.0)
                } else {
                    self.rng.poisson(1.6)
                };
                t.push(vec![
                    Val::Int(sid),
                    Val::Int(c.id),
                    Val::Date(date),
                    on_time,
                    damage,
                    cost_index,
                    utilization,
                    Val::Int(claims),
                ]);
            }
        }
        t
    }

    /// The tier factor recovered from a scaled table count.
    fn sizing_factor(&self) -> usize {
        (self.sizing.carriers / 40).max(1)
    }
}

/// A `NUMERIC(_,4)` value from an `f64`.
fn dec4(x: f64) -> Val {
    Val::Dec((x * 10_000.0).round() as i64, 4)
}

/// A plausible base exchange rate (units of the currency per 1 USD).
fn base_rate(code: &str) -> f64 {
    match code {
        "USD" => 1.0,
        "EUR" => 0.92,
        "GBP" => 0.79,
        "JPY" => 148.0,
        "CNY" => 7.2,
        "CHF" => 0.88,
        "CAD" => 1.36,
        "AUD" => 1.52,
        "SGD" => 1.34,
        "HKD" => 7.82,
        "INR" => 83.0,
        "BRL" => 4.95,
        "MXN" => 17.1,
        "ZAR" => 18.7,
        "AED" => 3.67,
        "SAR" => 3.75,
        "SEK" => 10.6,
        "NOK" => 10.8,
        "PLN" => 4.0,
        "TRY" => 27.0,
        "KRW" => 1320.0,
        "NZD" => 1.63,
        "DKK" => 6.86,
        "ILS" => 3.7,
        _ => 1.0,
    }
}
