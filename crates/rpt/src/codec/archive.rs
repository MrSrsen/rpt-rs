//! L1 — the record read state machine.
//!
//! Models the byte cursor, the running XOR [`Mask`] (initial `0`), and the record-info
//! stack, so `load_block` performs the record-spanning, demasked reads that reconstruct the
//! logical content of the stream. `load_stream_header` extracts the per-stream IV.

use super::header::StreamHeader;
use super::mask::Mask;
use super::tslv::{self, Flags};
use crate::error::{CodecError, Result};

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
            .ok_or_else(|| {
                CodecError::new(format!(
                    "tried to read {n} bytes past end of {}-byte stream",
                    self.d.len()
                ))
                .at(self.pos)
            })?;
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
                return Err(CodecError::new(
                    "reached end of stream while searching for record type",
                )
                .record(want_type)
                .at(self.pos)
                .into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StreamId;

    /// The raw `Contents` bytes of a committed fixture, plus the offset whose low-bit flip clears the
    /// header's `useFixed` short (found by search, so it survives the fixture being re-authored).
    fn fixture_with_usefixed_offset() -> Option<(Vec<u8>, Vec<u8>, usize)> {
        let path = rpt_test_support::fixture("tests/fixtures/reports/synthetic/blank_report.rpt");
        let file = std::fs::read(&path).ok()?;
        let rpt = crate::Rpt::open(&path).ok()?;
        let raw = rpt.stream(&StreamId::Contents)?.raw_bytes().to_vec();
        let base = ReadArchive::new(&raw).load_stream_header().ok()?;
        let off = (0..64).find(|&off| {
            let mut p = raw.clone();
            p[off] ^= 1;
            ReadArchive::new(&p).load_stream_header().is_ok_and(|h| {
                h.is_encrypted && !h.use_fixed_key && h.version == base.version && h.iv == base.iv
            })
        })?;
        Some((file, raw, off))
    }

    /// Clearing the header's `useFixed` flag changes nothing about how the stream decodes: the
    /// payload still decrypts with the universal built-in key, byte for byte. This mirrors the
    /// engine, which ignores the flag the same way — a report whose only modification is this bit
    /// cleared opens normally in the designer, with no password prompt. The test guards
    /// against "helpfully" branching on the flag later and refusing a file Crystal reads fine.
    #[test]
    fn non_fixed_key_flag_does_not_change_decoding() {
        let Some((_, raw, off)) = fixture_with_usefixed_offset() else {
            eprintln!("[skip] fixture absent or no useFixed bit found");
            return;
        };
        let mut patched = raw.clone();
        patched[off] ^= 1;

        let header = ReadArchive::new(&patched)
            .load_stream_header()
            .expect("parses");
        assert!(header.is_encrypted, "still encrypted");
        assert!(!header.use_fixed_key, "flag really is cleared");

        let plain = crate::codec::decode_contents(&raw).expect("control decodes");
        let patched_plain =
            crate::codec::decode_contents(&patched).expect("flag must not gate decoding");
        assert_eq!(
            plain, patched_plain,
            "the flag is inert — same logical bytes either way"
        );
    }

    /// A stream encrypted with a key this reader does not have is diagnosed as a KEY problem, not
    /// as a zlib problem. Crystal never emits one, but a third-party application with its own copy
    /// of the encryption library can round-trip reports under its own key — and since that key is
    /// not in the file, the payload simply will not decrypt. Simulated here by corrupting the
    /// ciphertext, which produces the same observable: an encrypted stream that decrypts to
    /// non-zlib bytes.
    ///
    /// Both flag states are checked, because the key and the flag are INDEPENDENT setters — a foreign
    /// key can arrive with `useFixed` still set, so the diagnosis must not be gated on the flag.
    #[test]
    fn foreign_key_is_reported_as_a_key_problem() {
        let Some((_, raw, off)) = fixture_with_usefixed_offset() else {
            eprintln!("[skip] fixture absent or no useFixed bit found");
            return;
        };
        let header_end = {
            let mut a = ReadArchive::new(&raw);
            a.next_record(StreamHeader::RECORD_TYPE).unwrap();
            a.top_record_end()
        };

        for clear_flag in [false, true] {
            let mut p = raw.clone();
            if clear_flag {
                p[off] ^= 1;
            }
            // Perturb the ciphertext body: stands in for "encrypted under a different key".
            p[header_end] ^= 0xff;

            let err = crate::codec::decode_contents(&p)
                .expect_err("a stream we cannot key must not decode");
            let msg = err.to_string();
            assert!(
                msg.contains("key this reader does not have"),
                "clear_flag={clear_flag}: {msg}"
            );
            let hint = if clear_flag {
                "useFixed = 0"
            } else {
                "claims the built-in key"
            };
            assert!(msg.contains(hint), "clear_flag={clear_flag}: {msg}");
            assert!(msg.len() < 400, "diagnostic must stay readable: {msg}");
        }
    }

    /// Emit a `.rpt` whose `Contents` payload is encrypted with a **non-built-in key** — the exact
    /// artifact a third-party host with its own copy of the encryption key would produce. No SAP
    /// binary can author one, so it is built here: decrypt the real
    /// payload with the built-in key, then re-encrypt the same deflate stream under a DIFFERENT real
    /// Crystal key — the QESession universal key — reusing the header IV. The result is structurally
    /// a perfectly valid report that simply cannot be keyed by anyone but its author.
    ///
    /// `RPT_EMIT_FOREIGNKEY=<path>` writes it; add `RPT_FOREIGNKEY_CLEAR_FLAG=1` to also clear
    /// `useFixed`. Write OUTSIDE `tests/fixtures/reports/`, which is an auto-discovered corpus.
    #[test]
    fn emit_foreign_key_report() {
        let Some(out) = std::env::var_os("RPT_EMIT_FOREIGNKEY") else {
            return;
        };
        let (file, raw, off) = fixture_with_usefixed_offset().expect("fixture");

        let header = ReadArchive::new(&raw).load_stream_header().expect("header");
        let iv: [u8; 16] = header.iv.as_slice().try_into().expect("16-byte IV");
        let body_off = {
            let mut a = ReadArchive::new(&raw);
            a.next_record(StreamHeader::RECORD_TYPE).unwrap();
            a.top_record_end()
        };

        // Built-in key -> plaintext deflate -> re-encrypt under the other universal key.
        let deflate = super::super::crypto::cfb_decrypt(&iv, &raw[body_off..]);
        let foreign =
            super::super::aes128::cfb_encrypt(&iv, &deflate, super::super::qe_crypto::round_keys());

        let mut stream = raw[..body_off].to_vec();
        stream.extend_from_slice(&foreign);
        if std::env::var_os("RPT_FOREIGNKEY_CLEAR_FLAG").is_some() {
            stream[off] ^= 1;
        }

        let rewritten =
            crate::container::rewrite_stream(&file, &StreamId::Contents, &stream).expect("rewrite");
        std::fs::write(&out, rewritten).expect("write");
        eprintln!(
            "wrote {:?} (Contents re-keyed to the QE universal key)",
            out
        );
    }

    /// Emit a `.rpt` whose `Contents` header declares `useFixed = 0` while the payload stays
    /// encrypted with the built-in key — the probe that established the flag is inert (the designer
    /// opens it normally, no prompt). Kept for re-running that check against a future engine build.
    /// Off by default; set `RPT_EMIT_NONFIXED=<path>` to write one. Write it OUTSIDE
    /// `tests/fixtures/reports/`, which is an auto-discovered baseline corpus.
    #[test]
    fn emit_non_fixed_key_report() {
        let Some(out) = std::env::var_os("RPT_EMIT_NONFIXED") else {
            return;
        };
        let (file, raw, off) = fixture_with_usefixed_offset().expect("fixture");
        let mut patched = raw.clone();
        patched[off] ^= 1;
        let rewritten = crate::container::rewrite_stream(&file, &StreamId::Contents, &patched)
            .expect("rewrite");
        std::fs::write(&out, rewritten).expect("write");
        eprintln!(
            "wrote {:?} (useFixed cleared at raw Contents offset {off})",
            out
        );
    }
}
