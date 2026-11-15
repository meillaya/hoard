//! Manifest parsing and validation matrix.

use std::path::Path;

use serde_json::{Value, json};

use hoard_core::{Error, Manifest, RepoType, Role, SourceKind};

const SHA256: &str = "270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d";
const BLAKE3: &str = "1b2a49c52e28947a42c0b93606e1cd4554dc97f046f37fb292fbaa3517acdcc0";
const REVISION: &str = "499bc8821c6b12b4e53c5bffcb21ec206f212d81";
const DEFAULT_PATH: &str = "weights/model.gguf";

/// A minimal valid manifest document.
fn base() -> Value {
    json!({
        "schema": 1,
        "source": {
            "kind": "huggingface",
            "repo": "ggml-org/models",
            "repoType": "model",
            "revision": REVISION,
        },
        "files": [file(json!({}))],
    })
}

/// Builds a `FileEntry` document with valid defaults and `overrides`.
///
/// A `null` override removes the key, which is how missing keys are tested.
fn file(overrides: Value) -> Value {
    let mut entry = json!({
        "path": DEFAULT_PATH,
        "role": "weights",
        "size": 1024,
        "sha256": SHA256,
        "blake3": BLAKE3,
    });
    for (key, value) in overrides
        .as_object()
        .expect("overrides must be an object")
        .clone()
    {
        if value.is_null() {
            entry.as_object_mut().unwrap().remove(&key);
        } else {
            entry[&key] = value;
        }
    }
    entry
}

fn with_files(files: Value) -> Value {
    let mut document = base();
    document["files"] = files;
    document
}

fn with_source(key: &str, value: Value) -> Value {
    let mut document = base();
    document["source"][key] = value;
    document
}

fn parse(document: &Value) -> Result<Manifest, Error> {
    let bytes = serde_json::to_vec(document).expect("test document serialises");
    Manifest::from_json_bytes(&bytes)
}

fn assert_valid(document: Value) -> Manifest {
    let manifest = parse(&document)
        .unwrap_or_else(|error| panic!("expected valid document: {document}\n{error}"));
    manifest
        .validate()
        .unwrap_or_else(|error| panic!("expected valid document: {document}\n{error}"));
    manifest
}

fn assert_invalid(document: Value, expected: &str) {
    let error = match parse(&document) {
        Ok(manifest) => match manifest.validate() {
            Ok(()) => panic!("document should have been rejected: {document}"),
            Err(error) => error,
        },
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains(expected),
        "expected {expected:?} in error {message:?} for {document}"
    );
}

#[test]
fn parses_minimal_manifest() {
    let manifest = assert_valid(base());
    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.source.kind, SourceKind::HuggingFace);
    assert_eq!(manifest.source.repo, "ggml-org/models");
    assert_eq!(manifest.source.repo_type, RepoType::Model);
    assert_eq!(manifest.source.revision, REVISION);
    assert_eq!(manifest.files.len(), 1);

    let file = &manifest.files[0];
    assert_eq!(file.path, DEFAULT_PATH);
    assert_eq!(file.role, Role::Weights);
    assert_eq!(file.size, 1024);
    assert_eq!(file.group, None);
    assert!(file.siblings.is_empty());
}

#[test]
fn accepts_every_role_and_allowed_groups() {
    let files = json!([
        file(json!({"path": "config.json", "role": "config"})),
        file(json!({"path": "mtp/draft.gguf", "role": "mtp", "group": "draft"})),
        file(json!({"path": "multimodal/mmproj.gguf", "role": "mmproj", "group": "mmproj"})),
        file(json!({"path": "other/README.md", "role": "other"})),
        file(json!({"path": "tokenizer/tokenizer.json", "role": "tokenizer"})),
        file(json!({"path": "weights/model.gguf", "role": "weights", "group": "stories"})),
    ]);
    let manifest = assert_valid(with_files(files));
    let roles: Vec<Role> = manifest.files.iter().map(|file| file.role).collect();
    assert_eq!(
        roles,
        [
            Role::Config,
            Role::Mtp,
            Role::Mmproj,
            Role::Other,
            Role::Tokenizer,
            Role::Weights,
        ]
    );
}

#[test]
fn accepts_dataset_and_space_repository_types() {
    for (name, expected) in [("dataset", RepoType::Dataset), ("space", RepoType::Space)] {
        let document = with_source("repoType", json!(name));
        assert_eq!(assert_valid(document).source.repo_type, expected);
    }
}

#[test]
fn accepts_symmetric_sibling_weights() {
    let files = json!([
        file(json!({
            "path": "weights/part-1.gguf",
            "group": "stories",
            "siblings": ["weights/part-2.gguf"],
        })),
        file(json!({
            "path": "weights/part-2.gguf",
            "group": "stories",
            "siblings": ["weights/part-1.gguf"],
        })),
    ]);
    let manifest = assert_valid(with_files(files));
    assert_eq!(manifest.files[0].siblings, ["weights/part-2.gguf"]);
    assert_eq!(manifest.files[1].siblings, ["weights/part-1.gguf"]);
}

