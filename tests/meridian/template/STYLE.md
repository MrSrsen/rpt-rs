# Meridian report style guide + base-template spec

The shared visual/authoring conventions for the Meridian report corpus, and the build recipe for the base report
(`meridian_base.rpt`) that every report is derived from. Goal: 25 reports that read as **one company**.

**How to use:** build `meridian_base.rpt` once from §5, then **File → Save As** it into
`tests/meridian/reports/<division>/<name>.rpt` for each report, and add that report's data + body. The base carries
all shared chrome (logo, title, footer, fonts, colors, page setup).

> **Harness caveat:** keep `meridian_base.rpt` here under `tests/meridian/template/` — **NOT** under
> `tests/meridian/reports/`, or the recursive fixture harness will try to render/baseline it as a report.

---

## 1. Brand colors (tokens)

| Token | Hex | RGB (Crystal) | Use |
| ----- | --- | ------------- | --- |
| Navy | `#1F3A5F` | 31, 58, 95 | Report title, company name, group-1 headers, column-header fill |
| Teal | `#2C8C99` | 44, 140, 153 | Accent rules, underlines, chart primary, KPI highlights |
| Charcoal | `#222222` | 34, 34, 34 | Body text |
| Muted gray | `#6B7683` | 107, 118, 131 | Footer text, secondary labels |
| Light fill | `#F2F5F8` | 242, 245, 248 | Zebra/alt-row shading, group-band background |
| White | `#FFFFFF` | 255, 255, 255 | Text on navy fills, page background |
| Alert red | `#9E2B25` | 158, 43, 37 | Negative amounts, over-budget / SLA-breach conditional formatting |

---

## 2. Typography

Body font **Arial** (a safe, metrics-stable default; the deterministic `ApproxLayout` renderer approximates all fonts,
so family choice is about the intended visual look, not baseline stability).

| Element | Font | Size | Weight | Color |
| ------- | ---- | ---- | ------ | ----- |
| Report title | Arial | 16 | Bold | Navy |
| Company name (header) | Arial | 11 | Bold | Navy |
| Group #1 header | Arial | 11 | Bold | Navy |
| Group #2–#7 header | Arial | 10 → 9 | Bold | Navy |
| Column headings | Arial | 8 | Bold | White on Navy fill (or Navy on teal underline) |
| Body / detail | Arial | 9 | Regular | Charcoal |
| Subtotals / totals | Arial | 9 | Bold | Charcoal (Navy for grand total) |
| Page footer | Arial | 7 | Regular | Muted gray |

---

## 3. Page geometry (A4 portrait default)

- **Size:** A4 = **11906 × 16838 twips** (210 × 297 mm). Portrait is the default.
- **Margins:** 0.5 in = **720 twips** on all four sides → usable width **10466 twips** (~7.27 in).
- **Landscape overrides** (set per report): **R05** Invoice Aging Crosstab, **R07** Operational Manifest, **R15**
  Executive Dashboard, **R16** Revenue Crosstab, **R17** Capital Projects Gantt (timeline width). Landscape A4 =
  16838 × 11906 twips.

---

## 4. Standard field formats

| Kind | Format | Example |
| ---- | ------ | ------- |
| Currency | field's own currency symbol, 2 dp, thousands sep, **negatives in ( )** and Alert-red | `€ 12,480.00`, `(€ 305.00)` |
| Multi-currency label | show `currency_code` next to converted values | `12,480.00 EUR` |
| Date | `dd-MMM-yyyy` (unambiguous, European) | `07-Jul-2026` |
| DateTime | `dd-MMM-yyyy HH:mm` | `07-Jul-2026 14:37` |
| Integer / count | `#,##0` | `9,932` |
| Decimal / rate | `#,##0.00` (4 dp for FX/OHLC) | `1.0850` |
| Percent | `#,##0.0%` | `67.9%` |
| Boolean (`is_*`) | `Yes` / `No` via formula | `Yes` |

---

## 5. Building `meridian_base.rpt` (step by step)

1. **New → Blank Report**, no data source yet (chrome is data-independent; add tables after each Save-As).
2. **Page setup:** A4, Portrait; margins 0.5 in all sides. **File → Options → Fields**: set the default Date / Number /
   Currency formats from §4. Default font Arial 9, Charcoal.
