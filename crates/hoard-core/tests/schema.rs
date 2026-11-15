//! JSON Schema generation and regeneration tests.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn schema_path() -> PathBuf {
    repo_root().join("schemas/manifest-v1.schema.json")
}

fn pretty_schema() -> String {
    let mut text = serde_json::to_string_pretty(&hoard_core::manifest_schema()).unwrap();
    text.push('\n');
    text
}

#[test]
fn committed_schema_is_current() {
    let path = schema_path();
    let actual = pretty_schema();

    if std::env::var_os("UPDATE_SCHEMAS").is_some() {
        std::fs::write(&path, &actual).expect("rewrite schema");
        return;
    }

    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    assert_eq!(
        actual, expected,
        "schemas/manifest-v1.schema.json is stale; regenerate it with \
         `UPDATE_SCHEMAS=1 cargo test -p hoard-core --test schema`"
    );
}

#[test]
fn schema_matches_committed_value() {
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(schema_path()).unwrap()).unwrap();
    assert_eq!(hoard_core::manifest_schema(), expected);
}

#[test]
fn schema_is_deterministic() {
    assert_eq!(hoard_core::manifest_schema(), hoard_core::manifest_schema());
    assert_eq!(pretty_schema(), pretty_schema());
}

#[test]
fn schema_describes_the_manifest_v1_shape() {
    let schema = hoard_core::manifest_schema();

    assert_eq!(schema["title"], "Manifest");
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["type"], "object");

    let required: Vec<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    for field in ["schema", "source", "files"] {
        assert!(required.contains(&field), "root required field {field}");
    }

    // `schema` is pinned to 1.
    assert_eq!(schema["properties"]["schema"]["minimum"], 1);
    assert_eq!(schema["properties"]["schema"]["maximum"], 1);
    // `files` is non-empty.
    assert_eq!(schema["properties"]["files"]["minItems"], 1);

    let definitions = schema["$defs"].as_object().expect("$defs object");
    for definition in ["FileEntry", "Role", "Source", "SourceKind", "RepoType"] {
        assert!(
            definitions.contains_key(definition),
            "missing $defs/{definition}"
        );
    }

    let file_entry = &definitions["FileEntry"];
    assert_eq!(file_entry["additionalProperties"], false);
    let file_required: Vec<&str> = file_entry["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    for field in ["path", "role", "size", "sha256", "blake3"] {
        assert!(
            file_required.contains(&field),
            "FileEntry required field {field}"
        );
    }

    let hash_pattern = "^[0-9a-f]{64}$";
    assert_eq!(
        file_entry["properties"]["sha256"]["pattern"], hash_pattern,
        "sha256 schema must be 64 lowercase hex characters"
    );
    assert_eq!(
        file_entry["properties"]["blake3"]["pattern"], hash_pattern,
        "blake3 schema must be 64 lowercase hex characters"
    );
    assert_eq!(file_entry["properties"]["size"]["minimum"], 1);

    let revision_pattern = "^[0-9a-f]{40}$";
    assert_eq!(schema["properties"]["source"]["$ref"], "#/$defs/Source");
    assert_eq!(
        definitions["Source"]["properties"]["revision"]["pattern"],
        revision_pattern
    );

    // Role values are enumerated.
    let roles: Vec<&str> = definitions["Role"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variant| variant["const"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        ["weights", "mmproj", "mtp", "tokenizer", "config", "other"]
    );
}
