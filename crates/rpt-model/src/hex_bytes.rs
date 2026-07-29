//! Serde codec for the model's embedded binary payloads: a lowercase hex string.
//!
//! serde's default for a byte vector is a sequence of integers, which a pretty-printer spends a
//! line and roughly twenty characters on per byte — an embedded megabyte bitmap becomes twenty
//! megabytes of text. Hex keeps every byte (the payload round-trips exactly) at two characters
//! each, and keeps a byte's position readable: byte `i` is always at character `2i`, so an offset
//! from a hex dump of the source file indexes straight into the encoded string.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

/// Lowercase hex digits, indexed by nibble.
const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// The nibble a hex digit denotes, or `None` if `c` is not one.
fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Serialize `bytes` as a lowercase hex string.
pub(crate) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        hex.push(DIGITS[usize::from(b >> 4)] as char);
        hex.push(DIGITS[usize::from(b & 0x0f)] as char);
    }
    serializer.serialize_str(&hex)
}

/// Decode a lowercase-or-uppercase hex string back to bytes.
///
/// An error names the offending position rather than quoting the string, which can be megabytes
/// long.
pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let hex = String::deserialize(deserializer)?;
    if hex.len() % 2 != 0 {
        return Err(D::Error::custom(format!(
            "hex string has an odd length ({})",
            hex.len()
        )));
    }
    hex.as_bytes()
        .chunks_exact(2)
        .enumerate()
        .map(|(i, pair)| match (nibble(pair[0]), nibble(pair[1])) {
            (Some(hi), Some(lo)) => Ok((hi << 4) | lo),
            _ => Err(D::Error::custom(format!(
                "invalid hex digit pair at byte {i}"
            ))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::PictureObject;

    /// Exercises the codec on its own, without the rest of a picture's fields.
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Blob {
        #[serde(with = "super")]
        data: Vec<u8>,
    }

    #[test]
    fn every_byte_round_trips() {
        let blob = Blob {
            data: (0u8..=255).collect(),
        };
        let json = serde_json::to_string(&blob).expect("serialize");
        assert!(json.starts_with("{\"data\":\"000102030405"), "{json}");
        assert!(json.ends_with("fcfdfeff\"}"), "{json}");
        let back: Blob = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.data, blob.data);
    }

    #[test]
    fn empty_bytes_are_an_empty_string() {
        let json = serde_json::to_string(&Blob { data: Vec::new() }).expect("serialize");
        assert_eq!(json, "{\"data\":\"\"}");
    }

    #[test]
    fn uppercase_is_accepted() {
        let blob: Blob = serde_json::from_str("{\"data\":\"DEadBEef\"}").expect("deserialize");
        assert_eq!(blob.data, [0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn malformed_hex_is_rejected() {
        for bad in ["\"abc\"", "\"zz\"", "[1, 2]"] {
            let json = format!("{{\"data\":{bad}}}");
            serde_json::from_str::<Blob>(&json).expect_err("malformed hex must not deserialize");
        }
    }

    /// The picture field is wired to the codec, not just the codec itself.
    #[test]
    fn a_picture_carries_its_bytes_as_hex() {
        let picture = PictureObject {
            data: vec![0x42, 0x4d, 0x00],
            ..PictureObject::default()
        };
        let json = serde_json::to_string(&picture).expect("serialize");
        assert!(json.contains("\"data\":\"424d00\""), "{json}");
        let back: PictureObject = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, picture);
    }
}
