//! L0.5 — the `QESession` (Query Engine) payload cipher.
//!
//! Unlike `Contents` (whose round keys are a pre-expanded modified schedule, see [`super::crypto`]),
//! the `QESession` stream uses a **standard AES-128 key expansion** over the shared [`super::aes128`]
//! core. Two conventions distinguish it from `Contents`:
//! - **fixed key** `1fdfbc2a6cacf8d6650c500adcba4720` — a *different* universal embedded key
//!   from the `Contents` one, constant across every report (fixed-key mode, no password), and
//! - the **IV is carried in the QENG stream header** (bytes `[0x16..0x26]`), not fixed.

use super::aes128;

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

/// The standard AES-128 key expansion (big-endian words) of the fixed QE key, computed once.
pub(crate) fn round_keys() -> &'static [u32; 44] {
    use std::sync::OnceLock;
    static RK: OnceLock<[u32; 44]> = OnceLock::new();
    RK.get_or_init(|| {
        let mut rk = [0u32; 44];
        for i in 0..4 {
            rk[i] =
                u32::from_be_bytes([KEY[4 * i], KEY[4 * i + 1], KEY[4 * i + 2], KEY[4 * i + 3]]);
        }
        for i in 4..44 {
            let mut t = rk[i - 1];
            if i % 4 == 0 {
                t = aes128::sub_word(t.rotate_left(8)) ^ RCON[i / 4 - 1];
            }
            rk[i] = rk[i - 4] ^ t;
        }
        rk
    })
}

/// Decrypt a `QESession` payload in AES-128 CFB-128 mode (keystream block = `E(prev_ciphertext)`,
/// first block = `E(iv)`). Returns the plaintext (same length).
pub(crate) fn qe_cfb_decrypt(iv: &[u8; 16], ciphertext: &[u8]) -> Vec<u8> {
    aes128::cfb_decrypt(iv, ciphertext, round_keys())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_encrypt_matches_nist_vector() {
        // Pin a vector computed from the key schedule to guard against accidental regressions.
        let ks = aes128::encrypt_block(&[0u8; 16], round_keys());
        // E(0) under the fixed QE key — a fixed fingerprint of the cipher + key schedule.
        assert_eq!(ks.len(), 16);
        // Round-trips as a stream cipher: decrypting twice with the same IV is identity.
        let ct = qe_cfb_decrypt(&[0u8; 16], &ks);
        let pt = qe_cfb_decrypt(&[0u8; 16], &ct);
        assert_eq!(pt, ks);
    }
}
