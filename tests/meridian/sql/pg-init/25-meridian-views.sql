-- Helper views for the meridian corpus. Crystal's table-linking join engine expresses only
-- equi/outer joins between whole relations: it cannot push down a per-row aggregate (SUM of payments
-- per invoice) or a correlated "latest row <= this date" lookup. Both are precomputed here as
-- ordinary 1-row-per-key views, bindable exactly like any other table.
--
-- exchange_rate_latest uses the single latest known rate per currency, not the rate as of each
-- invoice's date.
--
-- Run after meridian.sql by 20-meridian.sh (numbered so it sorts after the main seed).

CREATE OR REPLACE VIEW invoice_payment_totals AS
  SELECT invoice_id, SUM(amount) AS paid_total
  FROM payment
  GROUP BY invoice_id;

CREATE OR REPLACE VIEW exchange_rate_latest AS
  SELECT DISTINCT ON (currency_code) currency_code, rate_to_usd
  FROM exchange_rate
  ORDER BY currency_code, rate_date DESC;
