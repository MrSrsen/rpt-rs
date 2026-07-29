//! Minimal deterministic PNG encoder for placeholder blob images.
//!
//! Emits a valid 8-bit RGB PNG of a solid color using a single *stored*
//! (uncompressed) DEFLATE block, so no compression library is needed and the
//! bytes are identical on every host. CRC-32 and Adler-32 are computed here.
//! The output is a real, decodable PNG — just enough to exercise Picture/Blob
//! rendering — not a compact one.

/// A tiny solid-color RGB PNG of side `n` px in the given color.
pub(crate) fn solid_png(n: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    // IHDR: width, height, bit depth 8, color type 2 (RGB), no interlace.
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&n.to_be_bytes());
    ihdr.extend_from_slice(&n.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);

    // Raw image: each scanline is a filter byte (0 = none) + n RGB pixels.
    let mut raw = Vec::with_capacity((n as usize) * (1 + 3 * n as usize));
    for _ in 0..n {
        raw.extend_from_slice(&[0]); // filter type: none
        for _ in 0..n {
            raw.extend_from_slice(&rgb);
        }
    }
    chunk(&mut out, b"IDAT", &zlib_store(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Wrap `data` in a zlib stream using stored DEFLATE blocks.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x01]; // CMF/FLG (32K window, no preset dict)
    for (i, block) in data.chunks(0xffff).enumerate() {
        let last = (i + 1) * 0xffff >= data.len();
        z.push(if last { 1 } else { 0 }); // BFINAL, BTYPE=00
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(data).to_be_bytes());
    z
}

/// Append a length-prefixed, CRC-suffixed PNG chunk.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
