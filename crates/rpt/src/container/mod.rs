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
use crate::error::{Error, Result};

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
            .map_err(|e| Error::container(format!("open compound file: {e}")))?;

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
                .map_err(|e| Error::container(format!("open stream {path:?}: {e}")))?
                .read_to_end(&mut bytes)
                .map_err(|e| Error::container(format!("read stream {path:?}: {e}")))?;
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

/// The common, human-meaningful fields of the MS-OLEPS `SummaryInformation` property set.
///
/// Only the string properties relevant to a report are extracted; the full property set is
/// preserved verbatim in the container's stream bytes for round-trip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SummaryInformation {
    pub title: Option<String>,
    pub subject: Option<String>,
    pub author: Option<String>,
    pub keywords: Option<String>,
    pub comments: Option<String>,
    pub last_author: Option<String>,
    /// Whether a preview thumbnail (`PID_THUMBNAIL`) is stored — the engine's
    /// `SummaryInfo.IsSavingWithPreview` (XML `EnableSavePreviewPicture`).
    pub has_thumbnail: bool,
}

// MS-OLEPS PropertyIdentifier values for the SummaryInformation property set.
const PID_TITLE: u32 = 0x02;
const PID_SUBJECT: u32 = 0x03;
const PID_AUTHOR: u32 = 0x04;
const PID_KEYWORDS: u32 = 0x05;
const PID_COMMENTS: u32 = 0x06;
const PID_LAST_AUTHOR: u32 = 0x08;
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
                _ => {}
            }
        }
        Some(info)
    }
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
