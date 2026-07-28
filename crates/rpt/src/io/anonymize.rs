//! Strip personally identifying authoring metadata from a report, producing clean `.rpt` bytes.
//!
//! Reports carry who made them and where: the OLE `SummaryInformation` property set records the
//! author and the last person to save, and a re-imported subreport records the full path of the
//! `.rpt` it was imported from. None of it affects how the report renders, and all of it leaks a
//! real person and a real machine layout into any corpus the file is committed to.
//!
//! **Every edit is same-length.** A value's length prefix is left exactly as it was and only its
//! character bytes are overwritten — with the replacement text where there is one, then NULs to the
//! original width. That is the whole safety argument: no record length, no enclosing record length,
//! no property offset and no section size ever changes, so nothing this crate does not model can be
//! disturbed. Readers of both affected formats stop a string at its first NUL — the `0x0142` decoder
//! trims trailing NULs, and an OLEPS string property is NUL-terminated — so the padding is invisible.
//!
//! The author and last-saver are blanked outright; they are identity and nothing else. The re-import
//! path is **reduced to its file name** rather than blanked, because it is the only evidence in the
//! file that a subreport was imported at all — `SubreportObject.IsImported` is resolved from it, so
//! emptying it would silently turn a genuine fact false. The directory prefix is what identifies a
//! person and a machine (`\\HOST\user\Documents\...`); the bare file name is the subreport's own
//! name, which the `Subdocument` storage already records anyway.
//!
//! The container is rebuilt stream by stream: an affected TSLV stream is decoded to its logical
//! bytes, edited, re-encoded, and spliced back with `rewrite_stream`. A stream with nothing to
//! remove is not touched at all, so an already-clean report round-trips byte-identically.

use crate::container::{rewrite_stream, scrub_identity_properties, Container};
use crate::error::Result;
use crate::records::RecordStream;
use crate::{Rpt, StreamId};

use super::patch::patch_leaf_region;

/// The record type carrying a re-imported subreport's source path.
const REIMPORT_INFO: u16 = 0x0142;

/// One piece of identifying metadata removed by [`Rpt::anonymize`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// What was removed, as a stable field name: `author`, `last_saved_by`, or
    /// `reimport.source_path`.
    pub field: &'static str,
    /// What the value was replaced with — empty for a blanked field, the retained file name for a
    /// re-import path.
    pub replacement: String,
    /// The stream it was removed from (`Contents`, `SummaryInformation`, `Subdocument 1/Contents`).
    pub stream: String,
    /// The value that was there — reported so a caller can show what it discarded. It is gone from
    /// the returned bytes.
    pub value: String,
}

/// What [`Rpt::anonymize`] removed. An empty [`Self::removals`] means the report carried none of the
/// metadata this pass covers, and the returned bytes are the input unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnonymizeReport {
    /// Every value removed, in the order the streams were processed.
    pub removals: Vec<Removal>,
}

impl AnonymizeReport {
    /// Whether anything was removed.
    pub fn is_empty(&self) -> bool {
        self.removals.is_empty()
    }
}

impl Rpt {
    /// Remove personally identifying authoring metadata and return the new `.rpt` file bytes,
    /// together with a report of exactly what was removed.
    ///
    /// Covers the author and last-saver in the `SummaryInformation` property set (blanked), and the
    /// re-import source path (`0x0142`) of the main report and of every subreport (reduced to its
    /// file name, so `IsImported` survives — the module docs explain why). Every edit is same-length,
    /// so the result is a structurally identical file that the engine and this reader both open
    /// normally, and the decoded model is unchanged apart from those fields. A report with nothing to
    /// remove comes back byte-identical.
    ///
    /// The database connection's stored path is **not** touched: it is a live datasource locator, not
    /// authoring metadata, and blanking it would break the report against its own data.
    ///
    /// # Errors
    ///
    /// Returns [`Err`] if the container cannot be reopened, a stream cannot be re-encoded, or a
    /// rewritten stream cannot be spliced back. A stream whose payload does not decode is skipped
    /// rather than fatal — it has nothing readable to scrub.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rpt::Rpt;
    ///
    /// let rpt = Rpt::open("report.rpt")?;
    /// let (bytes, removed) = rpt.anonymize()?;
    /// for r in &removed.removals {
    ///     println!("{}: removed {} ({:?})", r.stream, r.field, r.value);
    /// }
    /// std::fs::write("clean.rpt", bytes)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn anonymize(&self) -> Result<(Vec<u8>, AnonymizeReport)> {
        let mut bytes = self.original_bytes().to_vec();
        let mut report = AnonymizeReport::default();

