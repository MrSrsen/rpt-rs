//! Top-level `<Report>` document assembly.

use std::fmt::Write as _;

use rpt::model::{FieldKind, FieldKindData, Report};

use crate::export::database::{
    formula_fields, parameter_fields, running_total_fields, write_database, write_parameter,
    write_running_total,
};
use crate::export::objects::{write_area, write_node};
use crate::export::util::{
    b, escape, escape_text, paper_size_str, paper_source_str, value_type_name, write_collection,
};

/// Serialize a [`Report`] to XML (`full` also appends the raw record tree).
pub(crate) fn to_xml(file: &str, report: &Report, full: bool) -> String {
    let mut o = String::new();
    o.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    // (body assembled below with `\n`; converted to a BOM + CRLF document on return)
    // Top-level report attributes (subreports emit only `Name`, see `write_report_core` caller).
    // HasSavedData is `report.HasSavedData.ToString()`: true when the file carries a saved result
    // set (detected from the saved-data block descriptor record; the rows themselves are not read).
    let _ = writeln!(
        o,
        "<Report Name=\"\" FileName=\"{}\" HasSavedData=\"{}\">",
        escape(file),
        b(report.has_saved_data),
    );

    // Embedinfo — embedded OLE objects, summarised by their Ole stream digest.
    write_collection(&mut o, "  ", "Embedinfo", &report.embeds, |o, e| {
        let _ = writeln!(
            o,
            "    <Embed Name=\"{}\" Size=\"{}\" MD5Hash=\"{}\" />",
            escape(&e.name),
            e.size,
            escape(&e.md5_hash)
        );
    });

    let si = &report.summary_info;
    let _ = writeln!(
        o,
        "  <Summaryinfo KeywordsinReport=\"{}\" ReportAuthor=\"{}\" ReportComments=\"{}\" ReportSubject=\"{}\" ReportTitle=\"{}\" />",
        escape(&si.keywords), escape(&si.author), escape(&si.comments), escape(&si.subject), escape(&si.title));

    // ReportOptions — EnableSavePreviewPicture reflects the stored preview flag; EnableSaveDataWithReport
    // is bit 0 of the report-header option byte; the remaining flags are constant defaults
    // (SaveSummariesWithReport on, the rest off/empty).
    let ro = &report.report_options;
    let _ = writeln!(
        o,
        "  <ReportOptions EnableSaveDataWithReport=\"{}\" EnableSavePreviewPicture=\"{}\" EnableSaveSummariesWithReport=\"{}\" EnableUseDummyData=\"{}\" initialDataContext=\"\" initialReportPartName=\"\" />",
        b(ro.save_data_with_report),
        b(ro.save_preview_picture),
        b(ro.save_summaries_with_report),
        b(ro.use_dummy_data),
    );

    let po = &report.print_options;
    let m = &po.margins;
    // Page geometry (content size = paper less margins, from the 0x18e/0x66 records) and the
    // DEVMODE orientation/size/source (0x07). PrinterDuplex is left at its Default (its DEVMODE
    // field sits in a variable-offset tail); PrinterName is reported empty.
    let _ = writeln!(
        o,
        "  <PrintOptions PageContentHeight=\"{}\" PageContentWidth=\"{}\" PaperOrientation=\"{:?}\" PaperSize=\"{}\" PaperSource=\"{}\" PrinterDuplex=\"{:?}\" PrinterName=\"{}\">",
        po.content_height.0,
        po.content_width.0,
        po.paper_orientation,
        paper_size_str(po.paper_size),
        paper_source_str(po.paper_source),
        po.printer_duplex,
        escape(&po.printer_name),
    );
    let _ = writeln!(
        o,
        "    <PageMargins bottomMargin=\"{}\" leftMargin=\"{}\" rightMargin=\"{}\" topMargin=\"{}\" />",
        m.bottom.0, m.left.0, m.right.0, m.top.0
    );
    o.push_str("    <PageMarginConditionFormulas />\n");
    o.push_str("  </PrintOptions>\n");

    // Subreports — each is a full nested <Report> (decoded from its Subdocument stream).
    write_collection(&mut o, "  ", "SubReports", &report.subreports, |o, sub| {
        let _ = writeln!(o, "    <Report Name=\"{}\">", escape(&sub.name));
        // The shared body is written at the top-level (2-space) indent; a subreport sits two
        // levels deeper (inside <SubReports><Report>), so re-indent its lines by 4 spaces.
        let mut body = String::new();
        write_report_core(&mut body, &sub.report, &sub.links, true, &sub.name);
        for line in body.lines() {
            if line.is_empty() {
                o.push('\n');
            } else {
                let _ = writeln!(o, "    {line}");
            }
        }
        o.push_str("    </Report>\n");
    });

    write_report_core(&mut o, report, &[], false, "");
    write_saved_data(&mut o, report);

    if full {
        o.push_str("  <Records>\n");
        for node in &report.records {
            write_node(&mut o, node, 2);
        }
        o.push_str("  </Records>\n");
    }

    o.push_str("</Report>\n");

    // .NET XmlWriter output: a UTF-8 BOM, then CRLF line endings. Every remaining literal `\n` is a
    // structural newline (text-content newlines were already entitized to `&#xD;&#xA;` in
    // `escape_text`, and attribute values drop control chars), so a blanket `\n` -> `\r\n` is exact.
    let mut doc = String::with_capacity(o.len() + o.len() / 32 + 3);
    doc.push('\u{FEFF}');
    let mut rest = o.as_str();
    while let Some(i) = rest.find('\n') {
        doc.push_str(&rest[..i]);
        doc.push_str("\r\n");
        rest = &rest[i + 1..];
    }
    doc.push_str(rest);
    doc
}

