//! L0 — the CFB/OLE2 compound-file container.
//!
//! `.rpt` files are Microsoft Compound File Binary documents. This layer is fully
//! documented and handled by the [`cfb`] crate; our job is to enumerate the streams,
//! classify them ([`StreamId`]), load their bytes, and parse the standard
//! `SummaryInformation` property set.
//!
//! The container reads every stream into memory at [`Container::open`] so the upper layers
//! (and `save`) own the bytes directly.

mod stream_id;

pub use stream_id::StreamId;

use std::io::{Cursor, Read};
use std::path::PathBuf;

use crate::bytes::{u16_le, u32_le};
use crate::error::{ContainerError, Result};

/// One loaded stream: its symbolic id, original OLE path, and raw bytes.
#[derive(Clone)]
pub(crate) struct LoadedStream {
    pub id: StreamId,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for LoadedStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedStream")
            .field("id", &self.id)
            .field("path", &self.path)
            .field("len", &self.bytes.len())
            .finish()
    }
}

/// An opened compound file with all of its streams loaded into memory.
#[derive(Debug)]
pub(crate) struct Container {
    streams: Vec<LoadedStream>,
}

impl Container {
    /// Open a compound file from raw bytes, enumerate every stream, and load it into memory.
    pub(crate) fn from_bytes(data: &[u8]) -> Result<Container> {
        let mut comp = cfb::CompoundFile::open(Cursor::new(data))
            .map_err(|e| ContainerError::new("open compound file", e.to_string()))?;

        // Collect stream paths first (walk borrows immutably; open_stream needs &mut).
        let paths: Vec<PathBuf> = comp
            .walk()
            .filter(|e| e.is_stream())
            .map(|e| e.path().to_path_buf())
            .collect();

        let mut streams = Vec::with_capacity(paths.len());
        for path in paths {
            let mut bytes = Vec::new();
            comp.open_stream(&path)
                .map_err(|e| {
                    ContainerError::new("open stream", e.to_string()).stream(path.display())
                })?
                .read_to_end(&mut bytes)
                .map_err(|e| {
                    ContainerError::new("read stream", e.to_string()).stream(path.display())
                })?;
            let id = StreamId::classify(&path);
            streams.push(LoadedStream { id, path, bytes });
        }

        Ok(Container { streams })
    }

    /// All loaded streams, in directory order.
    pub(crate) fn streams(&self) -> &[LoadedStream] {
        &self.streams
    }

    /// The bytes of the first stream matching `id`, if present.
    pub(crate) fn stream_bytes(&self, id: &StreamId) -> Option<&[u8]> {
        self.streams
            .iter()
            .find(|s| &s.id == id)
            .map(|s| s.bytes.as_slice())
    }

    /// Parse the `SummaryInformation` property set, if present.
    pub(crate) fn summary_info(&self) -> Option<SummaryInformation> {
        self.stream_bytes(&StreamId::SummaryInformation)
            .and_then(SummaryInformation::parse)
    }
}

/// Rewrite one stream of a compound file, copying every other stream verbatim. Opens `original`
/// (the whole `.rpt`), replaces the bytes of the first stream classifying as `target`, and returns
/// the re-written container. Used by the writer to splice a re-encoded `Contents` back into the
/// file. Errors if `original` is not a compound file or carries no stream matching `target`.
pub(crate) fn rewrite_stream(
    original: &[u8],
    target: &StreamId,
    new_bytes: &[u8],
) -> Result<Vec<u8>> {
    use std::io::{Seek, SeekFrom, Write};

    let mut comp = cfb::CompoundFile::open(Cursor::new(original.to_vec()))
        .map_err(|e| ContainerError::new("open compound file", e.to_string()))?;

    let path = comp
        .walk()
        .filter(|e| e.is_stream())
        .map(|e| e.path().to_path_buf())
        .find(|p| &StreamId::classify(p) == target)
        .ok_or_else(|| {
            ContainerError::new("find stream", "no matching stream in container")
                .stream(format!("{target:?}"))
        })?;

    {
        let mut stream = comp.open_stream(&path).map_err(|e| {
            ContainerError::new("open stream", e.to_string()).stream(path.display())
        })?;
        stream.set_len(new_bytes.len() as u64).map_err(|e| {
            ContainerError::new("resize stream", e.to_string()).stream(path.display())
        })?;
        stream.seek(SeekFrom::Start(0)).map_err(|e| {
            ContainerError::new("seek stream", e.to_string()).stream(path.display())
        })?;
        stream.write_all(new_bytes).map_err(|e| {
            ContainerError::new("write stream", e.to_string()).stream(path.display())
        })?;
        stream.flush().map_err(|e| {
            ContainerError::new("flush stream", e.to_string()).stream(path.display())
        })?;
    }
    comp.flush()
        .map_err(|e| ContainerError::new("flush compound file", e.to_string()))?;
    Ok(comp.into_inner().into_inner())
}

