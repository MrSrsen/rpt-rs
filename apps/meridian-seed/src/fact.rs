//! Transactional / fact-table generators — the volume surface. These derive
//! from the master indexes recorded on [`World`] and keep amounts as integer
//! cents so totals reconcile exactly (order total = sum of its line amounts,
//! invoice gross = net + tax, and so on).

use crate::pools;
use crate::sql::{col, fk, fk_text, nul, Table, Ty, Val};
use crate::world::{InvoiceIx, LegIx, OrderIx, OrderLineIx, ShipmentIx, World};

// Order-status ids (1-based, mirror `pools::ORDER_STATUSES`).
const OS_QUOTE: i64 = 1;
const OS_CANC: i64 = 6;
// Payment-status ids.
const PS_OPEN: i64 = 1;
const PS_PARTIAL: i64 = 2;
const PS_PAID: i64 = 3;
const PS_OVERDUE: i64 = 4;
const PS_VOID: i64 = 5;
// Shipment-status ids.
const SS_DELIV: i64 = 5;

impl World {
    /// A weighted order status.
    fn order_status(&mut self) -> i64 {
        let r = self.rng.real(0.0, 1.0);
        match r {
            x if x < 0.08 => OS_QUOTE,
            x if x < 0.25 => 2, // Confirmed
            x if x < 0.40 => 3, // Picking
            x if x < 0.62 => 4, // Shipped
            x if x < 0.94 => 5, // Closed
            _ => OS_CANC,
        }
    }

    pub(crate) fn gen_orders(&mut self) -> (Table, Table) {
        let mut orders = Table::new(
            "sales_order",
            vec![
                col("order_id", Ty::Int),
                fk("customer_id", "customer", "customer_id"),
                col("order_date", Ty::Date),
                col("required_date", Ty::Date),
                fk("status_id", "order_status", "id"),
                fk_text("currency_code", "currency", "currency_code", 3),
                fk("sales_rep_id", "employee", "employee_id"),
                fk("ship_to_city_id", "city", "city_id"),
                fk("incoterm_id", "incoterm", "incoterm_id"),
                col("total_amount", Ty::Numeric(14, 2)),
            ],
        );
        let mut lines = Table::new(
            "order_line",
            vec![
                col("order_line_id", Ty::BigInt),
                fk("order_id", "sales_order", "order_id"),
                fk("product_id", "product", "product_id"),
                col("quantity", Ty::Int),
                col("unit_price", Ty::Numeric(12, 2)),
                col("discount_pct", Ty::Numeric(6, 3)),
                col("line_amount", Ty::Numeric(14, 2)),
            ],
        );

        let incoterms = pools::INCOTERMS.len() as i64;
        let mut line_id = 0i64;
        for i in 0..self.sizing.orders {
            let id = i as i64 + 1;
            // Customers are chosen in proportion to their Pareto spend weight.
            let customer = self.weighted_customer();
            let order_day = self.business_day();
            let required = order_day + self.rng.int(7, 45) as i32;
            let status = self.order_status();
            let ship_city = if self.rng.chance(0.7) {
                customer.city_id
            } else {
                self.weighted_city().id
            };
            let incoterm = self.rng.int(1, incoterms);

            // Lines per order: a Poisson count (at least one).
            let n = 1 + self.rng.poisson(2.3);
            let mut total = 0i64;
            let mut line_ix = Vec::new();
            for _ in 0..n {
                line_id += 1;
                let product = *self.rng.pick(&self.products.clone());
                // Quantity: a Poisson count (at least one). Line amount is then
                // log-normal (skewed unit price × count).
                let qty = 1 + self.rng.poisson(11.0);
                let unit = product.price_cents;
                let disc_milli = if self.rng.chance(0.4) {
                    self.rng.int(0, 25_000) // up to 25.000%
                } else {
                    0
                };
                let disc_frac = disc_milli as f64 / 100_000.0;
                let amount = ((qty * unit) as f64 * (1.0 - disc_frac)).round() as i64;
                total += amount;
                line_ix.push(OrderLineIx {
                    id: line_id,
                    amount_cents: amount,
                });
                lines.push(vec![
                    Val::Int(line_id),
                    Val::Int(id),
                    Val::Int(product.id),
                    Val::Int(qty),
                    Val::Dec(unit, 2),
                    Val::Dec(disc_milli, 3),
                    Val::Dec(amount, 2),
                ]);
            }
            self.order_lines_by_order.insert(id, line_ix);
            self.orders.push(OrderIx {
                id,
                customer_id: customer.id,
                ccy: customer.ccy,
                order_day,
                status_id: status,
                total_cents: total,
            });
            orders.push(vec![
                Val::Int(id),
                Val::Int(customer.id),
                Val::Date(order_day),
                Val::Date(required),
                Val::Int(status),
                Val::Text(customer.ccy.into()),
                Val::Int(customer.sales_rep_id),
                Val::Int(ship_city),
                Val::Int(incoterm),
                Val::Dec(total, 2),
            ]);
        }
        (orders, lines)
    }

