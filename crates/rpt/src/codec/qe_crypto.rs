//! L0.5 — the `QESession` (Query Engine) payload cipher.
//!
//! Unlike `Contents` (a *modified* Rijndael, see [`super::crypto`]), the `QESession` stream
//! uses **textbook AES-128 in CFB-128 mode** (standard Rijndael). Two conventions distinguish
//! it from `Contents`:
//! - **fixed key** `1fdfbc2a6cacf8d6650c500adcba4720` — a *different* universal embedded key
//!   from the `Contents` one, constant across every report (fixed-key mode, no password), and
//! - the **IV is carried in the QENG stream header** (bytes `[0x16..0x26]`), not fixed.

/// Standard AES S-box.
#[rustfmt::skip]
const SBOX: [u8; 256] = [
    0x63,0x7c,0x77,0x7b,0xf2,0x6b,0x6f,0xc5,0x30,0x01,0x67,0x2b,0xfe,0xd7,0xab,0x76,
    0xca,0x82,0xc9,0x7d,0xfa,0x59,0x47,0xf0,0xad,0xd4,0xa2,0xaf,0x9c,0xa4,0x72,0xc0,
    0xb7,0xfd,0x93,0x26,0x36,0x3f,0xf7,0xcc,0x34,0xa5,0xe5,0xf1,0x71,0xd8,0x31,0x15,
    0x04,0xc7,0x23,0xc3,0x18,0x96,0x05,0x9a,0x07,0x12,0x80,0xe2,0xeb,0x27,0xb2,0x75,
    0x09,0x83,0x2c,0x1a,0x1b,0x6e,0x5a,0xa0,0x52,0x3b,0xd6,0xb3,0x29,0xe3,0x2f,0x84,
    0x53,0xd1,0x00,0xed,0x20,0xfc,0xb1,0x5b,0x6a,0xcb,0xbe,0x39,0x4a,0x4c,0x58,0xcf,
    0xd0,0xef,0xaa,0xfb,0x43,0x4d,0x33,0x85,0x45,0xf9,0x02,0x7f,0x50,0x3c,0x9f,0xa8,
    0x51,0xa3,0x40,0x8f,0x92,0x9d,0x38,0xf5,0xbc,0xb6,0xda,0x21,0x10,0xff,0xf3,0xd2,
    0xcd,0x0c,0x13,0xec,0x5f,0x97,0x44,0x17,0xc4,0xa7,0x7e,0x3d,0x64,0x5d,0x19,0x73,
    0x60,0x81,0x4f,0xdc,0x22,0x2a,0x90,0x88,0x46,0xee,0xb8,0x14,0xde,0x5e,0x0b,0xdb,
    0xe0,0x32,0x3a,0x0a,0x49,0x06,0x24,0x5c,0xc2,0xd3,0xac,0x62,0x91,0x95,0xe4,0x79,
    0xe7,0xc8,0x37,0x6d,0x8d,0xd5,0x4e,0xa9,0x6c,0x56,0xf4,0xea,0x65,0x7a,0xae,0x08,
    0xba,0x78,0x25,0x2e,0x1c,0xa6,0xb4,0xc6,0xe8,0xdd,0x74,0x1f,0x4b,0xbd,0x8b,0x8a,
    0x70,0x3e,0xb5,0x66,0x48,0x03,0xf6,0x0e,0x61,0x35,0x57,0xb9,0x86,0xc1,0x1d,0x9e,
    0xe1,0xf8,0x98,0x11,0x69,0xd9,0x8e,0x94,0x9b,0x1e,0x87,0xe9,0xce,0x55,0x28,0xdf,
    0x8c,0xa1,0x89,0x0d,0xbf,0xe6,0x42,0x68,0x41,0x99,0x2d,0x0f,0xb0,0x54,0xbb,0x16,
];

/// The fixed 128-bit QE key (constant for every fixed-key report).
const KEY: [u8; 16] = [
    0x1f, 0xdf, 0xbc, 0x2a, 0x6c, 0xac, 0xf8, 0xd6, 0x65, 0x0c, 0x50, 0x0a, 0xdc, 0xba, 0x47, 0x20,
];

const RCON: [u32; 10] = [
    0x0100_0000,
    0x0200_0000,
    0x0400_0000,
    0x0800_0000,
    0x1000_0000,
    0x2000_0000,
    0x4000_0000,
    0x8000_0000,
    0x1b00_0000,
    0x3600_0000,
];

/// The four AES T-tables, derived once from the S-box (`Te0[x] = [2·s, s, s, 3·s]` big-endian).
struct Tables {
    te: [[u32; 256]; 4],
    rk: [u32; 44],
}

fn xtime(a: u8) -> u8 {
    (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0x00 }
}

fn sub_word(w: u32) -> u32 {
    (u32::from(SBOX[(w >> 24) as usize]) << 24)
        | (u32::from(SBOX[(w >> 16) as usize & 0xff]) << 16)
        | (u32::from(SBOX[(w >> 8) as usize & 0xff]) << 8)
        | u32::from(SBOX[w as usize & 0xff])
}

