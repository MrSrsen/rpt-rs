//! `inspect` — a one-screen report + per-stream summary.

use rpt::model::{Report, ReportObjectKind};
use rpt::Rpt;
use serde::Serialize;

use crate::util::print_json;

pub(crate) const HELP: &str = "\
rpt inspect — one-screen report + per-stream summary

Report version, summary-info (title / author / subject), record + field counts, the most common
record types, each chart's data binding (value + category fields), and a per-stream listing (size,
encryption, IV length, version).

USAGE:
    rpt inspect <file.rpt> [--json]

OPTIONS:
    --json    emit the summary as JSON
";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryJson {
    title: Option<String>,
    author: Option<String>,
    subject: Option<String>,
}

#[derive(Serialize)]
struct TopType {
    #[serde(rename = "type")]
    name: String,
    count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldJson {
    name: String,
    value_type: String,
}

/// A chart object's decoded data binding — which report fields feed its values and categories. The
/// XML export intentionally emits a bare `<Chart>` (mirroring RptToXml, which never emits chart
/// internals), so this is the only human-facing surface for verifying a chart's bindings during
/// report authoring.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChartBindingJson {
    /// The chart object's name.
    name: String,
    /// The section the chart is placed in.
    section: String,
    /// The subreport the chart lives in, or `None` for a main-report chart.
    #[serde(skip_serializing_if = "Option::is_none")]
    subreport: Option<String>,
    /// The decoded visual type (`Bar`, `Line`, …).
    graph_type: String,
    /// The "Show value(s)" data bindings (the summarized value fields).
    data_refs: Vec<String>,
    /// The "On change of" category bindings (the group/category fields).
    category_refs: Vec<String>,
    /// The per-axis gridline modes (`group=<mode> value=<mode>`), for the axis families only —
    /// omitted for the Pie/Doughnut/Funnel/Gauge families, which carry no axes.
    #[serde(skip_serializing_if = "Option::is_none")]
    gridlines: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamHeadJson {
    id: String,
    bytes: usize,
    encrypted: bool,
    version: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectReport<'a> {
    file: &'a str,
    version: u16,
    record_count: usize,
    distinct_record_types: usize,
    summary_info: SummaryJson,
    top_record_types: Vec<TopType>,
    fields: Vec<FieldJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    charts: Vec<ChartBindingJson>,
    streams: Vec<StreamHeadJson>,
}

/// Format an axis chart's per-axis gridline modes as `group=<mode> value=<mode>`, or `None` for the
/// axis-less families (Pie/Doughnut/Funnel/Gauge), whose gridlines stay at the `None` default.
fn chart_gridlines(def: &rpt::model::ChartDefinition) -> Option<String> {
    use rpt::model::ChartGridType;
    if def.group_axis_gridlines == ChartGridType::None
        && def.value_axis_gridlines == ChartGridType::None
    {
        return None;
    }
    Some(format!(
        "group={:?} value={:?}",
        def.group_axis_gridlines, def.value_axis_gridlines
    ))
}

/// Collect every chart object's data binding from `report` and its nested subreports (recursively),
/// tagging each with `subreport` when it lives inside one.
fn collect_charts(report: &Report, subreport: Option<&str>, out: &mut Vec<ChartBindingJson>) {
    for area in &report.report_definition.areas {
        for sec in &area.sections {
            for obj in &sec.objects {
                if let ReportObjectKind::Chart(c) = &obj.kind {
                    out.push(ChartBindingJson {
                        name: obj.name.clone(),
                        section: sec.name.clone(),
                        subreport: subreport.map(str::to_string),
                        graph_type: format!("{:?}", c.definition.graph_type),
                        data_refs: c.data_refs.clone(),
                        category_refs: c.category_refs.clone(),
                        gridlines: chart_gridlines(&c.definition),
                    });
                }
            }
        }
    }
    for sub in &report.subreports {
        collect_charts(&sub.report, Some(&sub.name), out);
    }
}

pub(crate) fn inspect(file: &str, json: bool) -> rpt::Result<()> {
    let rpt = Rpt::open(file)?;
    let si = rpt.summary_info();
    let report = rpt.report();
    let top: Vec<(String, usize)> = report
        .record_inventory
        .iter()
        .take(8)
        .map(|e| match e.name {
            Some(n) => (n.to_string(), e.count),
            None => (format!("{:#06x}", e.tag), e.count),
        })
        .collect();
    let fields = &report.data_definition.field_definitions;
    let mut charts = Vec::new();
    collect_charts(report, None, &mut charts);

    if json {
        print_json(&InspectReport {
            file,
            version: report.version,
            record_count: report.record_count(),
            distinct_record_types: report.distinct_record_types(),
            summary_info: SummaryJson {
                title: si.and_then(|s| s.title.clone()),
                author: si.and_then(|s| s.author.clone()),
                subject: si.and_then(|s| s.subject.clone()),
            },
            top_record_types: top
                .iter()
                .map(|(n, c)| TopType {
                    name: n.clone(),
                    count: *c,
                })
                .collect(),
            fields: fields
                .iter()
                .map(|f| FieldJson {
                    name: f.name.clone(),
                    value_type: format!("{:?}", f.value_type),
                })
                .collect(),
            charts,
            streams: rpt
                .streams()
                .map(|(id, s)| StreamHeadJson {
                    id: format!("{id:?}"),
                    bytes: s.raw_bytes().len(),
                    encrypted: s.header().is_some_and(|h| h.is_encrypted),
                    version: s.header().map_or(0, |h| h.version),
                })
                .collect(),
        });
        return Ok(());
    }

    println!("file: {file}");
    if let Some(si) = si {
        if let Some(t) = &si.title {
            println!("title:  {t}");
        }
        if let Some(a) = &si.author {
            println!("author: {a}");
        }
        if let Some(s) = &si.subject {
            println!("subject: {s}");
        }
    }
    println!(
        "report: version={} · {} records · {} distinct types",
        report.version,
        report.record_count(),
        report.distinct_record_types()
    );
    let top_str: Vec<String> = top.iter().map(|(n, c)| format!("{n}×{c}")).collect();
    println!("  top record types: {}", top_str.join("  "));
    if !fields.is_empty() {
        let desc: Vec<String> = fields
            .iter()
            .map(|f| format!("{}:{:?}", f.name, f.value_type))
            .collect();
        println!("  fields ({}): {}", desc.len(), desc.join(", "));
    }
    if !charts.is_empty() {
        println!("charts ({}):", charts.len());
        for c in &charts {
            let loc = match &c.subreport {
                Some(s) => format!("{s}/{}", c.section),
                None => c.section.clone(),
            };
            let fmt_refs = |r: &[String]| {
                if r.is_empty() {
                    "(reuses report group)".to_string()
                } else {
                    r.join(", ")
                }
            };
            println!("  {} [{}] in {loc}", c.name, c.graph_type);
            println!("      values:     {}", fmt_refs(&c.data_refs));
            println!("      categories: {}", fmt_refs(&c.category_refs));
            if let Some(g) = &c.gridlines {
                println!("      gridlines:  {g}");
            }
        }
    }
    println!("streams:");
    for (id, stream) in rpt.streams() {
        let hdr = match stream.header() {
            Some(h) => format!(
                " [enc={} iv={}B ver={}]",
                h.is_encrypted,
                h.iv.len(),
                h.version
            ),
            None => String::new(),
        };
        println!(
            "  {:<28} {:>6} bytes{hdr}",
            format!("{id:?}"),
            stream.raw_bytes().len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // The builders below construct `rpt` model structs via `Default` + field assignment rather than
    // full struct literals (which would spell out every nested chart/definition field); the lint
    // cannot offer a useful rewrite here, so it is allowed for this test module.
    #![allow(clippy::field_reassign_with_default)]

    use super::*;
    use rpt::model::{
        Area, ChartObject, Report, ReportObject, ReportObjectKind, Section, Subreport,
    };

    fn chart_obj(name: &str, data: &[&str], cats: &[&str]) -> ReportObject {
        let mut c = ChartObject::default();
        c.data_refs = data.iter().map(|s| s.to_string()).collect();
        c.category_refs = cats.iter().map(|s| s.to_string()).collect();
        let mut o = ReportObject::default();
        o.name = name.to_string();
        o.kind = ReportObjectKind::Chart(Box::new(c));
        o
    }

    fn report_with_objects(section: &str, objects: Vec<ReportObject>) -> Report {
        let mut sec = Section::default();
        sec.name = section.to_string();
        sec.objects = objects;
        let mut area = Area::default();
        area.sections = vec![sec];
        let mut report = Report::default();
        report.report_definition.areas = vec![area];
        report
    }

    #[test]
    fn collect_charts_surfaces_main_and_subreport_bindings() {
        let mut report = report_with_objects(
            "ReportHeaderSection1",
            vec![chart_obj(
                "Graph1",
                &["Sum of {orders.id}"],
                &["{orders.created_at}"],
            )],
        );
        // A subreport carrying its own chart is surfaced too, tagged with the subreport name.
        let sub_report = report_with_objects(
            "DetailSection1",
            vec![chart_obj("SubGraph", &["Count of {items.sku}"], &[])],
        );
        let mut sub = Subreport::default();
        sub.name = "Sub1".into();
        sub.report = Box::new(sub_report);
        report.subreports = vec![sub];

        let mut charts = Vec::new();
        collect_charts(&report, None, &mut charts);
        assert_eq!(charts.len(), 2, "one main + one subreport chart");

        let main = &charts[0];
        assert_eq!(main.name, "Graph1");
        assert_eq!(main.section, "ReportHeaderSection1");
        assert_eq!(main.subreport, None);
        assert_eq!(main.graph_type, "Bar");
        assert_eq!(main.data_refs, vec!["Sum of {orders.id}"]);
        assert_eq!(main.category_refs, vec!["{orders.created_at}"]);

        let subchart = &charts[1];
        assert_eq!(subchart.name, "SubGraph");
        assert_eq!(subchart.subreport.as_deref(), Some("Sub1"));
        assert_eq!(subchart.data_refs, vec!["Count of {items.sku}"]);
        assert!(
            subchart.category_refs.is_empty(),
            "reused-group chart has no category ref"
        );
    }

    #[test]
    fn collect_charts_ignores_non_chart_objects() {
        let report = report_with_objects("D", vec![ReportObject::default()]);
        let mut charts = Vec::new();
        collect_charts(&report, None, &mut charts);
        assert!(charts.is_empty(), "a default (text) object is not a chart");
    }
}