        // The property set is plain bytes (neither encrypted nor deflated), so it is edited directly.
        let summary = Container::from_bytes(&bytes)?
            .stream_bytes(&StreamId::SummaryInformation)
            .map(<[u8]>::to_vec);
        if let Some(raw) = summary {
            if let Some((edited, removed)) = scrub_identity_properties(&raw) {
                if !removed.is_empty() {
                    for (field, value) in removed {
                        report.removals.push(Removal {
                            field,
                            stream: "SummaryInformation".to_string(),
                            value,
                            replacement: String::new(),
                        });
                    }
                    bytes = rewrite_stream(&bytes, &StreamId::SummaryInformation, &edited)?;
                }
            }
        }

        // The re-import path lives in a `0x0142` leaf, in the main `Contents` and in each
        // `Subdocument N/Contents`. Re-open the container each pass: every splice yields a fresh
        // image, so ids collected from a stale one could point into the wrong bytes.
        for (id, label) in tslv_streams(&bytes)? {
            let Some(raw) = Container::from_bytes(&bytes)?
                .stream_bytes(&id)
                .map(<[u8]>::to_vec)
            else {
                continue;
            };
            // Every one of these streams uses the `Contents` dialect, whatever its path.
            let stream = RecordStream::decode(StreamId::Contents, &raw);
            if stream.logical_bytes().is_empty() {
                continue;
            }
            let mut logical = stream.logical_bytes().to_vec();
            let removed = shorten_reimport_paths(&stream, &mut logical)?;
            if removed.is_empty() {
                continue;
            }
            for (value, replacement) in removed {
                report.removals.push(Removal {
                    field: "reimport.source_path",
                    stream: label.clone(),
                    value,
                    replacement,
                });
            }
            let new_raw = crate::codec::encode_contents(&raw, &logical)?;
            bytes = rewrite_stream(&bytes, &id, &new_raw)?;
        }

        Ok((bytes, report))
    }
}

/// Every `Contents`-dialect TSLV stream in the container, as `(id, display label)`: the report's own
/// `Contents` and one per `Subdocument N` storage. Subdocument streams classify to
/// [`StreamId::Other`] carrying their full path, which is both how `rewrite_stream` finds them and
/// what the label reports.
fn tslv_streams(bytes: &[u8]) -> Result<Vec<(StreamId, String)>> {
    let container = Container::from_bytes(bytes)?;
    let mut out = vec![(StreamId::Contents, "Contents".to_string())];
    for s in container.streams() {
        if let StreamId::Other(name) = &s.id {
            if name.starts_with("Subdocument ") && name.ends_with("/Contents") {
                out.push((s.id.clone(), name.clone()));
            }
        }
    }
    Ok(out)
}

/// Reduce the source path of every `0x0142` record in `logical` to its file name, returning the
/// `(original, replacement)` pairs changed.
///
/// The leaf is `[u32 BE len][path, len bytes including its NUL][fixed 17-byte trailer]`. Only the
/// path bytes are rewritten — the file name, then NULs to the original width — through
/// [`patch_leaf_region`] so the record's stack mask is reapplied. `len` and the trailer are untouched,
/// so the leaf keeps its width and the decoder (which trims trailing NULs) reads the file name back.
fn shorten_reimport_paths(
    stream: &RecordStream,
    logical: &mut [u8],
) -> Result<Vec<(String, String)>> {
    let mut targets = Vec::new();
    let tree = stream.record_tree();
    for root in &tree {
        root.walk(&mut |node| {
            if node.rtype != REIMPORT_INFO {
                return;
            }
            let leaf = node.leaf_bytes(logical);
            let Some(len) = leaf
                .get(..4)
                .and_then(|b| b.try_into().ok())
                .map(|b| u32::from_be_bytes(b) as usize)
            else {
                return;
            };
            // A length of 1 is the empty path (its NUL alone) — nothing to remove.
            if len <= 1 || 4 + len > leaf.len() {
                return;
            }
            let path = String::from_utf8_lossy(&leaf[4..4 + len])
                .trim_end_matches('\0')
                .to_string();
            // Already just a file name: nothing identifying left to strip.
            let name = file_name(&path);
            if !path.is_empty() && name != path {
                targets.push((node, len, path, name));
            }
        });
    }
    let mut removed = Vec::with_capacity(targets.len());
    for (node, len, path, name) in targets {
        // The file name is a suffix of the path, so it always fits; the rest becomes NUL padding.
        let mut new_path = vec![0u8; len];
        new_path[..name.len()].copy_from_slice(name.as_bytes());
        patch_leaf_region(node, logical, 4, &new_path)?;
        removed.push((path, name));
    }
    Ok(removed)
}

/// The file-name component of a stored Windows path (`\\HOST\u\dir\r.rpt` -> `r.rpt`). Both
/// separators are honoured: the format stores backslashes, but a path copied from elsewhere may not.
fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}