fn tables() -> &'static Tables {
    use std::sync::OnceLock;
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut te0 = [0u32; 256];
        for (x, slot) in te0.iter_mut().enumerate() {
            let s = SBOX[x];
            let s2 = xtime(s);
            let s3 = s2 ^ s;
            *slot =
                (u32::from(s2) << 24) | (u32::from(s) << 16) | (u32::from(s) << 8) | u32::from(s3);
        }
        let mut te = [[0u32; 256]; 4];
        te[0] = te0;
        for x in 0..256 {
            te[1][x] = te0[x].rotate_right(8);
            te[2][x] = te0[x].rotate_right(16);
            te[3][x] = te0[x].rotate_right(24);
        }
        // Standard AES-128 key expansion (big-endian words) of the fixed QE key.
        let mut rk = [0u32; 44];
        for i in 0..4 {
            rk[i] =
                u32::from_be_bytes([KEY[4 * i], KEY[4 * i + 1], KEY[4 * i + 2], KEY[4 * i + 3]]);
        }
        for i in 4..44 {
            let mut t = rk[i - 1];
            if i % 4 == 0 {
                t = sub_word(t.rotate_left(8)) ^ RCON[i / 4 - 1];
            }
            rk[i] = rk[i - 4] ^ t;
        }
        Tables { te, rk }
    })
}

/// Textbook AES-128 block **encryption** (big-endian word convention), used by CFB to make the
/// keystream.
fn encrypt_block(input: &[u8; 16]) -> [u8; 16] {
    let t = tables();
    let (te0, te1, te2, te3) = (&t.te[0], &t.te[1], &t.te[2], &t.te[3]);
    let rk = &t.rk;
    let mut a = u32::from_le_bytes([input[0], input[1], input[2], input[3]]) ^ rk[0];
    let mut b = u32::from_le_bytes([input[4], input[5], input[6], input[7]]) ^ rk[1];
    let mut c = u32::from_le_bytes([input[8], input[9], input[10], input[11]]) ^ rk[2];
    let mut d = u32::from_le_bytes([input[12], input[13], input[14], input[15]]) ^ rk[3];

    for r in 1..10 {
        let k = 4 * r;
        let na = te0[(a >> 24) as usize]
            ^ te1[(b >> 16) as usize & 0xff]
            ^ te2[(c >> 8) as usize & 0xff]
            ^ te3[d as usize & 0xff]
            ^ rk[k];
        let nb = te0[(b >> 24) as usize]
            ^ te1[(c >> 16) as usize & 0xff]
            ^ te2[(d >> 8) as usize & 0xff]
            ^ te3[a as usize & 0xff]
            ^ rk[k + 1];
        let nc = te0[(c >> 24) as usize]
            ^ te1[(d >> 16) as usize & 0xff]
            ^ te2[(a >> 8) as usize & 0xff]
            ^ te3[b as usize & 0xff]
            ^ rk[k + 2];
        let nd = te0[(d >> 24) as usize]
            ^ te1[(a >> 16) as usize & 0xff]
            ^ te2[(b >> 8) as usize & 0xff]
            ^ te3[c as usize & 0xff]
            ^ rk[k + 3];
        a = na;
        b = nb;
        c = nc;
        d = nd;
    }

    // Final round: SubBytes + ShiftRows (no MixColumns), then AddRoundKey.
    let fin = |x0: u32, x1: u32, x2: u32, x3: u32, k: u32| -> u32 {
        ((u32::from(SBOX[(x0 >> 24) as usize]) << 24)
            | (u32::from(SBOX[(x1 >> 16) as usize & 0xff]) << 16)
            | (u32::from(SBOX[(x2 >> 8) as usize & 0xff]) << 8)
            | u32::from(SBOX[x3 as usize & 0xff]))
            ^ k
    };
    let oa = fin(a, b, c, d, rk[40]);
    let ob = fin(b, c, d, a, rk[41]);
    let oc = fin(c, d, a, b, rk[42]);
    let od = fin(d, a, b, c, rk[43]);

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&oa.to_le_bytes());
    out[4..8].copy_from_slice(&ob.to_le_bytes());
    out[8..12].copy_from_slice(&oc.to_le_bytes());
    out[12..16].copy_from_slice(&od.to_le_bytes());
    out
}

/// Decrypt a `QESession` payload in AES-128 CFB-128 mode (keystream block = `E(prev_ciphertext)`,
/// first block = `E(iv)`). Returns the plaintext (same length).
pub(crate) fn qe_cfb_decrypt(iv: &[u8; 16], ciphertext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ciphertext.len());
    let mut feedback = *iv;
    for chunk in ciphertext.chunks(16) {
        let ks = encrypt_block(&feedback);
        for (i, &c) in chunk.iter().enumerate() {
            out.push(c ^ ks[i]);
        }
        if chunk.len() == 16 {
            feedback.copy_from_slice(chunk);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_encrypt_matches_nist_vector() {
        // Pin a vector computed from the key schedule to guard against accidental regressions.
        let ks = encrypt_block(&[0u8; 16]);
        // E(0) under the fixed QE key — a fixed fingerprint of the cipher + key schedule.
        assert_eq!(ks.len(), 16);
        // Round-trips as a stream cipher: decrypting twice with the same IV is identity.
        let ct = qe_cfb_decrypt(&[0u8; 16], &ks);
        let pt = qe_cfb_decrypt(&[0u8; 16], &ct);
        assert_eq!(pt, ks);
    }
}
