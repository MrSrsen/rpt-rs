# Format resolution

A field's displayed value is resolved from **two layers** (see `rpt-layout`'s format module):

1. the **locale** — the "system default" layer: separators, month/day names, AM/PM, default date order, default
   decimals, currency symbol; and
2. the field's **stored `FieldFormat`** record — the explicit authoring choices (decimals, negative style, currency
   symbol placement, date component forms, boolean word pair).

The field's own `use_system_defaults` flag arbitrates: when set, the locale supplies the effective format; otherwise the
stored record wins for the attributes it sets, with names/separators still coming from the locale (Crystal never stores
"January", only `MonthFormat::LongMonth`). This mirrors the native engine's runtime format resolution. The stored
numeric record's `thousands_separator` (grouping on/off), `suppress_if_zero` (a zero value renders blank),
`currency_position` (leading/trailing symbol placement), the date record's `date_order` (the day/month/year component
order), and the datetime record's `separator` (the string joining the date and time parts) are all honoured for an
explicit field; a system-default field takes grouping, symbol placement and component order from the locale.

The **time** record is the one place an explicit field takes nothing from the locale, because it stores its own
separators and designator text: `time_base` selects the clock (a 24-hour field emits no AM/PM designator at all,
whatever its designator strings hold), `hour`/`minute`/`second` the element styles, `am_pm_format` the designator's
placement, and
`hour_minute_separator`/`minute_second_separator` the joins — the minute-second one is genuinely the empty string on
some fields, which butts the two elements together (`0:0000`). Two rules are not in the bytes: the "no leading zero"
hour still occupies **two cells**, space-padded (midnight is `" 0"`, not `"0"`), and an element that is not shown takes
the separator *after* it, so `NoHour` renders `00` while `NoMinute` leaves the hour joined to the second by the
hour-minute separator.

The datetime record's `DateTimeOrder` sits **outside** that arbitration: it selects which of the two parts a
datetime-valued field shows and in what order, whatever the date/time sub-formats say. `DateOnly` and `TimeOnly`
collapse the field to a single part — a `DateOnly` field renders no time component at all, not a zero one.

A **positive** number reserves the character cells its negative form would occupy, so a column's positives line up with
its negatives: a leading cell for the minus (` 1,012`), a trailing cell for the closing bracket (`$53.90 `), both when
brackets wrap a symbol-less number (` 3080 `). A cell a leading/trailing currency symbol already fills is not padded,
and an unsigned value type — a page number — reserves none, having no negative form. The negative *currency* form is a
locale fact of its own, separate from the negative number form (`Locale::currency_negative`): en-US brackets an amount
where it leads a number with a minus, so a system-default currency field reserves the bracket's cell where a
system-default number reserves the minus's. An embedded run inside a text object reserves no cell — it sits in a
sentence, not a column.

`OneCurrencySymbolPerPage` is the one numeric attribute that is **not** a property of a value, so it is not resolved
here: the symbol prints on the field's first printed value of each page and is blanked on the rest, and which value is a
page's first is only known once pagination has settled page membership. It is applied as a post-pagination pass over the
laid-out pages (`rpt-layout`'s currency module), and the Page IR carries nothing extra for it. The symbol is *blanked*,
not deleted — one space per symbol character plus one, so a one-character symbol leaves a two-space gap — and a value
that printed no symbol anyway (a suppressed zero, or one replaced by a zero literal) neither claims the page's symbol
nor is rewritten. The symbol is granted once per page per **field object** — two flagged fields on one page each keep
their own symbol on their own first value. A flagged field inside a **subreport** resets **per subreport placement**,
and within one placement it follows the **host** page: a subreport flowing across three parent pages shows the symbol
once on each of them, while two placements of the same subreport on one host page each show their own. The grant is
keyed on the placement rather than the subreport's name, so repeating a subreport down a detail band does not blank
every instance after the first. The subreport's formatter therefore resolves nothing itself and hands its pending values
back, and the parent re-anchors each one as it merges the child's draw-ops, so a value the parent clips or slices away
drops out with it.

Because the locale is a *runtime* input, the same report legitimately renders differently on two hosts: a system-default
currency field shows `$` under en-US and `£` under en-GB, and a system-default date flips MM/DD to DD/MM. A report
mixing explicit and system-default fields therefore shows two currency symbols on a non-en-US host — that is the
engine's behaviour, not a defect. Pin `--locale` when comparing against a render produced elsewhere.

Not every decoded model field is a render input. These stored facts are exported/inspected but intentionally do **not**
feed the render pipeline (no wiring needed): `SummaryInfo.revision_number` / `last_saved_by` / `saved_printer_name`,
`ReportOptions.enable_verify_on_every_print` / `convert_null_field_to_default`, and
`Table.qualified_name`. Conversely, a field's `FieldValueType` (drives the format-spec branch above) and a chart's
`data_refs` / `category_refs` are verified render inputs.

---

← [Section-break & pagination controls](04-pagination.md) · [Index](README.md) ·
**Next:** [Paragraph typography](06-typography.md) →
