//! `formulas` — check every formula in a report without rendering it.
//!
//! A formula with a syntax error is not rejected anywhere in the pipeline: the parser recovers, the
//! evaluator runs the partial AST, and the field renders as a plausible-looking blank. On the render
//! path that now produces a diagnostic, but reaching it means having a datasource and rendering the
//! whole report. This is the read-only alternative — the file alone, no database, no render — so
//! "are this report's formulas valid?" is a question with a direct answer.
//!
//! Reports both syntax errors (the parser) and semantic ones (the validator: unknown functions, wrong
//! arity, operator type errors), for the main report and every subreport.

use crystal_formula::{validate_str, Severity, Syntax};
use rpt::model::{FieldKindData, FormulaSyntax, Report, ReportObjectKind};
use rpt::Rpt;
use serde::Serialize;

use crate::util::{print_json, CliError};

pub(crate) const HELP: &str = "\
rpt formulas — check every formula in a report, without rendering it

Parses and validates each formula the report defines and LISTS every one, so you can see what was
covered rather than only how many — with --source, the formula bodies too. Covers formula fields, the record- and group-selection
formulas, and conditional-format formulas wherever they hang — on a section, an object's format, its
border, or a field/text object's font colour — in the main report and every subreport. Problems are
reported under their formula with the message and the offending byte span. Reads the .rpt alone: no
database connection, no render.

This matters because nothing else rejects a broken formula. The parser recovers from a syntax error,
the evaluator runs the partial parse, and the field renders blank — indistinguishable from a null
value or an unimplemented feature.

USAGE:
    rpt formulas <file.rpt> [--source] [--json] [--quiet]

OPTIONS:
    --source   print each formula's source under its listing line
    --json     emit the listing and findings as JSON. Always includes each formula's source, kind,
               syntax, and size — no --source needed
    --quiet    print nothing; report only through the exit status (for CI)
    -h, --help show this help

OUTPUT:
    One line per formula, marked ok / warn / ERROR / empty, with any findings indented beneath it.
    `empty` is a formula field the report declares but left blank — listed so the accounting is
    complete, not counted as checked. With --source, the body is quoted under each line:

        report.rpt
          ok     formula \"Order Total\"                       crystal, 2 lines
                 │ Sum({orders.amount}, {orders.customer})
                 │   * (1 - {?Discount})
          ERROR  section Details's Section_Visibility formula  crystal, 1 line
                 │ {orders.total} > 100 and
                   error: expected an operand at byte 24
          empty  formula \"Unused\"                            crystal, 0 lines

        2 formulas checked, 1 declared but empty — 1 error, 0 warnings

EXIT STATUS:
    0   no errors (warnings may still have been printed)
    1   at least one formula has an error
";

/// One problem found in one formula.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Finding {
    /// The report the formula belongs to (empty for the main report).
    subreport: String,
    /// What the formula is: a formula field's name, or the role it plays.
    formula: String,
    /// `error` or `warning`.
    severity: &'static str,
    /// What is wrong.
    message: String,
    /// Byte offset of the problem within the formula body.
    start: usize,
    /// Byte offset one past the end.
    end: usize,
    /// The offending source text, when the span covers any.
    excerpt: String,
}

/// One formula that was checked, and how it came out.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Checked {
    /// The report the formula belongs to (empty for the main report).
    subreport: String,
    /// What the formula is, as displayed.
    formula: String,
    /// Where it is attached: `formula-field`, `record-selection`, `group-selection`,
    /// `section-format`, `object-format`, `border-format`, or `font-color-format`.
    kind: &'static str,
    /// The authoring dialect it was parsed as.
    syntax: &'static str,
    /// Lines in the formula body — enough to tell a one-liner from a real program.
    lines: usize,
    /// The formula's source. Always present in `--json`, where completeness beats terseness: a
    /// consumer can ignore a field it does not want, but cannot recover one that was never emitted.
    /// The text listing prints it only under `--source`, since a body is unbounded in size.
    body: String,
    errors: usize,
    warnings: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report_<'a> {
    file: &'a str,
    /// Formulas validated, across the main report and all subreports. Excludes those the report
    /// declares with an empty body, which are listed but have nothing to check.
    checked: usize,
    /// Formulas declared with an empty body.
    empty: usize,
    errors: usize,
    warnings: usize,
    /// Every formula checked, in report order — so a caller can see what was covered, not just how
    /// many. A formula with an empty body is skipped and does not appear.
    formulas: Vec<Checked>,
    findings: Vec<Finding>,
}

