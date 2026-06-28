//! Symbolic identity for the streams inside an `.rpt` compound file, so higher layers
//! address streams by meaning rather than by raw OLE path.
//!
//! Classification is **total**: anything unrecognised falls through to [`StreamId::Other`]
//! carrying its full path, so no stream is ever dropped.

use std::path::Path;

/// The MS-OLEPS summary-information stream name (`\x05SummaryInformation`).
pub(crate) const SUMMARY_INFORMATION: &str = "\u{5}SummaryInformation";

/// A classified stream within the compound file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StreamId {
    /// The report definition — the primary TSLV record stream.
    Contents,
    /// MS-OLEPS property set: title, author, subject, etc.
    SummaryInformation,
    /// Small fixed-size report metadata stream.
    ReportInfo,
    /// Parameter/prompt state.
    PromptManager,
    /// Saved query-engine session (often zlib-compressed saved data).
    QESession,
    /// The Crystal designer's private stream.
    DesignerStream,
    /// A subreport, stored as its own storage `Subdocument N`. Holds the index `N`.
    Subdocument(u32),
    /// An OLE embedding (`Embedding N` storage and the streams beneath it). Full path kept.
    Embedding(String),
    /// A chart cache stream (`CHART …`). Full name kept.
    Chart(String),
    /// Export-format options (`ExportFormatOptionsStream …`). Full name kept.
    ExportFormatOptions(String),
    /// A zlib-compressed BLOB (`zlibBLOB …`). Full name kept.
    ZlibBlob(String),
    /// Any stream we do not (yet) classify — carries the full OLE path verbatim.
    Other(String),
}

impl StreamId {
    /// Classify an OLE entry by its full path within the compound file.
    pub(crate) fn classify(path: &Path) -> StreamId {
        let comps: Vec<String> = path
            .components()
            .filter_map(|c| c.as_os_str().to_str().map(str::to_owned))
            .filter(|s| !s.is_empty() && s != "/")
            .collect();

        let Some(top) = comps.first() else {
            return StreamId::Other(String::new());
        };

        // Nested entries (anything below a top-level storage) keep their full path.
        if comps.len() > 1 {
            if top.starts_with("Embedding") {
                return StreamId::Embedding(join(&comps));
            }
            return StreamId::Other(join(&comps));
        }

        match top.as_str() {
            "Contents" => StreamId::Contents,
            SUMMARY_INFORMATION => StreamId::SummaryInformation,
            "ReportInfo" => StreamId::ReportInfo,
            "PromptManager" => StreamId::PromptManager,
            "QESession" => StreamId::QESession,
            "CrystalReportDesignerStream" => StreamId::DesignerStream,
            _ => classify_indexed(top),
        }
    }

    /// True for the TSLV streams the codec layer decodes (currently just `Contents`).
    pub(crate) fn is_tslv(&self) -> bool {
        matches!(self, StreamId::Contents)
            || matches!(self, StreamId::Other(n) if n.starts_with("ReportParametersStream"))
    }
}

fn classify_indexed(name: &str) -> StreamId {
    if let Some(n) = name.strip_prefix("Subdocument ") {
        if let Ok(idx) = n.trim().parse::<u32>() {
            return StreamId::Subdocument(idx);
        }
    }
    if name.starts_with("Embedding") {
        return StreamId::Embedding(name.to_owned());
    }
    if name.starts_with("CHART") {
        return StreamId::Chart(name.to_owned());
    }
    if name.starts_with("ExportFormatOptionsStream") {
        return StreamId::ExportFormatOptions(name.to_owned());
    }
    if name.starts_with("zlibBLOB") {
        return StreamId::ZlibBlob(name.to_owned());
    }
    StreamId::Other(name.to_owned())
}

fn join(comps: &[String]) -> String {
    comps.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn id(p: &str) -> StreamId {
        StreamId::classify(&PathBuf::from(p))
    }

    #[test]
    fn classifies_top_level_streams() {
        assert_eq!(id("/Contents"), StreamId::Contents);
        assert_eq!(id("/ReportInfo"), StreamId::ReportInfo);
        assert_eq!(id("/PromptManager"), StreamId::PromptManager);
        assert_eq!(id("/QESession"), StreamId::QESession);
        assert_eq!(id("/CrystalReportDesignerStream"), StreamId::DesignerStream);
    }

    #[test]
    fn classifies_summary_information_with_control_prefix() {
        assert_eq!(
            id(&format!("/{SUMMARY_INFORMATION}")),
            StreamId::SummaryInformation
        );
    }

    #[test]
    fn classifies_indexed_streams() {
        assert_eq!(id("/Subdocument 13"), StreamId::Subdocument(13));
        assert_eq!(id("/CHART 82l"), StreamId::Chart("CHART 82l".into()));
        assert_eq!(
            id("/zlibBLOB 314l"),
            StreamId::ZlibBlob("zlibBLOB 314l".into())
        );
    }

    #[test]
    fn nested_embedding_keeps_full_path() {
        assert_eq!(
            id("/Embedding 1/CONTENTS"),
            StreamId::Embedding("Embedding 1/CONTENTS".into())
        );
    }
}