/// Emit the report's stored saved data as `<SavedData>` (record count, columns, rows). Reports with
/// no decodable saved data emit an empty `<SavedData />`.
/// Map the lowercase canonical date-group period token (stored on `Group::date_condition`) to the
/// exact operand string the engine renders inside `GroupName ({field}, "…")`. The casing is the SDK
/// `CrGroupConditionEnum` spelling — most are plain title-case but `SemiAnnually` and the `By…`
/// time-of-day forms are not, so this table is authoritative rather than a title-case transform.
fn group_condition_operand(token: &str) -> &str {
    match token {
        "daily" => "Daily",
        "weekly" => "Weekly",
        "biweekly" => "Biweekly",
        "semimonthly" => "Semimonthly",
        "monthly" => "Monthly",
        "quarterly" => "Quarterly",
        "semiannually" => "SemiAnnually",
        "annually" => "Annually",
        "bysecond" => "BySecond",
        "byminute" => "ByMinute",
        "byhour" => "ByHour",
        "byampm" => "ByAMPM",
        // Unknown token: fall back to the raw stored value (never reached for decoded periods).
        other => other,
    }
}

fn write_saved_data(o: &mut String, report: &Report) {
    let Some(sd) = &report.saved_data else {
        o.push_str("  <SavedData />\n");
        return;
    };
    let _ = writeln!(o, "  <SavedData RecordCount=\"{}\">", sd.record_count);
    o.push_str("    <Fields>\n");
    for c in &sd.columns {
        let _ = writeln!(
            o,
            "      <Field Name=\"{}\" Type=\"{}\" />",
            escape(&c.name),
            value_type_name(c.value_type),
        );
    }
    o.push_str("    </Fields>\n");
    o.push_str("    <Rows>\n");
    for row in &sd.rows {
        o.push_str("      <Row>\n");
        for cell in row {
            let _ = writeln!(
                o,
                "        <F>{}</F>",
                escape_text(cell.as_deref().unwrap_or(""))
            );
        }
        o.push_str("      </Row>\n");
    }
    o.push_str("    </Rows>\n");
    o.push_str("  </SavedData>\n");
}