/// Check every formula in `file`. Errors (not warnings) make the command exit nonzero.
pub(crate) fn run(file: &str, json: bool, quiet: bool, source: bool) -> Result<(), CliError> {
    let rpt = Rpt::open(file)?;
    let mut findings = Vec::new();
    let mut formulas = Vec::new();
    check_report(rpt.report(), "", &mut findings, &mut formulas);
    for sub in &rpt.report().subreports {
        let name = if sub.name.is_empty() {
            "subreport"
        } else {
            &sub.name
        };
        check_report(&sub.report, name, &mut findings, &mut formulas);
    }
    let checked = formulas.iter().filter(|c| !c.is_empty()).count();
    let empty = formulas.len() - checked;

    // The parser can report the same problem more than once while recovering; a user needs the
    // distinct problems, not the recovery's step count.
    findings.dedup_by(|a, b| {
        (a.severity, &a.formula, &a.message, a.start)
            == (b.severity, &b.formula, &b.message, b.start)
    });
    let errors = findings.iter().filter(|f| f.severity == "error").count();
    let warnings = findings.len() - errors;

    // Tally each formula's own findings, so the listing can mark it and a reader can see at a glance
    // which of the checked formulas are the problem ones.
    for c in &mut formulas {
        for f in &findings {
            if f.subreport == c.subreport && f.formula == c.formula {
                match f.severity {
                    "error" => c.errors += 1,
                    _ => c.warnings += 1,
                }
            }
        }
    }

    if json {
        print_json(&Report_ {
            file,
            checked,
            empty,
            errors,
            warnings,
            formulas,
            findings,
        });
    } else if !quiet {
        print_listing(file, &formulas, &findings, source);
        println!("\n{}", summary(checked, empty, errors, warnings));
    }

    if errors > 0 {
        return Err(CliError::Strict(format!(
            "{file}: {errors} formula(s) have errors"
        )));
    }
    Ok(())
}

/// Print every formula that was checked, grouped by report, with its findings indented beneath it.
///
/// The listing is the point of the command: a count alone ("2 formulas checked") gives no way to tell
/// whether the thing you care about was among them, or whether the checker simply did not reach it.
fn print_listing(file: &str, formulas: &[Checked], findings: &[Finding], source: bool) {
    println!("{file}");
    if formulas.is_empty() {
        println!("  (this report defines no formulas)");
        return;
    }
    // Pad the status column to the widest label so the names line up.
    let width = formulas
        .iter()
        .map(|c| c.formula.chars().count())
        .max()
        .unwrap_or(0);
    let mut current = None;
    for c in formulas {
        if current != Some(&c.subreport) {
            if !c.subreport.is_empty() {
                println!("  subreport {}:", c.subreport);
            }
            current = Some(&c.subreport);
        }
        println!(
            "  {:5}  {:width$}  {}, {} line{}",
            status(c),
            c.formula,
            c.syntax,
            c.lines,
            if c.lines == 1 { "" } else { "s" },
        );
        if source {
            // Quoted with a rule so a multi-line body is unmistakably source and not another finding.
            // Not truncated: the flag is an explicit request for the whole thing.
            for line in c.body.lines() {
                println!("         │ {}", line.trim_end());
            }
        }
        for f in findings
            .iter()
            .filter(|f| f.subreport == c.subreport && f.formula == c.formula)
        {
            let near = if f.excerpt.is_empty() {
                String::new()
            } else {
                format!(" (near `{}`)", f.excerpt)
            };
            println!(
                "           {}: {} at byte {}{near}",
                f.severity, f.message, f.start
            );
        }
    }
}

impl Checked {
    /// Whether the report declares this formula but left its body blank — nothing to validate.
    fn is_empty(&self) -> bool {
        self.body.trim().is_empty()
    }
}