/// Load the `CONTENTS` stream bytes of `{storage_prefix}/Embedding {ordinal}` — the native image
/// data of a static/OLE picture. Path components are compared with OLE control-char prefixes
/// (`\x01`, `\x02`) stripped, so the `\x01Ole`/`CONTENTS` naming is matched robustly.
pub(crate) fn load_embedding_contents(
    container: &Container,
    storage_prefix: &str,
    ordinal: u32,
) -> Option<Vec<u8>> {
    let clean = |s: &str| -> String { s.chars().filter(|c| !c.is_control()).collect() };
    let want: Vec<String> = storage_prefix
        .split('/')
        .filter(|s| !s.is_empty())
        .map(clean)
        .chain([format!("Embedding {ordinal}"), "CONTENTS".to_owned()])
        .collect();
    container.streams().iter().find_map(|s| {
        let parts: Vec<String> = s
            .path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .filter(|c| !c.is_empty() && *c != "/" && *c != "\\")
            .map(clean)
            .collect();
        (parts == want).then(|| s.bytes.clone())
    })
}

/// Summarise embedded OLE objects: for each top-level `Embedding N` storage, hash each of its
/// OLE data streams into an [`Embed`](crate::model::Embed) (Name, byte size, Base64-MD5), in
/// directory order. The engine emits the OLE data streams — `Ole`, `OlePres000`, `Ole10Native` —
/// but not the `CompObj` (OLE class descriptor) or a `CONTENTS` sub-storage, so those are skipped.
pub(crate) fn raise_embeds(container: &Container) -> Vec<crate::model::Embed> {
    // Streams under an `Embedding N` storage that are not object data: `CompObj` is the OLE1
    // class-moniker blob, `CONTENTS` (when present) is a nested storage.
    const SKIP: [&str; 2] = ["CompObj", "CONTENTS"];
    let mut out = Vec::new();
    for s in container.streams() {
        // Path components below the root, e.g. `["Embedding 2", "\x01Ole"]`. Only top-level
        // embeddings count (a nested `Subdocument N/Embedding …` has three components).
        let parts: Vec<&str> = s
            .path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .filter(|c| !c.is_empty() && *c != "/" && *c != "\\")
            .collect();
        let [storage, stream] = parts.as_slice() else {
            continue;
        };
        // The stream name carries a `\x01`/`\x02` (OLE control) prefix; strip control chars for the Name.
        let name: String = stream.chars().filter(|c| !c.is_control()).collect();
        if storage.starts_with("Embedding ") && !SKIP.contains(&name.as_str()) {
            out.push(crate::model::Embed {
                name,
                size: s.bytes.len() as u64,
                md5_hash: crate::codec::md5_base64(&s.bytes),
            });
        }
    }
    out
}

/// The common, human-meaningful fields of the MS-OLEPS `SummaryInformation` property set.
///
/// Only the string properties relevant to a report are extracted; the full property set is
/// preserved verbatim in the container's stream bytes for round-trip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SummaryInformation {
    /// The report title (`PID_TITLE`).
    pub title: Option<String>,
    /// The report subject (`PID_SUBJECT`).
    pub subject: Option<String>,
    /// The report author (`PID_AUTHOR`).
    pub author: Option<String>,
    /// Keywords associated with the report (`PID_KEYWORDS`).
    pub keywords: Option<String>,
    /// Free-form comments (`PID_COMMENTS`).
    pub comments: Option<String>,
    /// The last author to save the report (`PID_LAST_AUTHOR`).
    pub last_author: Option<String>,
    /// The report revision number (`PID_REVNUMBER`) — stored as a string (e.g. `"128"`).
    pub revision_number: Option<String>,
    /// Whether a preview thumbnail (`PID_THUMBNAIL`) is stored — the engine's
    /// `SummaryInfo.IsSavingWithPreview`.
    pub has_thumbnail: bool,
}

// MS-OLEPS PropertyIdentifier values for the SummaryInformation property set.
const PID_TITLE: u32 = 0x02;
const PID_SUBJECT: u32 = 0x03;
const PID_AUTHOR: u32 = 0x04;
const PID_KEYWORDS: u32 = 0x05;
const PID_COMMENTS: u32 = 0x06;
const PID_LAST_AUTHOR: u32 = 0x08;
const PID_REVNUMBER: u32 = 0x09;
const PID_THUMBNAIL: u32 = 0x11;

const VT_LPSTR: u32 = 0x1E;
const VT_LPWSTR: u32 = 0x1F;