    /// A weighted payment status.
    fn payment_status(&mut self) -> i64 {
        let r = self.rng.real(0.0, 1.0);
        match r {
            x if x < 0.50 => PS_PAID,
            x if x < 0.68 => PS_PARTIAL,
            x if x < 0.86 => PS_OPEN,
            x if x < 0.98 => PS_OVERDUE,
            _ => PS_VOID,
        }
    }

    pub(crate) fn gen_invoices(&mut self) -> (Table, Table) {
        let mut invoices = Table::new(
            "invoice",
            vec![
                col("invoice_id", Ty::Int),
                fk("order_id", "sales_order", "order_id"),
                fk("customer_id", "customer", "customer_id"),
                col("invoice_date", Ty::Date),
                col("due_date", Ty::Date),
                fk_text("currency_code", "currency", "currency_code", 3),
                col("amount_net", Ty::Numeric(14, 2)),
                col("tax_amount", Ty::Numeric(14, 2)),
                col("amount_gross", Ty::Numeric(14, 2)),
                fk("status_id", "payment_status", "id"),
            ],
        );
        let mut inv_lines = Table::new(
            "invoice_line",
            vec![
                col("invoice_line_id", Ty::BigInt),
                fk("invoice_id", "invoice", "invoice_id"),
                fk("order_line_id", "order_line", "order_line_id"),
                col("amount", Ty::Numeric(14, 2)),
            ],
        );
        const TAX_RATES: [i64; 5] = [0, 5_000, 10_000, 19_000, 21_000]; // milli-percent

        let mut inv_id = 0i64;
        let mut inv_line_id = 0i64;
        for o in self.orders.clone() {
            if o.status_id == OS_QUOTE || o.status_id == OS_CANC {
                continue;
            }
            inv_id += 1;
            let invoice_day = o.order_day + self.rng.int(1, 10) as i32;
            let due = invoice_day + self.rng.int(15, 60) as i32;
            let net = o.total_cents;
            let rate = *self.rng.pick(&TAX_RATES);
            let tax = (net as f64 * rate as f64 / 1_000_000.0).round() as i64;
            let gross = net + tax;
            let status = self.payment_status();
            self.invoices.push(InvoiceIx {
                id: inv_id,
                ccy: o.ccy,
                gross_cents: gross,
                invoice_day,
                status_id: status,
            });
            invoices.push(vec![
                Val::Int(inv_id),
                Val::Int(o.id),
                Val::Int(o.customer_id),
                Val::Date(invoice_day),
                Val::Date(due),
                Val::Text(o.ccy.into()),
                Val::Dec(net, 2),
                Val::Dec(tax, 2),
                Val::Dec(gross, 2),
                Val::Int(status),
            ]);
            // One or two invoice lines drawn from the order's lines.
            if let Some(order_lines) = self.order_lines_by_order.get(&o.id).cloned() {
                let take = self.rng.int(1, 2).min(order_lines.len() as i64) as usize;
                for ol in order_lines.into_iter().take(take) {
                    inv_line_id += 1;
                    inv_lines.push(vec![
                        Val::Int(inv_line_id),
                        Val::Int(inv_id),
                        Val::Int(ol.id),
                        Val::Dec(ol.amount_cents, 2),
                    ]);
                }
            }
        }
        (invoices, inv_lines)
    }