/// The shared body of a `<Report>` (main or subreport): Database, DataDefinition, ReportDefinition.
fn write_report_core(
    o: &mut String,
    report: &Report,
    incoming_links: &[rpt::model::SubreportLink],
    is_subreport: bool,
    report_name: &str,
) {
    // Derived analytics (field UseCount + parameter usage flags) come from the engine layer,
    // computed once per report level and threaded into the pure-formatting writers below.
    // `incoming_links` are the parent's subreport links targeting this level (empty for the main
    // report), used to mark link-fed parameters as `InUse`.
    let analysis = crate::export::analysis::analyze(report, incoming_links);

    // Database — tables (with the SQL Command + connection), table links, and the field
    // schema, all decoded from the QESession stream.
    write_database(o, report, &analysis.field_use_counts);

    // DataDefinition — selection formulas, groups, sort fields, formula and parameter fields.
    o.push_str("  <DataDefinition>\n");
    write_selection_formulas(o, report);
    write_groups(o, report);
    write_sort_fields(o, report);
    write_formula_fields(o, report, &analysis);
    write_group_name_fields(o, report);
    write_parameter_defs(o, report, &analysis, report_name);
    write_running_totals(o, report);
    // SQLExpressionFields — always emitted; members not decoded yet.
    o.push_str("    <SQLExpressionFields />\n");
    write_summary_fields(o, &analysis);
    o.push_str("  </DataDefinition>\n");
    // CustomFunctions sits between DataDefinition and ReportDefinition (not decoded yet).
    o.push_str("  <CustomFunctions />\n");

    // SubReportLinks — emitted by every subreport (only), between CustomFunctions and
    // ReportDefinition. The main report never emits it.
    if is_subreport {
        write_subreport_links(o, incoming_links);
    }

    // ReportDefinition — areas / sections / objects.
    o.push_str("  <ReportDefinition>\n    <Areas>\n");
    for area in &report.report_definition.areas {
        write_area(o, report, area, 3);
    }
    o.push_str("    </Areas>\n  </ReportDefinition>\n");
}

/// `<GroupSelectionFormula>` + `<RecordSelectionFormula>`. Each is emitted as the engine's canonical
/// re-render when it is SQL-pushable, else as the stored body verbatim (see
/// `selection::render_selection`); an absent/empty formula is written self-closing.
fn write_selection_formulas(o: &mut String, report: &Report) {
    let dd = &report.data_definition;
    // Canonical alias.field casing (lowercased -> canonical) for re-casing refs in the engine's
    // re-rendered selection text (applied only to SQL-pushable formulas; see `render_selection`).
    let alias_canon: std::collections::HashMap<String, String> = report
        .database
        .tables
        .iter()
        .flat_map(|t| &t.data_fields)
        .filter_map(|f| f.long_name.clone())
        .map(|ln| (ln.to_ascii_lowercase(), ln))
        .collect();
    let sel_text = |body: &str| {
        crate::export::selection::render_selection(body, &alias_canon)
            .unwrap_or_else(|| body.to_string())
    };
    match &dd.group_selection {
        Some(f) if !f.0.is_empty() => {
            let _ = writeln!(
                o,
                "    <GroupSelectionFormula>{}</GroupSelectionFormula>",
                escape_text(&sel_text(&f.0))
            );
        }
        _ => o.push_str("    <GroupSelectionFormula />\n"),
    }
    match &dd.record_selection {
        Some(f) if !f.0.is_empty() => {
            let _ = writeln!(
                o,
                "    <RecordSelectionFormula>{}</RecordSelectionFormula>",
                escape_text(&sel_text(&f.0))
            );
        }
        _ => o.push_str("    <RecordSelectionFormula />\n"),
    }
}

/// `<Groups>` — one `<Group>` per decoded group carrying its brace-wrapped condition field.
fn write_groups(o: &mut String, report: &Report) {
    let dd = &report.data_definition;
    write_collection(o, "    ", "Groups", &dd.groups, |o, g| {
        let _ = writeln!(
            o,
            "      <Group ConditionField=\"{{{}}}\" />",
            escape(&g.condition_field)
        );
    });
}

/// `<SortFields>` — record and group sorts.
fn write_sort_fields(o: &mut String, report: &Report) {
    let dd = &report.data_definition;
    write_collection(o, "    ", "SortFields", &dd.record_sorts, |o, s| {
        // A plain field reference is brace-wrapped (`{Table.Field}`); a Top N / Bottom N group
        // sort already holds a full summary expression (`Sum ({…}, {…})`) — detected by its
        // parenthesis — and must not be re-wrapped.
        let field = if s.field.contains('(') {
            escape(&s.field)
        } else {
            format!("{{{}}}", escape(&s.field))
        };
        // A summary-based group sort is a `TopBottomNSortField` and carries three extra attrs
        // (SDK `TopBottomNSortField.NumberOfTopOrBottomNGroups` / `EnableDiscardOtherGroups` /
        // `NotInTopBottomNName`). Plain group-field sorts and record sorts (`topn == None`)
        // omit them, matching the engine oracle.
        let topn = match &s.topn {
            Some(t) => format!(
                " NumberOfTopOrBottomNGroups=\"{}\" EnableDiscardOtherGroups=\"{}\" NotInTopBottomNName=\"{}\"",
                t.number_of_groups,
                if t.discard_others { "True" } else { "False" },
                escape(&t.not_in_topn_name),
            ),
            None => String::new(),
        };
        let _ = writeln!(
            o,
            "      <SortField Field=\"{}\" SortDirection=\"{:?}\" SortType=\"{:?}\"{} />",
            field, s.direction, s.kind, topn
        );
    });
}