impl SummaryInformation {
    /// Best-effort parse of an OLEPS property-set stream. Returns `None` if the header is
    /// not a recognisable property set; unknown properties are ignored, never fatal.
    fn parse(data: &[u8]) -> Option<SummaryInformation> {
        // Property set header: byte-order(2)=FFFE, version(2), sysid(4), clsid(16),
        // num property sets(4), then [FMTID(16) + offset(4)] per set.
        if data.len() < 48 || u16_le(data, 0)? != 0xFFFE {
            return None;
        }
        if u32_le(data, 24)? < 1 {
            return None; // num property sets
        }
        let first_set_off = u32_le(data, 28 + 16)? as usize; // skip FMTID(16) of set 0
        let sect = data.get(first_set_off..)?;

        // Section: size(4), count(4), then count × (propid(4), value-offset(4)).
        let count = u32_le(sect, 4)? as usize;
        let mut info = SummaryInformation::default();
        for i in 0..count {
            let entry = 8 + i * 8;
            let pid = u32_le(sect, entry)?;
            let voff = u32_le(sect, entry + 4)? as usize;
            // The thumbnail is a (non-string) clipboard blob; only its presence matters.
            if pid == PID_THUMBNAIL {
                info.has_thumbnail = true;
                continue;
            }
            let Some(value) = read_string_property(sect, voff) else {
                continue;
            };
            match pid {
                PID_TITLE => info.title = Some(value),
                PID_SUBJECT => info.subject = Some(value),
                PID_AUTHOR => info.author = Some(value),
                PID_KEYWORDS => info.keywords = Some(value),
                PID_COMMENTS => info.comments = Some(value),
                PID_LAST_AUTHOR => info.last_author = Some(value),
                PID_REVNUMBER => info.revision_number = Some(value),
                _ => {}
            }
        }
        Some(info)
    }
}