    pub(crate) fn gen_payment(&mut self) -> Table {
        let mut t = Table::new(
            "payment",
            vec![
                col("payment_id", Ty::Int),
                fk("invoice_id", "invoice", "invoice_id"),
                col("payment_date", Ty::Date),
                col("amount", Ty::Numeric(14, 2)),
                col("method", Ty::Varchar(24)),
                fk_text("currency_code", "currency", "currency_code", 3),
            ],
        );
        let mut pid = 0i64;
        for inv in self.invoices.clone() {
            let installments: i64 = match inv.status_id {
                PS_PAID => 1 + i64::from(self.rng.chance(0.3)),
                PS_PARTIAL => 1,
                PS_OVERDUE => i64::from(self.rng.chance(0.4)),
                _ => 0,
            };
            if installments == 0 {
                continue;
            }
            // Split the gross into `installments` parts (partial pays less).
            let paid_total = if inv.status_id == PS_PAID {
                inv.gross_cents
            } else {
                (inv.gross_cents as f64 * self.rng.real(0.2, 0.7)).round() as i64
            };
            let mut remaining = paid_total;
            for k in 0..installments {
                pid += 1;
                let amount = if k + 1 == installments {
                    remaining
                } else {
                    let part = (paid_total as f64 * 0.5).round() as i64;
                    remaining -= part;
                    part
                };
                let day = inv.invoice_day + self.rng.int(5, 90) as i32;
                let method = *self.rng.pick(pools::PAYMENT_METHODS);
                t.push(vec![
                    Val::Int(pid),
                    Val::Int(inv.id),
                    Val::Date(day),
                    Val::Dec(amount, 2),
                    Val::Text(method.into()),
                    Val::Text(inv.ccy.into()),
                ]);
            }
        }
        t
    }

    /// A weighted shipment status.
    fn shipment_status(&mut self) -> i64 {
        let r = self.rng.real(0.0, 1.0);
        match r {
            x if x < 0.08 => 1, // Booked
            x if x < 0.18 => 2, // Picked Up
            x if x < 0.34 => 3, // In Transit
            x if x < 0.42 => 4, // In Customs
            x if x < 0.92 => SS_DELIV,
            _ => 6, // Exception
        }
    }

