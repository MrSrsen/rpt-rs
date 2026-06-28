//! Top-level `<Report>` document assembly.

use std::fmt::Write as _;

use rpt::model::{FieldKind, FieldKindData, Report};

use crate::database::{
    formula_fields, parameter_fields, running_total_fields, write_database, write_parameter,
    write_running_total,
};
use crate::objects::{write_area, write_node};
use crate::util::{b, escape, escape_text, paper_size_str, paper_source_str, value_type_name};

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
    if report.embeds.is_empty() {
        o.push_str("  <Embedinfo />\n");
    } else {
        o.push_str("  <Embedinfo>\n");
        for e in &report.embeds {
            let _ = writeln!(
                o,
                "    <Embed Name=\"{}\" Size=\"{}\" MD5Hash=\"{}\" />",
                escape(&e.name),
                e.size,
                escape(&e.md5_hash)
            );
        }
        o.push_str("  </Embedinfo>\n");
    }

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
    if report.subreports.is_empty() {
        o.push_str("  <SubReports />\n");
    } else {
        o.push_str("  <SubReports>\n");
        for sub in &report.subreports {
            let _ = writeln!(o, "    <Report Name=\"{}\">", escape(&sub.name));
            // The shared body is written at the top-level (2-space) indent; a subreport sits two
            // levels deeper (inside <SubReports><Report>), so re-indent its lines by 4 spaces.
            let mut body = String::new();
            write_report_core(&mut body, &sub.report, &sub.links, true);
            for line in body.lines() {
                if line.is_empty() {
                    o.push('\n');
                } else {
                    let _ = writeln!(o, "    {line}");
                }
            }
            o.push_str("    </Report>\n");
        }
        o.push_str("  </SubReports>\n");
    }

    write_report_core(&mut o, report, &[], false);

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

