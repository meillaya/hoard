//! Tests for the committed manifest fixtures.
//!
//! `fixtures/manifests/stories260k.json` records real Hugging Face metadata for
//! `ggml-org/models`; `fixtures/manifests/synthetic.json` uses fabricated
//! digests and only exercises structure and canonicalisation.
//!
//! The real GGUF file under `fixtures/real/` is gitignored; when it is absent
//! the tests that need it skip.

use std::path::{Path, PathBuf};

use hoard_core::hash::{blake3_file, sha256_file};
use hoard_core::{Blake3Digest, Manifest, RepoType, Role, SourceKind};

const STORIES_BLAKE3: &str = "1b2a49c52e28947a42c0b93606e1cd4554dc97f046f37fb292fbaa3517acdcc0";
const STORIES_SHA256: &str = "270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d";
const STORIES_REVISION: &str = "499bc8821c6b12b4e53c5bffcb21ec206f212d81";
const STORIES_SIZE: u64 = 1_185_376;

/// BLAKE3 of the canonical bytes of `fixtures/manifests/synthetic.json`.
const SYNTHETIC_MANIFEST_HASH: &str =
    "2624411eb97fa2e6ffebe635982ad3918b6d79237bf1cda897a2baf52fa68862";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    repo_root().join("fixtures/manifests").join(name)
}

/// Reads a fixture, asserts that it is canonical, valid, and returns it.
fn load_canonical_fixture(name: &str) -> (Manifest, Vec<u8>) {
    let path = fixture(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!("cannot read {}: {error}", path.display());
    });
    let manifest = Manifest::from_json_bytes(&bytes).unwrap_or_else(|error| {
        panic!("{} is not a valid manifest: {error}", path.display());
    });
    manifest.validate().unwrap_or_else(|error| {
        panic!("{} violates manifest v1: {error}", path.display());
    });
    assert_eq!(
        manifest.canonical_bytes(),
        bytes,
        "{} must be stored in canonical form; regenerate it from the library",
        path.display()
    );
    (manifest, bytes)
}

#[test]
fn stories260k_fixture_is_canonical_and_real() {
    let (manifest, bytes) = load_canonical_fixture("stories260k.json");

    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.source.kind, SourceKind::HuggingFace);
    assert_eq!(manifest.source.repo, "ggml-org/models");
    assert_eq!(manifest.source.repo_type, RepoType::Model);
    assert_eq!(manifest.source.revision, STORIES_REVISION);

    assert_eq!(manifest.files.len(), 1);
    let file = &manifest.files[0];
    assert_eq!(file.path, "tinyllamas/stories260K.gguf");
    assert_eq!(file.role, Role::Weights);
    assert_eq!(file.group, None);
    assert_eq!(file.size, STORIES_SIZE);
    assert_eq!(file.sha256.to_string(), STORIES_SHA256);
    assert_eq!(file.blake3.to_string(), STORIES_BLAKE3);
    assert!(file.siblings.is_empty());

    // The manifest hash is the BLAKE3 of the canonical bytes, not of the source
    // data: hash it from the committed artifact directly.
    let expected = format!("blake3:{}", blake3::hash(&bytes).to_hex());
    assert_eq!(manifest.hash().to_string(), expected);
}

#[test]
fn stories260k_fixture_matches_downloaded_file() {
    let real = repo_root().join("fixtures/real/stories260K.gguf");
    if !real.exists() {
        eprintln!(
            "skipping: {} is absent (fetch it with scripts/fetch-fixtures.sh)",
            real.display()
        );
        return;
    }

    let (manifest, _) = load_canonical_fixture("stories260k.json");
    let file = &manifest.files[0];

    assert_eq!(
        sha256_file(&real).unwrap().to_string(),
        file.sha256.to_string(),
        "recorded SHA-256 must match the downloaded file"
    );
    assert_eq!(
        blake3_file(&real).unwrap().to_string(),
        file.blake3.to_string(),
        "recorded BLAKE3 must match the downloaded file"
    );
    assert_eq!(
        std::fs::metadata(&real).unwrap().len(),
        file.size,
        "recorded size must match the downloaded file"
    );
    assert_eq!(file.blake3.to_string(), STORIES_BLAKE3);
    assert_eq!(file.sha256.to_string(), STORIES_SHA256);
}

#[test]
fn synthetic_fixture_is_canonical_and_has_a_stable_hash() {
    let (manifest, _bytes) = load_canonical_fixture("synthetic.json");

    assert_eq!(manifest.source.repo, "example/synthetic-artifacts");
    assert_eq!(manifest.files.len(), 4);

    let expected = format!("blake3:{SYNTHETIC_MANIFEST_HASH}");
    assert_eq!(
        manifest.hash().to_string(),
        expected,
        "synthetic fixture hash changed; canonical encoding changed or the fixture was edited"
    );

    // Two weights files with symmetric siblings.
    let weights: Vec<_> = manifest
        .files
        .iter()
        .filter(|file| file.role == Role::Weights)
        .collect();
    assert_eq!(weights.len(), 2);
    assert_eq!(weights[0].group.as_deref(), Some("stories"));
    assert_eq!(weights[0].siblings, ["weights/part-2.gguf"]);
    assert_eq!(weights[1].siblings, ["weights/part-1.gguf"]);
}

#[test]
fn fixtures_report_serde_round_trip() {
    for name in ["stories260k.json", "synthetic.json"] {
        let (manifest, _) = load_canonical_fixture(name);
        let serialised = serde_json::to_vec(&manifest).expect("manifest serialises");
        let reparsed =
            Manifest::from_json_bytes(&serialised).expect("round-tripped manifest parses");
        assert_eq!(reparsed, manifest, "{name} serde round trip");
    }
}

#[test]
fn fixture_hash_equals_hash_of_canonical_bytes() {
    for name in ["stories260k.json", "synthetic.json"] {
        let (manifest, bytes) = load_canonical_fixture(name);
        let expected: Blake3Digest = format!("blake3:{}", blake3::hash(&bytes).to_hex())
            .parse()
            .expect("digest string parses");
        assert_eq!(manifest.hash(), expected, "{name}");
    }
}