    pub(crate) fn gen_shipment(&mut self) -> Table {
        let mut t = Table::new(
            "shipment",
            vec![
                col("shipment_id", Ty::Int),
                fk("order_id", "sales_order", "order_id"),
                fk("carrier_id", "carrier", "carrier_id"),
                fk("origin_facility_id", "facility", "facility_id"),
                fk("dest_facility_id", "facility", "facility_id"),
                fk("mode_id", "shipment_mode", "mode_id"),
                fk("service_level_id", "service_level", "level_id"),
                fk("status_id", "shipment_status", "id"),
                col("booked_date", Ty::Date),
                col("planned_pickup", Ty::Date),
                col("actual_pickup", Ty::Date),
                col("planned_delivery", Ty::Date),
                nul("actual_delivery", Ty::Date),
                col("weight_kg", Ty::Numeric(12, 2)),
                col("volume_m3", Ty::Numeric(12, 3)),
                col("chargeable_weight", Ty::Numeric(12, 2)),
                col("freight_cost", Ty::Numeric(14, 2)),
                fk_text("currency_code", "currency", "currency_code", 3),
                col("distance_km", Ty::Numeric(10, 2)),
            ],
        );
        let levels = pools::SERVICE_LEVELS;
        for i in 0..self.sizing.shipments {
            let id = i as i64 + 1;
            let order = *self.rng.pick(&self.orders.clone());
            let carrier = *self.rng.pick(&self.carriers.clone());
            let origin = *self.rng.pick(&self.facilities.clone());
            let mut dest = *self.rng.pick(&self.facilities.clone());
            if dest.id == origin.id && self.facilities.len() > 1 {
                dest = *self.rng.pick(&self.facilities.clone());
            }
            // Weighted service level (Standard most common, Priority rare).
            let level_idx = self.rng.weighted(&[25, 45, 22, 8]);
            let target = levels[level_idx].2;
            let status = self.shipment_status();

            let booked = order.order_day + self.rng.int(1, 7) as i32;
            let planned_pickup = booked + self.rng.int(1, 5) as i32;
            let actual_pickup = planned_pickup + self.rng.int(-1, 3) as i32;
            let planned_delivery = actual_pickup + target as i32;
            // Transit time is right-skewed (log-normal around the SLA target), so
            // the transit-time histogram has a realistic tail. Delivered shipments
            // have an actual delivery; in-flight ones may not (NULL → `HasValue`).
            let actual_delivery_day = if status == SS_DELIV {
                let transit = self.rng.lognormal(target as f64, 0.5).round().max(1.0) as i32;
                Some(actual_pickup + transit)
            } else if self.rng.chance(0.3) {
                let transit = self.rng.lognormal(target as f64, 0.6).round().max(1.0) as i32;
                Some(actual_pickup + transit)
            } else {
                None
            };
            // The shipment window closes at the actual delivery, or (in transit)
            // at the planned delivery; legs and events stay inside it.
            let end_day = actual_delivery_day.unwrap_or(planned_delivery);
            let actual_delivery_val = actual_delivery_day.map(Val::Date).unwrap_or(Val::Null);

            // Mass / size / cost are right-skewed (log-normal).
            let weight_kg = self.rng.lognormal(400.0, 1.0).clamp(5.0, 60_000.0);
            let volume_m3 = self.rng.lognormal(6.0, 0.9).clamp(0.1, 400.0);
            let charge_kg = weight_kg.max(volume_m3 * 167.0); // dimensional weight
            let weight = Val::Dec((weight_kg * 100.0).round() as i64, 2);
            let volume = Val::Dec((volume_m3 * 1_000.0).round() as i64, 3);
            let charge_w = Val::Dec((charge_kg * 100.0).round() as i64, 2);
            let freight = self.money_skewed(2_500.0, 0.85);
            let distance = self.dec_skewed(800.0, 0.9, 2);

            self.shipments.push(ShipmentIx {
                id,
                carrier_id: carrier.id,
                origin_facility: origin.id,
                dest_facility: dest.id,
                ccy: order.ccy,
                pickup_day: actual_pickup,
                end_day,
                delivered: actual_delivery_day.is_some(),
            });
            t.push(vec![
                Val::Int(id),
                Val::Int(order.id),
                Val::Int(carrier.id),
                Val::Int(origin.id),
                Val::Int(dest.id),
                Val::Int(carrier.mode_id),
                Val::Int(level_idx as i64 + 1),
                Val::Int(status),
                Val::Date(booked),
                Val::Date(planned_pickup),
                Val::Date(actual_pickup),
                Val::Date(planned_delivery),
                actual_delivery_val,
                weight,
                volume,
                charge_w,
                Val::Dec(freight, 2),
                Val::Text(order.ccy.into()),
                distance,
            ]);
        }
        t
    }

