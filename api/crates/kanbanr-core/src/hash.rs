//! A stable content hash for values kanbanr persists and compares across runs and Rust versions
//! (re-import keys, mirrored-issue change detection). `std`'s `DefaultHasher` is not guaranteed
//! stable across releases, so this is a plain FNV-1a 64 — not cryptographic, just deterministic.

/// FNV-1a 64 of `s`, as 16 lowercase hex digits.
pub fn stable_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_fnv1a_64_reference_values() {
        assert_eq!(stable_hash(""), "cbf29ce484222325");
        assert_eq!(stable_hash("a"), "af63dc4c8601ec8c");
        assert_eq!(stable_hash("foobar"), "85944171f73967e8");
    }
}
