//! Content hashing shared by the render backends' image-dedup caches.

/// FNV-1a 64-bit hash of an image's bytes — the dedup/decode-cache key so identical images share one
/// decode or embedded entry. A hash (not the raw bytes) keeps the map key small; a collision only
/// over-shares two truly distinct images, astronomically unlikely for report imagery.
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_known_vectors() {
        assert_eq!(content_hash(b""), 0xcbf29ce484222325);
        assert_eq!(content_hash(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(content_hash(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn distinct_bytes_hash_distinctly() {
        assert_ne!(content_hash(b"one"), content_hash(b"two"));
    }
}