3. **Report Header (RHb)** — height ~1080 twips (0.75 in):
   - **Insert → Picture** → `tests/meridian/template/assets/mgl_logo.png`, ~0.55 in square, at the left margin.
   - Text object **"MERIDIAN GLOBAL LOGISTICS"** — Navy, Arial 11 Bold — to the right of the logo.
   - A **`@ReportTitle`** text/formula object — Navy, Arial 16 Bold — below the company name (each report sets its title
     via the `@ReportTitle` formula, so the base has one place to change).
   - A **teal accent rule** (Line or 1-px Box, ~20 twips thick, Teal) spanning the usable width at the band's bottom.
4. **Page Header (PHb)** — leave mostly empty for per-report column headings; optionally a thin Light-fill/teal rule.
5. **Page Footer (PFb)** — height ~360 twips (0.25 in), Arial 7 Muted-gray, with a thin top rule:
   - **Left:** text `Confidential · Meridian Global Logistics · synthetic demo data`.
   - **Center:** Print Date special field (`dd-MMM-yyyy`).
   - **Right:** `"Page " + PageNumber + " of " + TotalPageCount` (or the **Page N of M** special field).
6. **Report Footer (RFb):** empty in the base (reports add grand totals).
7. **Save** as `tests/meridian/template/meridian_base.rpt`.

Then per report: **File → Save As** into `tests/meridian/reports/<division>/<name>.rpt`, open **Database Expert** to add
tables or a **Command** (raw SQL), and build the body.

---

## 6. Layout & section conventions

- **Group headers:** group name Navy Bold, with a teal underline rule; indent deeper levels slightly. Turn on
  *Keep Group Together*.
- **Zebra striping:** optional Light-fill background on alternating detail rows (`Remainder(RecordNumber,2)=0`).
- **Subtotals** in the group footer, Bold; **grand total** in the report footer, Navy Bold, above a teal rule.
- **Suppress-if-blank** on optional sections; **suppress** detail on drill-down group reports (*Hide (Drill-Down OK)*).
- Give **every detail an explicit sort or group** (Postgres has no implicit order — harness requirement).
- Satisfy each report's **RecordSelectionFormula** against seeded data (see `tests/meridian/SCHEMA.md` for ranges).

### Multi-page reports (pagination)
Several reports run long and paginate (from the small-tier volume): **R03** Sales by Region & Rep (~5–8+ pp), **R02**
Product Catalog, **R16** Revenue Crosstab, **R19** Employee Directory, **R01** Customer Statement (new page per customer),
and **R07** Operational Manifest (the extreme — dozens+ at the large tier). Conventions so long reports read well:
- **Column headers live in the Page Header** so they repeat on every page; the opening chart/summary lives in the
  **Report Header** so it appears on page 1 only; the grand total lives in the **Report Footer** (last page only).
- On a group that breaks across pages, **repeat the group header** (Group Expert → *Repeat Group Header On Each Page*)
  and mark it "(continued)".
- Carry running subtotals across breaks with a **"brought forward"** line at the top of a continued group.
- Turn on **Keep Group Together** so a group's rows don't orphan a lone line at a page foot.
- Every page carries the shared **Page N of M** footer; the count reaches M ≥ 5 on the long reports.

---

## 7. Naming conventions

| Thing | Convention | Examples |
| ----- | ---------- | -------- |
| Report file | `snake_case.rpt` under a division dir | `sales/customer_statement.rpt` |
| Formula | `@PascalCase`, descriptive | `@ReportTitle`, `@AddressBlock`, `@Balance`, `@AgingBucket`, `@TransitDays` |
| Parameter | `?PascalCase` | `?StatementDate`, `?ReportingCurrency`, `?Regions`, `?DateRange` |
| Running total | `RT` + name | `RTBalance`, `RTRevenue` |
| Group | named by its dimension | `Region`, `Country`, `Customer` |

---

## 8. Assets

- `assets/mgl_logo.svg` — editable source (navy badge, teal meridian-globe + shipment dot, "MGL", teal underscore).
- `assets/mgl_logo.png` — 512×512 RGBA, the embeddable header logo.
- Regenerate the PNG: `inkscape mgl_logo.svg --export-text-to-path --export-plain-svg=/tmp/p.svg && rsvg-convert -w 512 -h 512 /tmp/p.svg -o mgl_logo.png`.

Logo motif: a *meridian* (longitude line) on a globe = the company name; the teal dot is a shipment moving along a route.