/// `<FormulaFieldDefinitions>` — one per decoded formula field; always emitted open/close (never
/// self-closing). The body is element text with newlines preserved so multi-line formulas diff
/// line-by-line.
fn write_formula_fields(
    o: &mut String,
    report: &Report,
    analysis: &crate::export::analysis::ReportAnalysis,
) {
    o.push_str("    <FormulaFieldDefinitions>\n");
    for f in formula_fields(report) {
        if let FieldKindData::Formula(ff) = &f.kind {
            // A formula referencing an undefined parameter/formula fails to compile at load → the
            // engine reports it as UnknownField/0 (see `crate::export::analysis::stale_formulas`); otherwise use
            // the decoded value type and length.
            let (value_type, number_of_bytes) = if analysis.stale_formulas.contains(&f.name) {
                ("UnknownField".to_string(), 0)
            } else {
                (value_type_name(f.value_type), ff.number_of_bytes)
            };
            // `{@name}` formula reference, value type, then the body as element text (newlines
            // preserved so multi-line formulas diff line-by-line).
            let _ = writeln!(
                o,
                "      <FormulaFieldDefinition FormulaName=\"{{@{}}}\" Kind=\"{}\" Name=\"{}\" NumberOfBytes=\"{}\" ValueType=\"{}\" Syntax=\"{}\">{}</FormulaFieldDefinition>",
                escape(&f.name),
                f.kind.field_kind().name(),
                escape(&f.name),
                number_of_bytes,
                value_type,
                ff.syntax.name(),
                escape_text(&ff.text.0)
            );
        }
    }
    o.push_str("    </FormulaFieldDefinitions>\n");
}

/// `<GroupNameFieldDefinitions>` — one auto-generated per group, synthesised from the decoded groups.
fn write_group_name_fields(o: &mut String, report: &Report) {
    let dd = &report.data_definition;
    if dd.groups.is_empty() {
        o.push_str("    <GroupNameFieldDefinitions />\n");
    } else {
        o.push_str("    <GroupNameFieldDefinitions>\n");
        for (i, g) in dd.groups.iter().enumerate() {
            let field = &g.condition_field;
            // A date/time/boolean group's grouping period is appended as a string operand:
            // `GroupName ({field}, "Monthly")`. It appears in FormulaName/Name only — not
            // GroupNameFieldName. Built with the raw field, then XML-escaped as a whole (the literal
            // quotes around the period become `&quot;`). `date_condition` stores the lowercase
            // canonical token (also matched by the render pipeline); the exact engine operand string
            // is looked up here — most are simple title-case but `SemiAnnually` and the `By…`
            // time-of-day forms are not, so a naive first-letter cap is wrong.
            let group_name = match &g.date_condition {
                Some(c) => {
                    let cond = group_condition_operand(c);
                    format!("GroupName ({{{field}}}, \"{cond}\")")
                }
                None => format!("GroupName ({{{field}}})"),
            };
            let gn = escape(&group_name);
            let _ = writeln!(
                o,
                "      <GroupNameFieldDefinition FormulaName=\"{gn}\" Group=\"CrystalDecisions.CrystalReports.Engine.Group\" GroupNameFieldName=\"Group #{n} Name: {f}\" Kind=\"{kind}\" Name=\"{gn}\" NumberOfBytes=\"65534\" ValueType=\"StringField\" />",
                f = escape(field),
                n = i + 1,
                kind = FieldKind::GroupNameField.name(),
            );
        }
        o.push_str("    </GroupNameFieldDefinitions>\n");
    }
}