    pub(crate) fn gen_shipment_leg(&mut self) -> Table {
        let mut t = Table::new(
            "shipment_leg",
            vec![
                col("leg_id", Ty::BigInt),
                fk("shipment_id", "shipment", "shipment_id"),
                col("sequence", Ty::Int),
                fk("from_facility_id", "facility", "facility_id"),
                fk("to_facility_id", "facility", "facility_id"),
                fk("mode_id", "shipment_mode", "mode_id"),
                fk("carrier_id", "carrier", "carrier_id"),
                fk("vehicle_id", "vehicle", "vehicle_id"),
                col("planned_depart", Ty::Date),
                col("actual_depart", Ty::Date),
                col("planned_arrive", Ty::Date),
                col("actual_arrive", Ty::Date),
                col("distance_km", Ty::Numeric(10, 2)),
                col("leg_cost", Ty::Numeric(14, 2)),
            ],
        );
        let modes = pools::MODES.len() as i64;
        let mut leg_id = 0i64;
        for s in self.shipments.clone() {
            // Legs per shipment: a Poisson count (at least one).
            let n = 1 + self.rng.poisson(1.4);
            // Partition the shipment window [pickup, end] into `n` contiguous
            // legs by cumulative random shares: leg 1 departs at pickup, the last
            // arrives at end, and each leg starts where the previous one ended.
            let span = (s.end_day - s.pickup_day).max(0) as i64;
            let weights: Vec<i64> = (0..n).map(|_| self.rng.int(1, 10)).collect();
            let total: i64 = weights.iter().sum::<i64>().max(1);
            let mut acc = 0i64;
            let mut from = s.origin_facility;
            let mut depart = s.pickup_day;
            let mut legs = Vec::new();
            for (i, w) in weights.iter().enumerate() {
                leg_id += 1;
                let seq = i as i64 + 1;
                acc += *w;
                let arrive = if seq == n {
                    s.end_day
                } else {
                    s.pickup_day + (span * acc / total) as i32
                };
                let to = if seq == n {
                    s.dest_facility
                } else {
                    self.rng.pick(&self.facilities.clone()).id
                };
                let mode = self.rng.int(1, modes);
                let vehicle = self.pick_vehicle(s.carrier_id);
                let actual_depart = depart;
                let actual_arrive = arrive;
                // Planned arrival runs a touch ahead of the actual (a small delay),
                // clamped to stay within the leg window.
                let planned_depart = actual_depart;
                let planned_arrive = (actual_arrive - self.rng.int(0, 2) as i32).max(actual_depart);
                let distance = self.dec_skewed(300.0, 0.9, 2);
                let cost = self.money_skewed(1_500.0, 0.8);
                legs.push(LegIx {
                    id: leg_id,
                    from_facility: from,
                    to_facility: to,
                    arrive_day: actual_arrive,
                });
                t.push(vec![
                    Val::Int(leg_id),
                    Val::Int(s.id),
                    Val::Int(seq),
                    Val::Int(from),
                    Val::Int(to),
                    Val::Int(mode),
                    Val::Int(s.carrier_id),
                    Val::Int(vehicle),
                    Val::Date(planned_depart),
                    Val::Date(actual_depart),
                    Val::Date(planned_arrive),
                    Val::Date(actual_arrive),
                    distance,
                    Val::Dec(cost, 2),
                ]);
                from = to;
                depart = arrive;
            }
            self.legs_by_shipment.insert(s.id, legs);
        }
        t
    }

    /// A vehicle belonging to `carrier`, or any vehicle if it owns none.
    fn pick_vehicle(&mut self, carrier_id: i64) -> i64 {
        if let Some(v) = self.vehicles_by_carrier.get(&carrier_id).cloned() {
            if !v.is_empty() {
                return *self.rng.pick(&v);
            }
        }
        self.rng.int(1, self.sizing.vehicles as i64)
    }