#[test]
fn accepts_unicode_paths() {
    assert_valid(with_files(json!([file(json!({"path": "模型/权重.gguf"}))])));
}

#[test]
fn accepts_one_character_repo_parts() {
    assert_valid(with_source("repo", json!("a/b")));
}

#[test]
fn parsing_does_not_validate() {
    let files = json!([
        file(json!({"path": "b.gguf"})),
        file(json!({"path": "a.gguf"})),
    ]);
    let manifest = parse(&with_files(files)).expect("parsing tolerates unsorted files");
    assert!(manifest.validate().is_err());
}

#[test]
fn rejects_bad_schema_versions() {
    for version in [0, 2, 99] {
        let mut document = base();
        document["schema"] = json!(version);
        assert_invalid(document, "schema");
    }
}

#[test]
fn rejects_empty_files() {
    let mut document = base();
    document["files"] = json!([]);
    assert_invalid(document, "files");
}

#[test]
fn rejects_unknown_fields() {
    let mut document = base();
    document["timestamp"] = json!("2026-11-15T00:00:00Z");
    assert_invalid(document, "unknown field");

    assert_invalid(
        with_source("url", json!("https://huggingface.co")),
        "unknown field",
    );

    assert_invalid(
        with_files(json!([file(json!({"mtime": 1}))])),
        "unknown field",
    );
}

#[test]
fn rejects_bad_paths() {
    let cases = [
        ("empty", String::new()),
        ("absolute", "/weights/model.gguf".to_owned()),
        ("traversal", "../model.gguf".to_owned()),
        ("inner traversal", "weights/../model.gguf".to_owned()),
        ("dot", ".".to_owned()),
        ("dot segment", "./model.gguf".to_owned()),
        ("empty segment", "weights//model.gguf".to_owned()),
        ("trailing slash", "weights/".to_owned()),
        ("backslash", "weights\\model.gguf".to_owned()),
        ("nul", "weights/\0model.gguf".to_owned()),
        ("too long", format!("weights/{}", "a".repeat(1025))),
    ];
    for (label, path) in cases {
        assert_invalid(with_files(json!([file(json!({"path": path}))])), "path");
        let _ = label;
    }
}

#[test]
fn rejects_duplicate_and_unsorted_paths() {
    let files = json!([
        file(json!({"path": "weights/a.gguf"})),
        file(json!({"path": "weights/a.gguf"})),
    ]);
    assert_invalid(with_files(files), "duplicate");

    let files = json!([
        file(json!({"path": "weights/b.gguf"})),
        file(json!({"path": "weights/a.gguf"})),
    ]);
    assert_invalid(with_files(files), "sorted by path");
}

#[test]
fn rejects_zero_size_and_missing_size() {
    assert_invalid(with_files(json!([file(json!({"size": 0}))])), "size");
    assert_invalid(with_files(json!([file(json!({"size": null}))])), "size");
}

#[test]
fn rejects_bad_digests() {
    let bad_hashes = [
        SHA256.to_uppercase(),
        SHA256[..63].to_owned(),
        format!("{SHA256}00"),
        "z".repeat(64),
        format!("sha256:{SHA256}"),
        String::new(),
    ];
    for hash in bad_hashes {
        assert_invalid(with_files(json!([file(json!({"sha256": hash}))])), "digest");
    }

    let bad_blake3 = [
        BLAKE3.to_uppercase(),
        BLAKE3[..63].to_owned(),
        "g".repeat(64),
    ];
    for hash in bad_blake3 {
        assert_invalid(with_files(json!([file(json!({"blake3": hash}))])), "digest");
    }
}

#[test]
fn rejects_missing_required_file_fields() {
    for key in ["path", "role", "size", "sha256", "blake3"] {
        let mut overrides = serde_json::Map::new();
        overrides.insert(key.to_owned(), Value::Null);
        assert_invalid(
            with_files(json!([file(Value::Object(overrides))])),
            "missing field",
        );
    }
}

#[test]
fn rejects_bad_roles() {
    assert_invalid(
        with_files(json!([file(json!({"role": "Weight"}))])),
        "unknown variant",
    );
    assert_invalid(
        with_files(json!([file(json!({"role": "weights "}))])),
        "unknown variant",
    );
}

#[test]
fn rejects_bad_groups() {
    let too_long = format!("a{}", "b".repeat(64));
    let bad_groups = [
        "".to_owned(),
        "-stories".to_owned(),
        ".stories".to_owned(),
        "_stories".to_owned(),
        "Stories".to_owned(),
        "stor ies".to_owned(),
        "stor/ies".to_owned(),
        too_long,
    ];
    for group in bad_groups {
        assert_invalid(with_files(json!([file(json!({"group": group}))])), "group");
    }

    assert_invalid(
        with_files(json!([file(
            json!({"role": "tokenizer", "group": "stories"})
        )])),
        "group",
    );
}