/// `<ParameterFieldDefinitions>` — the report's own parameters as full definitions, then the
/// subreport-link parameter stubs. Always emitted open/close.
fn write_parameter_defs(
    o: &mut String,
    report: &Report,
    analysis: &crate::export::analysis::ReportAnalysis,
    report_name: &str,
) {
    o.push_str("    <ParameterFieldDefinitions>\n");
    // The report's own parameters, as full definitions. InUse / DataFetching usage flags are
    // engine-derived (an aggregation, not stored in the file), keyed by parameter name in the
    // analysis computed above.
    for f in parameter_fields(report) {
        if let FieldKindData::Parameter(p) = &f.kind {
            let usage = analysis
                .parameter_usage
                .get(&f.name)
                .copied()
                .unwrap_or_default();
            write_parameter(
                o,
                &f.name,
                p,
                usage.in_use,
                usage.data_fetching,
                report_name,
            );
        }
    }
    // Subreport-link stubs. Linking a main-report field into a subreport auto-creates a parameter on
    // that subreport (named `Pm-<mainfield>`, or explicitly by SAP B1); the owning report then lists
    // each such linked parameter as a stub carrying only Name, IsLinkedToSubreport=True and the
    // subreport's ReportName. There is one `0x0106` link record per subreport parameter, so every
    // parameter of a linked subreport (one with `0x0106` records) is a stub. The stub Name is the
    // subreport's own parameter name, read from the subreport's parameter list (the link record
    // stores only the main-report field). Stubs are grouped by subreport in ascending name order;
    // within a subreport they follow its parameter-definition order.
    let mut linked: Vec<&rpt::model::Subreport> = report
        .subreports
        .iter()
        .filter(|s| !s.links.is_empty())
        .collect();
    linked.sort_by(|a, b| a.name.cmp(&b.name));
    for sub in linked {
        for fd in &sub.report.data_definition.field_definitions {
            if matches!(&fd.kind, FieldKindData::Parameter(_)) {
                let _ = writeln!(
                    o,
                    "      <ParameterFieldDefinition Name=\"{}\" IsLinkedToSubreport=\"True\" ReportName=\"{}\" />",
                    escape(&fd.name),
                    escape(&sub.name)
                );
            }
        }
    }
    o.push_str("    </ParameterFieldDefinitions>\n");
}

/// `<RunningTotalFieldDefinitions>` — decoded from the 0x80(reset)+0x7e(operation) record pairs.
fn write_running_totals(o: &mut String, report: &Report) {
    let rts: Vec<_> = running_total_fields(report).collect();
    write_collection(o, "    ", "RunningTotalFieldDefinitions", &rts, |o, f| {
        if let FieldKindData::RunningTotal(rt) = &f.kind {
            write_running_total(o, f, rt);
        }
    });
}

/// `<SummaryFields>` — the engine-derived list of placed summaries (deduped + ordered).
fn write_summary_fields(o: &mut String, analysis: &crate::export::analysis::ReportAnalysis) {
    write_collection(
        o,
        "    ",
        "SummaryFields",
        &analysis.summary_fields,
        |o, s| {
            let group = if s.grouped {
                " Group=\"CrystalDecisions.CrystalReports.Engine.Group\""
            } else {
                ""
            };
            let _ = writeln!(
            o,
            "      <SummaryFieldDefinition FormulaName=\"{nm}\"{group} Kind=\"{kind}\" Name=\"{nm}\" NumberOfBytes=\"{nb}\" Operation=\"{op}\" OperationParameter=\"0\" SummarizedField=\"{sf}\" ValueType=\"{vt}\" />",
            kind = FieldKind::SummaryField.name(),
            nm = escape(&s.formula_name),
            nb = s.number_of_bytes,
            op = escape(&s.operation),
            sf = s.summarized_field_type,
            vt = s.value_type,
        );
        },
    );
}

/// `<SubReportLinks>` — each link the parent established into this subreport, all three names
/// brace-wrapped: LinkedParameterName ({?param}), MainReportFieldName ({main field}) and
/// SubreportFieldName ({bound field}, or = LinkedParameterName when the parameter binds to no db
/// field). A linkless subreport still emits an empty self-closing element.
fn write_subreport_links(o: &mut String, incoming_links: &[rpt::model::SubreportLink]) {
    write_collection(o, "  ", "SubReportLinks", incoming_links, |o, link| {
        let lp = format!("{{?{}}}", link.linked_parameter.as_deref().unwrap_or(""));
        let mf = format!("{{{}}}", link.main_report_field);
        let sf = if link.subreport_field.is_empty() {
            lp.clone()
        } else {
            format!("{{{}}}", link.subreport_field)
        };
        let _ = writeln!(
            o,
            "    <SubReportLink LinkedParameterName=\"{}\" MainReportFieldName=\"{}\" SubreportFieldName=\"{}\" />",
            escape(&lp),
            escape(&mf),
            escape(&sf)
        );
    });
}