/// A formula's worst outcome, as the listing's status column. An error outranks a warning.
fn status(c: &Checked) -> &'static str {
    if c.is_empty() {
        "empty"
    } else if c.errors > 0 {
        "ERROR"
    } else if c.warnings > 0 {
        "warn"
    } else {
        "ok"
    }
}

/// The closing tally.
fn summary(checked: usize, empty: usize, errors: usize, warnings: usize) -> String {
    let plural = |n: usize, s: &str| format!("{n} {s}{}", if n == 1 { "" } else { "s" });
    if checked == 0 && empty == 0 {
        return "no formulas to check".to_string();
    }
    let empties = if empty == 0 {
        String::new()
    } else {
        format!(", {empty} declared but empty")
    };
    if checked == 0 {
        return format!("no formulas to check ({empty} declared but empty)");
    }
    format!(
        "{} checked{empties} — {}, {}",
        plural(checked, "formula"),
        plural(errors, "error"),
        plural(warnings, "warning")
    )
}

/// Check every formula `report` defines, recording each in `formulas` and any problem in `findings`.
///
/// Conditional-format formulas hang off several places in the model, not just the object format: a
/// section, an object's format, its border, and a field/text/field-heading object's font colour.
/// Walking only the object format misses most of them.
fn check_report(
    report: &Report,
    subreport: &str,
    findings: &mut Vec<Finding>,
    formulas: &mut Vec<Checked>,
) {
    let dd = &report.data_definition;
    let mut check = |label: String, body: &str, syntax: Syntax, kind: &'static str| {
        let empty = body.trim().is_empty();
        // A *named* formula field with a blank body was declared by the author and left unwritten, so
        // it is listed as `empty` — dropping it silently is the same "why isn't it here?" problem the
        // listing exists to answer. Every other kind exists only by virtue of having a body (a report
        // with no record selection stores an empty one), so there an empty body means absent, and
        // listing it would be noise.
        if empty && kind != "formula-field" {
            return;
        }
        formulas.push(Checked {
            subreport: subreport.to_string(),
            formula: label.clone(),
            kind,
            syntax: match syntax {
                Syntax::Basic => "basic",
                _ => "crystal",
            },
            lines: if empty {
                0
            } else {
                body.lines().count().max(1)
            },
            body: body.to_string(),
            errors: 0,
            warnings: 0,
        });
        if empty {
            return;
        }
        let ctx = crystal_formula::ValidationContext::default();
        for d in validate_str(body, syntax, &ctx) {
            findings.push(Finding {
                subreport: subreport.to_string(),
                formula: label.clone(),
                severity: match d.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                },
                message: d.message,
                start: d.start,
                end: d.end,
                excerpt: excerpt(body, d.start, d.end),
            });
        }
    };

    for f in &dd.field_definitions {
        if let FieldKindData::Formula(ff) = &f.kind {
            let syntax = match ff.syntax {
                FormulaSyntax::Basic => Syntax::Basic,
                _ => Syntax::Crystal,
            };
            check(
                format!("formula \"{}\"", f.name),
                &ff.text.0,
                syntax,
                "formula-field",
            );
        }
    }
    if let Some(sel) = &dd.record_selection {
        check(
            "the record-selection formula".to_string(),
            &sel.0,
            Syntax::Crystal,
            "record-selection",
        );
    }
    if let Some(sel) = &dd.group_selection {
        check(
            "the group-selection formula".to_string(),
            &sel.0,
            Syntax::Crystal,
            "group-selection",
        );
    }
    // Section-level conditional formats (e.g. `Section_Visibility`) — the most common kind in practice.
    for area in &report.report_definition.areas {
        for section in &area.sections {
            for (key, body) in &section.condition_formulas {
                check(
                    format!("section {}'s {key} formula", section.name),
                    body,
                    Syntax::Crystal,
                    "section-format",
                );
            }
        }
    }
    // Per-object conditional formats. These are the ones a render evaluates most often and the least
    // visible when broken — a wrong colour looks deliberate.
    for obj in report.objects() {
        for (key, body) in &obj.format.condition_formulas {
            check(
                format!("{}'s {key} formula", obj.name),
                body,
                Syntax::Crystal,
                "object-format",
            );
        }
        for (key, body) in &obj.border.condition_formulas {
            check(
                format!("{}'s border {key} formula", obj.name),
                body,
                Syntax::Crystal,
                "border-format",
            );
        }
        for (key, body) in font_color_formulas(&obj.kind) {
            check(
                format!("{}'s font-colour {key} formula", obj.name),
                body,
                Syntax::Crystal,
                "font-color-format",
            );
        }
    }
}

