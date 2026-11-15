//! Canonical JSON encoding for manifests.
//!
//! The canonical encoding is the single byte representation used for the
//! manifest hash and for the manifests committed to the repository:
//!
//! - object keys are sorted lexicographically at every level,
//! - the document is pretty-printed with two-space indentation and LF line
//!   endings,
//! - the document ends with exactly one trailing newline,
//! - `files` is sorted by `path` and each `siblings` list is sorted.

use serde_json::{Map, Value};

use crate::manifest::Manifest;

/// Returns the canonical JSON encoding of `manifest`.
///
/// The bytes are UTF-8, use LF line endings, and end with exactly one newline.
/// The input is canonicalised even when it was parsed from JSON whose object
/// keys or arrays were not in canonical order.
pub fn canonical_bytes(manifest: &Manifest) -> Vec<u8> {
    let mut canonical = manifest.clone();
    canonical
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    for file in &mut canonical.files {
        file.siblings.sort();
    }

    let value = serde_json::to_value(&canonical).expect("manifest always serialises to JSON");
    let mut json = serde_json::to_string_pretty(&sort_object_keys(value))
        .expect("manifest JSON is always a valid JSON document");
    json.push('\n');
    json.into_bytes()
}

/// Rebuilds `value` with every object's keys in lexicographic order.
pub(crate) fn sort_object_keys(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<(String, Value)> = object.into_iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, sort_object_keys(value));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_object_keys).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_object_keys_recursively() {
        let value = serde_json::json!({
            "b": {"z": 1, "a": [{"y": 2, "x": 3}]},
            "a": null,
        });
        let sorted = sort_object_keys(value);
        assert_eq!(
            serde_json::to_string(&sorted).unwrap(),
            r#"{"a":null,"b":{"a":[{"x":3,"y":2}],"z":1}}"#
        );
    }

    #[test]
    fn canonical_bytes_canonicalises_arrays() {
        let json = br#"{"schema":1,"source":{"revision":"499bc8821c6b12b4e53c5bffcb21ec206f212d81","repoType":"model","repo":"a/b","kind":"huggingface"},"files":[{"path":"b.gguf","role":"weights","size":2,"sha256":"0000000000000000000000000000000000000000000000000000000000000002","blake3":"0000000000000000000000000000000000000000000000000000000000000001"},{"path":"a.gguf","role":"weights","size":1,"sha256":"0000000000000000000000000000000000000000000000000000000000000004","blake3":"0000000000000000000000000000000000000000000000000000000000000003"}]}"#;
        let manifest = Manifest::from_json_bytes(json).unwrap();
        let canonical = String::from_utf8(manifest.canonical_bytes()).unwrap();
        let a = canonical.find("\"path\": \"a.gguf\"").unwrap();
        let b = canonical.find("\"path\": \"b.gguf\"").unwrap();
        assert!(a < b, "files must be sorted by path:\n{canonical}");
        assert!(canonical.starts_with("{\n  \"files\": [\n"));
        assert!(canonical.ends_with("}\n"));
    }
}
