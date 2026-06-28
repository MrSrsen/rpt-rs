//! L1 — the record read state machine.
//!
//! Models the byte cursor, the running XOR [`Mask`] (initial `0`), and the record-info
//! stack, so `load_block` performs the record-spanning, demasked reads that reconstruct the
//! logical content of the stream. `load_stream_header` extracts the per-stream IV.

use super::header::StreamHeader;
use super::mask::Mask;
use super::tslv::{self, Flags};
use crate::error::{Error, Result};

/// A record on the read stack.
#[derive(Debug, Clone)]
struct RecordInfo {
    /// Cursor position where this record's content begins.
    start: usize,
    /// Declared content length (may grow via record-extension in `load_block`).
    length: u64,
    #[allow(dead_code)] // read by the write path
    rtype: u16,
    /// Length-encoding kind (0/1/2/4), controls the record-extension block size.
    len_kind: u8,
}

/// A parsed TSLV record header.
#[derive(Debug, Clone)]
pub(crate) struct ParsedHeader {
    pub rtype: u16,
    #[allow(dead_code)]
    pub subtype: Option<u16>,
    pub length: u64,
    pub len_kind: u8,
}

/// The read state machine over one stream's bytes.
#[derive(Debug)]
pub(crate) struct ReadArchive<'a> {
    d: &'a [u8],
    pos: usize,
    mask: Mask,
    stack: Vec<RecordInfo>,
    /// Suppress record-extension while parsing a header.
    in_header_parse: bool,
}

impl<'a> ReadArchive<'a> {
    pub(crate) fn new(data: &'a [u8]) -> ReadArchive<'a> {
        ReadArchive {
            d: data,
            pos: 0,
            mask: Mask::INITIAL,
            stack: Vec::new(),
            in_header_parse: false,
        }
    }

    #[allow(dead_code)] // diagnostic accessor used by tests
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    pub(crate) fn at_end(&self) -> bool {
        self.pos >= self.d.len()
    }

    // -- low level -----------------------------------------------------------

    fn raw(&mut self, n: usize) -> Result<Vec<u8>> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.d.len())
            .ok_or_else(|| Error::codec(format!("read past end of stream at {}", self.pos)))?;
        let out = self.d[self.pos..end].to_vec();
        self.pos = end;
        Ok(out)
    }

    fn n_bytes_left_in_record(&self) -> u64 {
        let Some(top) = self.stack.last() else {
            return 0;
        };
        let used = self.pos as i64 - top.start as i64;
        if used < 0 {
            top.length
        } else {
            top.length.saturating_sub(used as u64)
        }
    }

    /// Read `n` bytes (extending the top record if needed), then XOR the mask.
    fn load_block(&mut self, n: usize) -> Result<Vec<u8>> {
        if !self.in_header_parse && !self.stack.is_empty() {
            let block: u64 = if self.stack.last().unwrap().len_kind == 1 {
                0x100
            } else {
                0x10000
            };
            while self.n_bytes_left_in_record() < n as u64 {
                self.stack.last_mut().unwrap().length += block;
            }
        }
        let mut out = self.raw(n)?;
        self.mask.apply(&mut out);
        Ok(out)
    }

    fn load_short(&mut self) -> Result<u16> {
        let b = self.load_block(2)?;
        Ok(((b[1] as u16) << 8) | b[0] as u16) // big-endian on disk
    }

    // -- TSLV header parsing -------------------------------------------------

    /// Parse a bit-packed record header and advance the mask.
    pub(crate) fn load_tslv_header(&mut self) -> Result<ParsedHeader> {
        let prev = self.in_header_parse;
        self.in_header_parse = true;
        let result = self.load_tslv_header_inner();
        self.in_header_parse = prev;
        result
    }

    fn load_tslv_header_inner(&mut self) -> Result<ParsedHeader> {
        let fwv = self.load_block(2)?;
        let mut fw = [fwv[0], fwv[1]];
        let flags = Flags::decode(&fw);

        let rtype = if flags.extended_value {
            let v = self.load_block(2)?;
            ((v[1] as u16) << 8) | v[0] as u16 // byte-swap → big-endian
        } else {
            // Inline type: the cleared flag word, read big-endian — record `f8 64` → type
            // `0x0064` (mask advances to 0x64), not the little-endian `0x6400`.
            tslv::clear_flag_bits(&mut fw);
            ((fw[0] as u16) << 8) | fw[1] as u16
        };

        let subtype = if flags.extended_type {
            let st = self.load_block(2)?;
            Some(((st[1] as u16) << 8) | st[0] as u16)
        } else {
            None
        };

        let length = if flags.len_kind != 0 {
            // Length is big-endian on disk; load_block yields disk order.
            let lb = self.load_block(flags.len_kind as usize)?;
            tslv::be_scalar(&lb)
        } else {
            0
        };

        self.mask.advance(rtype);
        Ok(ParsedHeader {
            rtype,
            subtype,
            length,
            len_kind: flags.len_kind,
        })
    }

    /// Parse headers (pushing record info) until `want_type`.
    pub(crate) fn next_record(&mut self, want_type: u16) -> Result<()> {
        loop {
            let h = self.load_tslv_header()?;
            self.stack.push(RecordInfo {
                start: self.pos,
                length: h.length,
                rtype: h.rtype,
                len_kind: h.len_kind,
            });
            if h.rtype == want_type {
                return Ok(());
            }
            self.skip_rest_of_record();
            if self.at_end() {
                return Err(Error::codec(format!(
                    "record type {want_type:#06x} not found before end of stream"
                )));
            }
        }
    }

    /// Advance past the current record's remaining bytes and pop it.
    fn skip_rest_of_record(&mut self) {
        if !self.stack.is_empty() {
            self.pos += self.n_bytes_left_in_record() as usize;
            self.stack.pop();
        }
    }

    /// Read the type-`0xffff` record → flags + IV.
    pub(crate) fn load_stream_header(&mut self) -> Result<StreamHeader> {
        self.next_record(StreamHeader::RECORD_TYPE)?;
        let is_enc = self.load_short()? != 0;
        let version = self.load_short()?;
        let use_fixed = self.load_short()? != 0;
        let iv = if is_enc {
            self.load_block(16)?
        } else {
            Vec::new()
        };
        Ok(StreamHeader {
            is_encrypted: is_enc,
            version,
            use_fixed_key: use_fixed,
            iv,
        })
    }

    /// The byte offset just past the current (top) record — i.e. where the payload begins
    /// after [`load_stream_header`]. The header record may declare trailing bytes (an
    /// `extra` field) beyond the IV, so this is *not* the read cursor.
    pub(crate) fn top_record_end(&self) -> usize {
        match self.stack.last() {
            Some(top) => top.start + top.length as usize,
            None => self.pos,
        }
    }
}