/// The font-colour conditional formulas of the object kinds that carry one.
fn font_color_formulas(kind: &ReportObjectKind) -> &[(String, String)] {
    match kind {
        ReportObjectKind::Field(f) => &f.font_color.condition_formulas,
        ReportObjectKind::Text(t) => &t.font_color.condition_formulas,
        ReportObjectKind::FieldHeading(h) => &h.font_color.condition_formulas,
        _ => &[],
    }
}

/// The source text a span covers, trimmed and capped. Empty when the span covers nothing usable.
fn excerpt(src: &str, start: usize, end: usize) -> String {
    const MAX: usize = 40;
    let end = end.min(src.len());
    if start >= end || !src.is_char_boundary(start) || !src.is_char_boundary(end) {
        return String::new();
    }
    let text = src[start..end].trim();
    match text.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(formula: &str, errors: usize, warnings: usize) -> Checked {
        Checked {
            subreport: String::new(),
            formula: formula.to_string(),
            kind: "formula-field",
            syntax: "crystal",
            lines: 1,
            body: "{a} > 1".to_string(),
            errors,
            warnings,
        }
    }

    #[test]
    fn the_summary_counts_and_pluralizes() {
        assert_eq!(summary(0, 0, 0, 0), "no formulas to check");
        assert_eq!(
            summary(1, 0, 0, 0),
            "1 formula checked — 0 errors, 0 warnings"
        );
        assert_eq!(
            summary(3, 0, 1, 2),
            "3 formulas checked — 1 error, 2 warnings"
        );
    }

    /// A formula the report declares but leaves blank is accounted for separately: it is listed, but
    /// counting it as "checked" would overstate what was verified.
    #[test]
    fn declared_but_empty_formulas_are_reported_apart_from_checked_ones() {
        assert_eq!(
            summary(2, 1, 0, 0),
            "2 formulas checked, 1 declared but empty — 0 errors, 0 warnings"
        );
        assert_eq!(
            summary(0, 2, 0, 0),
            "no formulas to check (2 declared but empty)"
        );

        let mut blank = checked("Unused", 0, 0);
        blank.body = "   \n ".to_string();
        assert!(blank.is_empty());
        assert_eq!(status(&blank), "empty");
        // And an empty body outranks nothing — a blank formula cannot have findings.
        assert!(!checked("real", 0, 0).is_empty());
    }

    /// The status column is what makes the listing scannable: a reader should be able to find the
    /// broken formula without reading the findings.
    #[test]
    fn status_reflects_the_worst_finding_on_each_formula() {
        assert_eq!(status(&checked("clean", 0, 0)), "ok");
        assert_eq!(status(&checked("warned", 0, 1)), "warn");
        assert_eq!(status(&checked("broken", 2, 0)), "ERROR");
        // An error outranks a warning on the same formula.
        assert_eq!(status(&checked("both", 1, 1)), "ERROR");
    }

    #[test]
    fn excerpt_quotes_the_offending_text_and_declines_a_useless_span() {
        assert_eq!(excerpt("Sum({a} * 2", 4, 7), "{a}");
        // An empty or reversed span, or one off a char boundary, yields nothing rather than a lie.
        assert_eq!(excerpt("abc", 2, 2), "");
        assert_eq!(excerpt("abc", 3, 1), "");
        assert_eq!(excerpt("héllo", 1, 2), "");
        // Whitespace-only spans carry no information.
        assert_eq!(excerpt("a   b", 1, 4), "");
    }

    #[test]
    fn excerpt_caps_a_long_span() {
        let long = "x".repeat(200);
        let e = excerpt(&long, 0, 200);
        assert!(e.ends_with('…'), "{e}");
        assert!(e.chars().count() <= 41, "{} chars", e.chars().count());
    }
}