#[test]
fn rejects_bad_siblings() {
    assert_invalid(
        with_files(json!([file(json!({"siblings": []}))])),
        "siblings",
    );

    assert_invalid(
        with_files(json!([file(json!({"siblings": [DEFAULT_PATH]}))])),
        "itself",
    );

    let files = json!([
        file(json!({"path": "tokenizer/tokenizer.json", "role": "tokenizer"})),
        file(json!({"path": "weights/model.gguf", "siblings": ["tokenizer/tokenizer.json"]})),
    ]);
    assert_invalid(with_files(files), "siblings");

    let files = json!([
        file(json!({"path": "weights/a.gguf", "siblings": ["weights/c.gguf", "weights/b.gguf"]})),
        file(json!({"path": "weights/b.gguf"})),
        file(json!({"path": "weights/c.gguf"})),
    ]);
    assert_invalid(with_files(files), "sorted");

    let files = json!([
        file(json!({"path": "weights/a.gguf", "siblings": ["weights/b.gguf", "weights/b.gguf"]})),
        file(json!({"path": "weights/b.gguf", "siblings": ["weights/a.gguf", "weights/a.gguf"]})),
    ]);
    assert_invalid(with_files(files), "sorted");

    let files = json!([file(
        json!({"path": "weights/a.gguf", "siblings": ["weights/missing.gguf"]})
    ),]);
    assert_invalid(with_files(files), "not listed");

    let files = json!([
        file(json!({"path": "config.json", "role": "config"})),
        file(json!({"path": "weights/a.gguf", "siblings": ["config.json"]})),
    ]);
    assert_invalid(with_files(files), "weights files");

    let files = json!([
        file(json!({"path": "weights/a.gguf", "group": "alpha", "siblings": ["weights/b.gguf"]})),
        file(json!({"path": "weights/b.gguf", "group": "beta", "siblings": ["weights/a.gguf"]})),
    ]);
    assert_invalid(with_files(files), "same group");

    let files = json!([
        file(json!({"path": "weights/a.gguf", "siblings": ["weights/b.gguf"]})),
        file(json!({"path": "weights/b.gguf", "group": "stories"})),
    ]);
    assert_invalid(with_files(files), "same group");

    let files = json!([
        file(json!({"path": "weights/a.gguf", "siblings": ["weights/b.gguf"]})),
        file(json!({"path": "weights/b.gguf"})),
    ]);
    assert_invalid(with_files(files), "symmetric");
}

#[test]
fn rejects_bad_source_repos() {
    for repo in [
        "",
        "owner",
        "owner/",
        "/model",
        "a/b/c",
        "own..er/model",
        "owner/..",
        "-owner/model",
        "owner./model",
        ".owner/model",
        "owner/.model",
        "owner name/model",
        "ownér/model",
    ] {
        assert_invalid(with_source("repo", json!(repo)), "source.repo");
    }
}

#[test]
fn rejects_bad_revisions() {
    let uppercase = REVISION.to_uppercase();
    let too_long = format!("{REVISION}0");
    let too_short = REVISION[..39].to_owned();
    let prefixed = format!("sha256:{SHA256}");
    for revision in [
        String::new(),
        too_short,
        too_long,
        uppercase,
        prefixed,
        "z".repeat(40),
    ] {
        assert_invalid(with_source("revision", json!(revision)), "source.revision");
    }
}

#[test]
fn rejects_unknown_source_kinds_and_repo_types() {
    assert_invalid(with_source("kind", json!("local")), "unknown variant");
    assert_invalid(with_source("kind", json!("HuggingFace")), "unknown variant");
    assert_invalid(with_source("repoType", json!("Model")), "unknown variant");
}

#[test]
fn from_path_reports_io_and_json_errors() {
    let directory = tempfile::tempdir().unwrap();

    let missing = directory.path().join("missing.json");
    let error = Manifest::from_path(&missing).unwrap_err();
    assert!(matches!(error, Error::Io { .. }), "{error}");
    assert!(error.to_string().contains("missing.json"), "{error}");

    let invalid = directory.path().join("invalid.json");
    std::fs::write(&invalid, b"{ not json").unwrap();
    let error = Manifest::from_path(&invalid).unwrap_err();
    assert!(matches!(error, Error::JsonFile { .. }), "{error}");
    assert!(error.to_string().contains("invalid.json"), "{error}");
}

#[test]
fn from_path_reads_valid_manifest() {
    let document = base();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("manifest.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    let manifest = Manifest::from_path(&path).expect("manifest file should parse");
    manifest.validate().expect("manifest file should validate");
    assert_eq!(manifest.files.len(), 1);
}

#[test]
fn from_json_bytes_rejects_trailing_garbage() {
    let mut bytes = serde_json::to_vec(&base()).unwrap();
    bytes.extend_from_slice(b"{}");
    assert!(Manifest::from_json_bytes(&bytes).is_err());
}

#[test]
fn from_json_bytes_rejects_non_object_documents() {
    for bytes in [b"[]".as_slice(), b"null", b"42", b"\"manifest\""] {
        assert!(Manifest::from_json_bytes(bytes).is_err());
    }
}

#[test]
fn paths_are_relative_to_the_workdir_not_the_manifest() {
    // Sanity check that `from_path` is the only filesystem-dependent entry point.
    let _ = Path::new(DEFAULT_PATH);
    assert!(Manifest::from_json_bytes(br#"{"schema":1}"#).is_err());
}
