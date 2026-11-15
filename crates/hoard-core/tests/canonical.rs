//! Byte-level canonical JSON tests.

use hoard_core::Manifest;

const REVISION: &str = "499bc8821c6b12b4e53c5bffcb21ec206f212d81";

/// A deliberately non-canonical document: object keys out of order, files out
/// of order, and sibling arrays out of order.
const SCRAMBLED: &[u8] = br#"{"schema":1,"source":{"revision":"499bc8821c6b12b4e53c5bffcb21ec206f212d81","repoType":"model","repo":"a/b","kind":"huggingface"},"files":[{"blake3":"4444444444444444444444444444444444444444444444444444444444444444","sha256":"3333333333333333333333333333333333333333333333333333333333333333","size":2,"role":"weights","path":"b.gguf","siblings":["a.gguf"]},{"blake3":"2222222222222222222222222222222222222222222222222222222222222222","sha256":"1111111111111111111111111111111111111111111111111111111111111111","size":1,"role":"weights","path":"a.gguf","siblings":["b.gguf"]}]}"#;

/// The canonical bytes of [`SCRAMBLED`].
const SCRAMBLED_CANONICAL: &str = r#"{
  "files": [
    {
      "blake3": "2222222222222222222222222222222222222222222222222222222222222222",
      "path": "a.gguf",
      "role": "weights",
      "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
      "siblings": [
        "b.gguf"
      ],
      "size": 1
    },
    {
      "blake3": "4444444444444444444444444444444444444444444444444444444444444444",
      "path": "b.gguf",
      "role": "weights",
      "sha256": "3333333333333333333333333333333333333333333333333333333333333333",
      "siblings": [
        "a.gguf"
      ],
      "size": 2
    }
  ],
  "schema": 1,
  "source": {
    "kind": "huggingface",
    "repo": "a/b",
    "repoType": "model",
    "revision": "499bc8821c6b12b4e53c5bffcb21ec206f212d81"
  }
}
"#;

fn parsed() -> Manifest {
    Manifest::from_json_bytes(SCRAMBLED).expect("scrambled document should parse")
}

#[test]
fn canonical_bytes_are_exact() {
    // The scrambled document parses but is not valid (files are unsorted);
    // canonicalisation is structural and does not validate.
    let manifest = parsed();
    assert!(manifest.validate().is_err());
    assert_eq!(
        manifest.canonical_bytes(),
        SCRAMBLED_CANONICAL.as_bytes(),
        "canonical encoding must match byte for byte"
    );
}

#[test]
fn canonical_bytes_are_idempotent() {
    let manifest = parsed();
    let canonical = manifest.canonical_bytes();
    let reparsed = Manifest::from_json_bytes(&canonical).expect("canonical bytes parse");
    assert_eq!(reparsed.canonical_bytes(), canonical);
    reparsed
        .validate()
        .expect("canonical bytes of a structurally valid manifest validate");
}

#[test]
fn canonical_encoding_ends_with_exactly_one_lf() {
    let bytes = parsed().canonical_bytes();
    assert!(bytes.ends_with(b"\n"), "canonical bytes must end with LF");
    assert!(
        !bytes.ends_with(b"\n\n"),
        "canonical bytes must have exactly one trailing LF"
    );
    assert!(
        !bytes.contains(&b'\r'),
        "canonical bytes must use LF line endings"
    );
    assert!(
        std::str::from_utf8(&bytes).is_ok(),
        "canonical bytes must be UTF-8"
    );
}

#[test]
fn canonical_byte_order_is_lexicographic() {
    let text = String::from_utf8(parsed().canonical_bytes()).unwrap();
    for key in ["files", "schema", "source"] {
        assert!(text.contains(&format!("\n  \"{key}\"")), "root key {key}");
    }
    let files = text.find("\"files\"").unwrap();
    let schema = text.find("\"schema\"").unwrap();
    let source = text.find("\"source\"").unwrap();
    assert!(
        files < schema && schema < source,
        "root keys must be sorted"
    );

    let blake3 = text.find("\"blake3\"").unwrap();
    let path = text.find("\"path\"").unwrap();
    let role = text.find("\"role\"").unwrap();
    let sha256 = text.find("\"sha256\"").unwrap();
    let size = text.find("\"size\"").unwrap();
    assert!(
        blake3 < path && path < role && role < sha256 && sha256 < size,
        "file keys must be sorted"
    );
}

#[test]
fn canonicalises_siblings_and_files() {
    let document = serde_json::json!({
        "schema": 1,
        "source": {
            "kind": "huggingface",
            "repo": "a/b",
            "repoType": "model",
            "revision": REVISION,
        },
        "files": [
            {
                "path": "weights/c.gguf",
                "role": "weights",
                "size": 3,
                "sha256": "3333333333333333333333333333333333333333333333333333333333333333",
                "blake3": "3333333333333333333333333333333333333333333333333333333333333333",
                "siblings": ["weights/b.gguf", "weights/a.gguf"],
            },
            {
                "path": "weights/a.gguf",
                "role": "weights",
                "size": 1,
                "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                "blake3": "1111111111111111111111111111111111111111111111111111111111111111",
                "siblings": ["weights/c.gguf", "weights/b.gguf"],
            },
        ],
    });
    let manifest = Manifest::from_json_bytes(&serde_json::to_vec(&document).unwrap()).unwrap();
    let canonical: serde_json::Value = serde_json::from_slice(&manifest.canonical_bytes()).unwrap();
    let files = canonical["files"].as_array().unwrap();
    assert_eq!(
        files[0]["path"], "weights/a.gguf",
        "files must be sorted by path"
    );
    assert_eq!(
        files[1]["path"], "weights/c.gguf",
        "files must be sorted by path"
    );
    assert_eq!(
        files[0]["siblings"],
        serde_json::json!(["weights/b.gguf", "weights/c.gguf"]),
        "siblings must be sorted"
    );
    assert_eq!(
        files[1]["siblings"],
        serde_json::json!(["weights/a.gguf", "weights/b.gguf"]),
        "siblings must be sorted"
    );
}

#[test]
fn hash_is_deterministic_and_matches_manual_blake3() {
    let manifest = parsed();
    let canonical = manifest.canonical_bytes();

    let expected = format!("blake3:{}", blake3::hash(&canonical).to_hex());
    assert_eq!(manifest.hash().to_string(), expected);
    assert_eq!(manifest.hash(), parsed().hash());
    assert_eq!(
        manifest.hash().as_bytes(),
        blake3::hash(&canonical).as_bytes()
    );
    assert_eq!(
        manifest.hash().to_hex(),
        blake3::hash(&canonical).to_hex().to_string()
    );

    let mut other = Manifest::from_json_bytes(SCRAMBLED).unwrap();
    other.files[0].size += 1;
    assert_ne!(other.hash(), manifest.hash());
}

#[test]
fn empty_manifest_canonicalises_without_files() {
    // Canonicalisation is purely structural; enforcement of non-empty `files`
    // belongs to validation.
    let manifest =
        Manifest::from_json_bytes(br#"{"schema":1,"source":{"kind":"huggingface","repo":"a/b","repoType":"model","revision":"499bc8821c6b12b4e53c5bffcb21ec206f212d81"},"files":[]}"#)
            .unwrap();
    assert!(manifest.validate().is_err());
    let canonical = String::from_utf8(manifest.canonical_bytes()).unwrap();
    assert!(canonical.contains("\"files\": []"));
    assert!(canonical.ends_with("}\n"));
}