/// The shared body of a `<Report>` (main or subreport): Database, DataDefinition, ReportDefinition.
fn write_report_core(
    o: &mut String,
    report: &Report,
    incoming_links: &[rpt::model::SubreportLink],
    is_subreport: bool,
) {
    // Derived analytics (field UseCount + parameter usage flags) come from the engine layer,
    // computed once per report level and threaded into the pure-formatting writers below.
    // `incoming_links` are the parent's subreport links targeting this level (empty for the main
    // report), used to mark link-fed parameters as `InUse`.
    let analysis = rpt_engine::analyze(report, incoming_links);

    // Database — tables (with the SQL Command + connection), table links, and the field
    // schema, all decoded from the QESession stream.
    write_database(o, report, &analysis.field_use_counts);

    // DataDefinition — selection formulas, groups, sort fields, formula and parameter fields.
    o.push_str("  <DataDefinition>\n");
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
    // A selection formula is emitted as the engine's canonical re-render when it is SQL-pushable,
    // else as the stored body verbatim (see `selection::render_selection`).
    let sel_text = |body: &str| {
        crate::selection::render_selection(body, &alias_canon).unwrap_or_else(|| body.to_string())
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
    if dd.groups.is_empty() {
        o.push_str("    <Groups />\n");
    } else {
        o.push_str("    <Groups>\n");
        for g in &dd.groups {
            let _ = writeln!(
                o,
                "      <Group ConditionField=\"{{{}}}\" />",
                escape(&g.condition_field)
            );
        }
        o.push_str("    </Groups>\n");
    }
    if dd.record_sorts.is_empty() {
        o.push_str("    <SortFields />\n");
    } else {
        o.push_str("    <SortFields>\n");
        for s in &dd.record_sorts {
            let _ = writeln!(
                o,
                "      <SortField Field=\"{{{}}}\" SortDirection=\"{:?}\" SortType=\"{:?}\" />",
                escape(&s.field),
                s.direction,
                s.kind
            );
        }
        o.push_str("    </SortFields>\n");
    }
    // Formula fields — the body is emitted as element text with newlines preserved so multi-line
    // formulas diff line-by-line.
    o.push_str("    <FormulaFieldDefinitions>\n");
    for f in formula_fields(report) {
        if let FieldKindData::Formula(ff) = &f.kind {
            // A formula referencing an undefined parameter/formula fails to compile at load → the
            // engine reports it as UnknownField/0 (see `rpt_engine::stale_formulas`); otherwise use
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
                "      <FormulaFieldDefinition FormulaName=\"{{@{}}}\" Kind=\"{}\" Name=\"{}\" NumberOfBytes=\"{}\" ValueType=\"{}\">{}</FormulaFieldDefinition>",
                escape(&f.name),
                f.kind.field_kind().name(),
                escape(&f.name),
                number_of_bytes,
                value_type,
                escape_text(&ff.text.0)
            );
        }
    }
    o.push_str("    </FormulaFieldDefinitions>\n");
    // GroupNameFieldDefinitions — one auto-generated per group, synthesised from the decoded groups.
    if dd.groups.is_empty() {
        o.push_str("    <GroupNameFieldDefinitions />\n");
    } else {
        o.push_str("    <GroupNameFieldDefinitions>\n");
        for (i, g) in dd.groups.iter().enumerate() {
            let field = &g.condition_field;
            // A date/time/boolean group's grouping period is appended as a title-cased string
            // operand: `GroupName ({field}, "Monthly")`. It appears in FormulaName/Name only — not
            // GroupNameFieldName. Built with the raw field, then XML-escaped as a whole (the literal
            // quotes around the period become `&quot;`).
            let group_name = match &g.date_condition {
                Some(c) => {
                    let mut ch = c.chars();
                    let cond = ch
                        .next()
                        .map(|f0| f0.to_ascii_uppercase().to_string() + ch.as_str())
                        .unwrap_or_default();
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
            write_parameter(o, &f.name, p, usage.in_use, usage.data_fetching);
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
    // RunningTotalFieldDefinitions — decoded from the 0x80(reset)+0x7e(operation) record pairs.
    let rts: Vec<_> = running_total_fields(report).collect();
    if rts.is_empty() {
        o.push_str("    <RunningTotalFieldDefinitions />\n");
    } else {
        o.push_str("    <RunningTotalFieldDefinitions>\n");
        for f in rts {
            if let FieldKindData::RunningTotal(rt) = &f.kind {
                write_running_total(o, f, rt);
            }
        }
        o.push_str("    </RunningTotalFieldDefinitions>\n");
    }
    // SQLExpressionFields — always emitted; members not decoded yet.
    o.push_str("    <SQLExpressionFields />\n");
    // SummaryFields — the engine-derived list of placed summaries (deduped + ordered).
    if analysis.summary_fields.is_empty() {
        o.push_str("    <SummaryFields />\n");
    } else {
        o.push_str("    <SummaryFields>\n");
        for s in &analysis.summary_fields {
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
        }
        o.push_str("    </SummaryFields>\n");
    }
    o.push_str("  </DataDefinition>\n");
    // CustomFunctions sits between DataDefinition and ReportDefinition (not decoded yet).
    o.push_str("  <CustomFunctions />\n");

    // SubReportLinks — emitted by every subreport (only), between CustomFunctions and
    // ReportDefinition. Each link the parent established into this subreport is listed here, all
    // three names brace-wrapped: LinkedParameterName ({?param}), MainReportFieldName ({main field})
    // and SubreportFieldName ({bound field}, or = LinkedParameterName when the parameter binds to no
    // db field). A linkless subreport still emits an empty self-closing element. The main report
    // never emits it.
    if is_subreport {
        if incoming_links.is_empty() {
            o.push_str("  <SubReportLinks />\n");
        } else {
            o.push_str("  <SubReportLinks>\n");
            for link in incoming_links {
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
            }
            o.push_str("  </SubReportLinks>\n");
        }
    }

    // ReportDefinition — areas / sections / objects.
    o.push_str("  <ReportDefinition>\n    <Areas>\n");
    for area in &report.report_definition.areas {
        write_area(o, area, 3);
    }
    o.push_str("    </Areas>\n  </ReportDefinition>\n");
}
