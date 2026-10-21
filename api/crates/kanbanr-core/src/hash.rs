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

/// The canonical form of a value (FEAT-187): compact JSON with every object's keys sorted, at every
/// depth. What a content revision hashes, so a revision depends on what a record holds and never
/// on how its file was written — quoting, indentation, key order or the YAML library.
///
/// Keys are sorted rather than left in declaration order because serde_json's map order depends on
/// whether some crate in the build turned on its `preserve_order` feature (the CLI does): one
/// record must hash the same in every build.
pub fn canonical_json<T: serde::Serialize + ?Sized>(value: &T) -> String {
    let value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
    let mut out = String::new();
    write_canonical(&value, &mut out);
    out
}

fn write_canonical(value: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&serde_json::to_string(scalar).unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_keys_at_every_depth() {
        let a = serde_json::json!({"b": 1, "a": {"y": [2, {"q": 1, "p": "x"}], "x": null}});
        assert_eq!(
            canonical_json(&a),
            r#"{"a":{"x":null,"y":[2,{"p":"x","q":1}]},"b":1}"#
        );
    }

    #[test]
    fn matches_the_fnv1a_64_reference_values() {
        assert_eq!(stable_hash(""), "cbf29ce484222325");
        assert_eq!(stable_hash("a"), "af63dc4c8601ec8c");
        assert_eq!(stable_hash("foobar"), "85944171f73967e8");
    }
}