/// An edited property-set stream, paired with the `(field, value)` pairs blanked out of it.
pub(crate) type ScrubbedPropertySet = (Vec<u8>, Vec<(&'static str, String)>);

/// Blank the identity properties (`PID_AUTHOR`, `PID_LAST_AUTHOR`) of an OLEPS property-set stream
/// **in place**, returning the edited stream bytes and the values removed.
///
/// The edit is deliberately **same-length**: each value's `vt` tag and declared length are left
/// alone and only its character bytes are overwritten with NULs, so every property offset and the
/// section size stay valid and the whole property set — including the thumbnail blob and any
/// property this crate does not model — survives byte-for-byte. A reader takes the value up to the
/// first NUL, so the property reads as an empty string. Rebuilding the section instead would mean
/// re-deriving every offset and re-serializing properties we do not understand.
///
/// Returns `None` if the stream is not a parseable property set, or `Some` with an empty removal
/// list if neither property is present or both are already blank.
pub(crate) fn scrub_identity_properties(data: &[u8]) -> Option<ScrubbedPropertySet> {
    if data.len() < 48 || u16_le(data, 0)? != 0xFFFE || u32_le(data, 24)? < 1 {
        return None;
    }
    let sect_off = u32_le(data, 28 + 16)? as usize;
    let count = u32_le(data.get(sect_off..)?, 4)? as usize;

    let mut out = data.to_vec();
    let mut removed = Vec::new();
    for i in 0..count {
        let entry = sect_off + 8 + i * 8;
        let pid = u32_le(data, entry)?;
        let label = match pid {
            PID_AUTHOR => "author",
            PID_LAST_AUTHOR => "last_saved_by",
            _ => continue,
        };
        let voff = sect_off + u32_le(data, entry + 4)? as usize;
        let Some(value) = read_string_property(data.get(sect_off..)?, voff - sect_off) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        // Body starts after the value's `vt`(4) + `len`(4). `len` counts characters for VT_LPSTR and
        // UTF-16 code units for VT_LPWSTR, so scale it to bytes before blanking.
        let vt = u32_le(data, voff)?;
        let units = u32_le(data, voff + 4)? as usize;
        let bytes = match vt {
            VT_LPSTR => units,
            VT_LPWSTR => units * 2,
            _ => continue,
        };
        let body = voff + 8;
        out.get_mut(body..body + bytes)?.fill(0);
        removed.push((label, value));
    }
    Some((out, removed))
}

/// Read a VT_LPSTR / VT_LPWSTR property at `off` within a section.
fn read_string_property(sect: &[u8], off: usize) -> Option<String> {
    let vt = u32_le(sect, off)?;
    let len = u32_le(sect, off + 4)? as usize;
    let body = sect.get(off + 8..)?;
    match vt {
        VT_LPSTR => {
            let raw = body.get(..len)?;
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            // Code-page string; decode as Latin-1 (lossless for the ASCII metadata we see).
            Some(raw[..end].iter().map(|&b| b as char).collect())
        }
        VT_LPWSTR => {
            let raw = body.get(..len * 2)?;
            let units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scrubber blanks exactly the two identity properties, leaves every other property
    /// readable, and does not change the stream's length — the same-length claim the whole
    /// anonymize path rests on.
    #[test]
    fn scrub_identity_properties_blanks_only_the_identity_fields() {
        let data = build_property_set(&[
            (PID_TITLE, "Quarterly Sales"),
            (PID_AUTHOR, "Ada Lovelace"),
            (PID_SUBJECT, "Sales"),
            (PID_LAST_AUTHOR, "Grace Hopper"),
            (PID_REVNUMBER, "7"),
        ]);
        let (edited, removed) =
            scrub_identity_properties(&data).expect("a built property set parses");

        assert_eq!(edited.len(), data.len(), "the edit must be same-length");
        assert_eq!(
            removed,
            vec![
                ("author", "Ada Lovelace".to_string()),
                ("last_saved_by", "Grace Hopper".to_string()),
            ]
        );

        let after = SummaryInformation::parse(&edited).expect("edited set still parses");
        assert_eq!(after.author.as_deref(), Some(""));
        assert_eq!(after.last_author.as_deref(), Some(""));
        // Everything around the blanked values is untouched, which is only possible because no
        // offset moved.
        assert_eq!(after.title.as_deref(), Some("Quarterly Sales"));
        assert_eq!(after.subject.as_deref(), Some("Sales"));
        assert_eq!(after.revision_number.as_deref(), Some("7"));
    }

    /// An already-clean set reports nothing to remove, so the caller can skip rewriting the stream.
    #[test]
    fn scrub_identity_properties_is_a_no_op_when_already_clean() {
        let data = build_property_set(&[(PID_TITLE, "T"), (PID_AUTHOR, "")]);
        let (edited, removed) = scrub_identity_properties(&data).expect("parses");
        assert!(removed.is_empty());
        assert_eq!(edited, data);
    }

    /// Build a minimal single-section OLEPS `SummaryInformation` property set carrying the given
    /// `(propid, VT_LPWSTR value)` entries, so the parser's property-id dispatch (incl.
    /// `PID_REVNUMBER`/`PID_LAST_AUTHOR`) is covered without any real report bytes.
    fn build_property_set(entries: &[(u32, &str)]) -> Vec<u8> {
        // Section body: size(4), count(4), count×(propid(4), value-offset(4)), then the values.
        let n = entries.len();
        let table_end = 8 + n * 8; // where the first value starts, section-relative
        let mut values = Vec::new();
        let mut offsets = Vec::new();
        for (_, s) in entries {
            offsets.push(table_end + values.len());
            values.extend_from_slice(&VT_LPWSTR.to_le_bytes());
            let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
            values.extend_from_slice(&(units.len() as u32).to_le_bytes());
            for u in units {
                values.extend_from_slice(&u.to_le_bytes());
            }
        }
        let mut sect = Vec::new();
        let size = table_end + values.len();
        sect.extend_from_slice(&(size as u32).to_le_bytes());
        sect.extend_from_slice(&(n as u32).to_le_bytes());
        for ((pid, _), voff) in entries.iter().zip(&offsets) {
            sect.extend_from_slice(&pid.to_le_bytes());
            sect.extend_from_slice(&(*voff as u32).to_le_bytes());
        }
        sect.extend_from_slice(&values);

        // Property-set header: byte-order(2)=FFFE, version(2), sysid(4), clsid(16),
        // numsets(4)=1, FMTID(16), first-section-offset(4). Section starts at 48.
        let mut data = Vec::new();
        data.extend_from_slice(&0xFFFEu16.to_le_bytes());
        data.extend_from_slice(&[0; 2]); // version
        data.extend_from_slice(&[0; 4]); // sysid
        data.extend_from_slice(&[0; 16]); // clsid
        data.extend_from_slice(&1u32.to_le_bytes()); // num property sets
        data.extend_from_slice(&[0; 16]); // FMTID of set 0
        data.extend_from_slice(&48u32.to_le_bytes()); // section offset
        assert_eq!(data.len(), 48);
        data.extend_from_slice(&sect);
        data
    }

    #[test]
    fn parses_revision_and_last_author() {
        // PID 0x08 = last author, PID 0x09 = revision number.
        let data = build_property_set(&[
            (PID_TITLE, "T"),
            (PID_LAST_AUTHOR, "usr"),
            (PID_REVNUMBER, "128"),
        ]);
        let info = SummaryInformation::parse(&data).expect("valid property set");
        assert_eq!(info.title.as_deref(), Some("T"));
        assert_eq!(info.last_author.as_deref(), Some("usr"));
        assert_eq!(info.revision_number.as_deref(), Some("128"));
    }

    #[test]
    fn absent_revision_is_none() {
        let info = SummaryInformation::parse(&build_property_set(&[(PID_TITLE, "T")])).unwrap();
        assert_eq!(info.revision_number, None);
        assert_eq!(info.last_author, None);
    }
}
