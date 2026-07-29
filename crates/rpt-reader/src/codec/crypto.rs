//! The `Contents` payload cipher.
//!
//! The compressed `Contents` payload is encrypted with a **modified AES-128 in CFB-128 mode**. It
//! shares the block function and CFB loop of the shared [`super::aes128`] core; what makes it
//! *modified* is the key schedule — the 44 round-key words below are the engine's pre-expansion of
//! the universal fixed key (constant for every fixed-key file), not a standard AES key expansion.

use super::aes128;

/// The 44 round-key words (universal fixed-key expansion). `RK[0..4]` little-endian is the
/// fixed key `11dd1896bd4a15cdbff2543503e6760f`.
#[rustfmt::skip]
const RK: [u32; 44] = [
    0x9618dd11,0xcd154abd,0x3554f2bf,0x0f76e603, 0xaf96a667,0x6283ecda,0x57d71e65,0x58a1f866,
    0x9fd7950d,0xfd5479d7,0xaa8367b2,0xf2229fd4, 0x080cdd84,0xf558a453,0x5fdbc3e1,0xadf95c35,
    0x99464b11,0x6c1eef42,0x33c52ca3,0x9e3c7096, 0x6217db1a,0x0e093458,0x3dcc18fb,0xa3f0686d,
    0xce52e710,0xc05bd348,0xfd97cbb3,0x5e67a3de, 0x0b58fa48,0xcb032900,0x3694e2b3,0x68f3416d,
    0x86dbc60d,0x4dd8ef0d,0x7b4c0dbe,0x13bf4cd3, 0x95f2a070,0xd82a4f7d,0xa36642c3,0xb0d90e10,
    0x96596a97,0x4e7325ea,0xed156729,0x5dcc6939,
];

/// The modified-AES-128 block **encryption** (used to make the keystream).
pub(crate) fn encrypt_block(input: &[u8; 16]) -> [u8; 16] {
    aes128::encrypt_block(input, &RK)
}

/// Decrypt `ciphertext` in CFB-128 mode (keystream block = `E(prev_ciphertext)`, first
/// block = `E(iv)`). Returns the plaintext (same length).
pub(crate) fn cfb_decrypt(iv: &[u8; 16], ciphertext: &[u8]) -> Vec<u8> {
    aes128::cfb_decrypt(iv, ciphertext, &RK)
}

/// Encrypt `plaintext` in CFB-128 mode — the inverse of [`cfb_decrypt`] (keystream block =
/// `E(prev_ciphertext)`, first block = `E(iv)`). Returns the ciphertext (same length).
pub(crate) fn cfb_encrypt(iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    aes128::cfb_encrypt(iv, plaintext, &RK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfb_encrypt_decrypt_round_trips() {
        // The CFB pair is a mutual inverse for any length, including a partial final block.
        let iv = [
            0x0d, 0x31, 0x92, 0x7f, 0x9e, 0xe3, 0xa7, 0xac, 0x12, 0xd9, 0x1f, 0x68, 0xd6, 0x6b,
            0x7e, 0x16,
        ];
        for len in [0usize, 1, 15, 16, 17, 31, 33, 100] {
            let plain: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(37)).collect();
            let ct = cfb_encrypt(&iv, &plain);
            assert_eq!(ct.len(), plain.len());
            assert_eq!(cfb_decrypt(&iv, &ct), plain, "len {len}");
        }
    }

    #[test]
    fn block_encrypt_known_answer() {
        // Known-answer vector: E(IV) → expected output.
        let iv = [
            0x0d, 0x31, 0x92, 0x7f, 0x9e, 0xe3, 0xa7, 0xac, 0x12, 0xd9, 0x1f, 0x68, 0xd6, 0x6b,
            0x7e, 0x16,
        ];
        let expect = [
            0x8a, 0xba, 0xad, 0x84, 0x22, 0x09, 0xad, 0xa4, 0x73, 0x1e, 0xdb, 0xb9, 0x36, 0x54,
            0x76, 0xd0,
        ];
        assert_eq!(encrypt_block(&iv), expect);
    }
}