    pub(crate) fn gen_tracking_event(&mut self) -> Table {
        let mut t = Table::new(
            "tracking_event",
            vec![
                col("event_id", Ty::BigInt),
                fk("shipment_id", "shipment", "shipment_id"),
                fk("leg_id", "shipment_leg", "leg_id"),
                col("event_time", Ty::Timestamp),
                col("event_type", Ty::Varchar(30)),
                fk("facility_id", "facility", "facility_id"),
                fk("city_id", "city", "city_id"),
                nul("temperature_c", Ty::Numeric(5, 2)),
                nul("notes", Ty::Varchar(200)),
            ],
        );
        let mut event_id = 0i64;
        let shipments = self.shipments.clone();
        for s in shipments {
            let legs = self
                .legs_by_shipment
                .get(&s.id)
                .cloned()
                .unwrap_or_default();
            if legs.is_empty() {
                continue;
            }
            // Events per shipment: a Poisson count (roughly 8–12), all inside the
            // shipment window and in chronological order.
            let n = 3 + self.rng.poisson(7.0);
            let span = (s.end_day - s.pickup_day).max(1) as i64;
            // Draw the scan times, then sort so they read in order. A delivered
            // shipment's terminal event lands at delivery midnight (so it never
            // sorts after `actual_delivery`); the rest stay strictly before the
            // window's upper bound.
            let intermediate = if s.delivered { n - 1 } else { n };
            let mut times: Vec<(i32, u32)> = Vec::with_capacity(n as usize);
            for _ in 0..intermediate {
                let day = s.pickup_day + self.rng.int(0, span - 1) as i32;
                times.push((day, self.secs()));
            }
            times.sort_unstable();
            if s.delivered {
                times.push((s.end_day, 0));
            }
            for (idx, (day, secs)) in times.iter().copied().enumerate() {
                event_id += 1;
                let terminal = s.delivered && idx + 1 == times.len();
                let leg = leg_for_day(&legs, day);
                let etype = if terminal {
                    "Delivered"
                } else {
                    *self.rng.pick(pools::EVENT_TYPES)
                };
                let facility = if self.rng.chance(0.5) {
                    leg.from_facility
                } else {
                    leg.to_facility
                };
                let city = self.facility_city(facility);
                // Temperature (reefer legs): a bounded normal around 4 °C.
                let temp = if self.rng.chance(0.4) {
                    Val::Dec(
                        (self.rng.bounded_normal(4.0, 9.0, -25.0, 30.0) * 100.0).round() as i64,
                        2,
                    )
                } else {
                    Val::Null
                };
                let notes = if self.rng.chance(0.25) {
                    Val::Text(format!("{etype} at hub; scan {event_id}."))
                } else {
                    Val::Null
                };
                t.push(vec![
                    Val::Int(event_id),
                    Val::Int(s.id),
                    Val::Int(leg.id),
                    Val::Ts(day, secs),
                    Val::Text(etype.into()),
                    Val::Int(facility),
                    Val::Int(city),
                    temp,
                    notes,
                ]);
            }
        }
        t
    }

    /// The city of a facility (falls back to the first city).
    fn facility_city(&self, facility_id: i64) -> i64 {
        self.facilities
            .iter()
            .find(|f| f.id == facility_id)
            .map(|f| f.city_id)
            .unwrap_or_else(|| self.cities.first().map(|c| c.id).unwrap_or(1))
    }

    pub(crate) fn gen_shipment_charge(&mut self) -> Table {
        let mut t = Table::new(
            "shipment_charge",
            vec![
                col("charge_id", Ty::BigInt),
                fk("shipment_id", "shipment", "shipment_id"),
                fk("charge_type_id", "charge_type", "charge_id"),
                col("amount", Ty::Numeric(14, 2)),
                fk_text("currency_code", "currency", "currency_code", 3),
            ],
        );
        let charge_types = pools::CHARGE_TYPES.len() as i64;
        let mut cid = 0i64;
        for s in self.shipments.clone() {
            // Always a freight charge; usually a fuel surcharge; sometimes others.
            let mut kinds = vec![1i64]; // Freight
            if self.rng.chance(0.85) {
                kinds.push(2); // Fuel Surcharge
            }
            let extra = self.rng.int(0, 2);
            for _ in 0..extra {
                kinds.push(self.rng.int(3, charge_types));
            }
            for kind in kinds {
                cid += 1;
                let amount = self.money_skewed(600.0, 0.9);
                t.push(vec![
                    Val::Int(cid),
                    Val::Int(s.id),
                    Val::Int(kind),
                    Val::Dec(amount, 2),
                    Val::Text(s.ccy.into()),
                ]);
            }
        }
        t
    }
}

/// The leg covering `day` — the first whose window ends on or after it (legs
/// partition the shipment window, so this always resolves to one).
fn leg_for_day(legs: &[LegIx], day: i32) -> LegIx {
    legs.iter()
        .find(|lg| day <= lg.arrive_day)
        .copied()
        .unwrap_or_else(|| *legs.last().expect("shipment has at least one leg"))
}
