//! Render-level checks that a field's **effective** display format resolves from its stored format
//! leaf, not from the locale alone — the layer where a resolution bug becomes wrong output.
//!
//! Hermetic: the fixture renders from its own embedded saved data, and the locale is pinned rather
//! than inherited from the host (a host whose `LC_NUMERIC` is en-GB resolves system-default fields to
//! `£` and DD/MM, which is correct behaviour and would otherwise read as a failure here).

use rpt_pages::DrawOp;
use rpt_render::{Locale, RenderOptions, RenderSource};
use rpt_test_support::fixture;

/// Every text run drawn across the document, in page order.
fn drawn_text(doc: &rpt_pages::PagedDocument) -> Vec<String> {
    doc.pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

/// `{Orders.Order Date}` in this report is a DateTime-valued field whose stored `DateTimeOrder` is
/// `DateOnly`: the engine renders the date and no time component at all. Formatting it as a full
/// datetime would append the row's zero time to every date on the page.
#[test]
fn datetime_field_stored_as_date_only_renders_no_time() {
    let path = fixture("tests/fixtures/reports/benbrahim777/China Orders, with running totals.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open");
    let doc = rpt_render::render_with(
        rpt.report(),
        RenderOptions {
            locale: Locale::from_tag("en-US"),
            ..Default::default()
        },
    );

    let text = drawn_text(&doc);
    let dates: Vec<&String> = text.iter().filter(|t| t.contains("/2001")).collect();
    assert!(!dates.is_empty(), "report draws order dates: {text:?}");
    for d in &dates {
        assert!(
            !d.contains(':'),
            "date-only field must carry no time component, got {d:?}"
        );
    }
    // The stored components are leading-zero day/month + long year, in the stored MonthDayYear order.
    assert!(dates.iter().any(|d| *d == "06/05/2001"), "{dates:?}");
}

/// The same field's stored `DateOrder` is `MonthDayYear` — an authoring choice the engine honours
/// regardless of the host's regional order. Rendered under a day-month-year locale it must keep its
/// stored order, or an ambiguous date silently reads as a different day.
#[test]
fn explicit_date_order_survives_a_day_month_year_locale() {
    let path = fixture("tests/fixtures/reports/benbrahim777/China Orders, with running totals.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open");
    let doc = rpt_render::render_with(
        rpt.report(),
        RenderOptions {
            locale: Locale::from_tag("en-GB"),
            ..Default::default()
        },
    );

    let text = drawn_text(&doc);
    assert!(
        text.iter().any(|t| t == "06/05/2001"),
        "stored MonthDayYear order must survive an en-GB render: {text:?}"
    );
}

/// Both currency fields here store their own `$` and opt out of system defaults, so the symbol is a
/// per-field fact and does not follow the render locale. (A *system-default* currency field does
/// follow it — the engine ignores the stored numeric attributes wholesale when `EnableSystemDefault`
/// is set — which is why one report can legitimately show two symbols on a non-en-US host.)
#[test]
fn explicit_currency_symbol_is_locale_invariant() {
    let path = fixture("tests/fixtures/reports/benbrahim777/China Orders, with running totals.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open");
    let render_in = |loc: Locale| {
        drawn_text(&rpt_render::render_with(
            rpt.report(),
            RenderOptions {
                locale: loc,
                datasource: RenderSource::Saved,
                ..Default::default()
            },
        ))
    };

    // The separators do follow the locale (`$40.50` vs `$40,50`); only the symbol is stored. The
    // trailing space is the cell this field's bracketed negative form reserves.
    for (tag, amount) in [
        ("en-US", "$40.50 "),
        ("en-GB", "$40.50 "),
        ("de-DE", "$40,50 "),
    ] {
        let text = render_in(Locale::from_tag(tag));
        assert!(
            text.iter().any(|t| t == amount),
            "stored $ symbol must survive a {tag} render: {text:?}"
        );
    }
}

/// The three `synthetic/datetime_*` fixtures each isolate one stored time element.
///
/// Their expected strings include the exact stored punctuation and padding — leading spaces included,
/// which is where a plausible-looking implementation goes wrong. Each covers something a locale-only
/// time pattern cannot produce:
///
/// * `seconds` — the stored minute-second separator is the EMPTY string, so the seconds butt
///   straight onto the minutes. A "sensible default" of `:` silently breaks this.
/// * `no_hour` — dropping the hour drops the separator stored after it too, leaving the minutes
///   alone rather than a leading `:`.
/// * `no_minute` — dropping the *middle* element keeps the hour-minute separator, which now joins
///   the hour to the seconds. This is the case that shows the rule is "each element owns the
///   separator after it", not "drop the separator beside the missing part".
///
/// All three render 24-hour with no designator despite storing ` am`/` pm`, because the stored clock
/// base says so — and the hour occupies two cells, so midnight is `' 0'` and not `'0'`.
#[test]
fn stored_time_elements_render_as_the_engine_writes_them() {
    for (name, expected) in [
        ("datetime_seconds", "05/26/2001   0:0000"),
        ("datetime_no_hour", "05/26/2001  00"),
        ("datetime_no_minute", "05/26/2001   0:00"),
    ] {
        let path = fixture(format!("tests/fixtures/reports/synthetic/{name}.rpt"));
        let rpt = rpt_reader::Rpt::open(&path).expect("open");
        let doc = rpt_render::render_with(
            rpt.report(),
            RenderOptions {
                locale: Locale::from_tag("en-US"),
                datasource: RenderSource::Saved,
                ..Default::default()
            },
        );

        let text = drawn_text(&doc);
        assert!(
            text.iter().any(|t| t == expected),
            "{name}: no run reads {expected:?}; datetime-ish runs were {:?}",
            text.iter()
                .filter(|t| t.contains("/2001"))
                .take(4)
                .collect::<Vec<_>>()
        );
    }
}

/// The `datetime_twelve_hour_ampm_*` fixtures carry a ten-row batch spanning a full day, so every
/// row's time of day differs — the one shape of fixture that can tell a correct read of a saved
/// `DateTime` from one that always answers midnight. A saved batch packs both halves of a `DateTime`
/// into one serial (the Julian day low, the seconds since midnight high), so a per-row assertion
/// here is what holds that unpacking honest.
///
/// Each expected string reflects the stored format exactly: the stored `AMString`/`PMString` verbatim
/// with no separator of its own, the hour space-padded to two cells on the 12-hour clock as it is on
/// the 24-hour one, and the designator placed by the stored `AMPMFormat` — before the time in one
/// fixture, after it in the other. The hour's pad is character-visible in the designator-first
/// fixture; leading it, in the designator-last one, it sits before a right-aligned string and so is
/// the one cell this rendering cannot show either way.
///
/// Both fields format time-only, so the day half of the serial is asserted where it is unpacked
/// instead (`rpt-data`'s own tests), splitting the same ten rows eight/two across two days.
#[test]
fn a_saved_datetime_keeps_its_time_of_day_per_row() {
    for (name, times) in [
        (
            "datetime_twelve_hour_ampm_before",
            [
                "fore-noon 9:15",
                "fore-noon11:45",
                "eve-ning 1:05",
                "eve-ning12:00",
                "eve-ning 2:30",
                "eve-ning 6:50",
                "eve-ning 9:05",
                "eve-ning11:45",
                "fore-noon12:00",
                "fore-noon 5:09",
            ],
        ),
        (
            "datetime_twelve_hour_ampm_after",
            [
                " 9:15fore-noon",
                "11:45fore-noon",
                " 1:05eve-ning",
                "12:00eve-ning",
                " 2:30eve-ning",
                " 6:50eve-ning",
                " 9:05eve-ning",
                "11:45eve-ning",
                "12:00fore-noon",
                " 5:09fore-noon",
            ],
        ),
    ] {
        let path = fixture(format!("tests/fixtures/reports/synthetic/{name}.rpt"));
        let rpt = rpt_reader::Rpt::open(&path).expect("open");
        let doc = rpt_render::render_with(
            rpt.report(),
            RenderOptions {
                locale: Locale::from_tag("en-US"),
                datasource: RenderSource::Saved,
                ..Default::default()
            },
        );
        let text = drawn_text(&doc);

        let drawn: Vec<&str> = text
            .iter()
            .filter(|t| t.contains("fore-noon") || t.contains("eve-ning"))
            .map(String::as_str)
            .collect();
        assert_eq!(
            drawn, times,
            "{name}: per-row times must follow the batch, not collapse to midnight"
        );
    }
}

/// A saved-batch `Time` field (a real SQL `time` column, not a `DateTime`'s time-of-day half) is a
/// bare seconds-since-midnight serial (`parse_time_cell` in `rpt-data`'s `source.rs`) — a distinct
/// code path from the packed `DateTime` serial above. Ten rows spanning most of the day, none at
/// midnight, so a decoder that silently zeroed the value cannot pass by producing "00:00:00" ten
/// times over — an all-midnight batch hides a wrong unit completely.
#[test]
fn a_saved_time_field_keeps_its_time_of_day_per_row() {
    let path = fixture("tests/fixtures/reports/synthetic/saved_time_of_day.rpt");
    let rpt = rpt_reader::Rpt::open(&path).expect("open");
    let doc = rpt_render::render_with(
        rpt.report(),
        RenderOptions {
            locale: Locale::from_tag("en-US"),
            datasource: RenderSource::Saved,
            ..Default::default()
        },
    );
    let text = drawn_text(&doc);

    let times: Vec<&str> = text
        .iter()
        .filter(|t| t.contains(':'))
        .map(String::as_str)
        .collect();
    // Matches the seconds-since-midnight batch (`rpt saved`), character for character.
    assert_eq!(
        times,
        [
            "06:45:00", "07:30:00", "08:15:00", "11:50:00", "12:05:00", "13:40:00", "15:25:00",
            "18:10:00", "20:55:00", "23:20:00",
        ],
        "per-row times must follow the seconds-since-midnight batch, not collapse to midnight"
    );
}
