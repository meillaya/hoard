//! Streaming SHA-256/BLAKE3 file hashing tests.

use std::io::Write;

use sha2::{Digest, Sha256};

use hoard_core::hash::{blake3_file, sha256_file};
use hoard_core::{Error, Hash32};

/// SHA-256 of `b"hello"`.
const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
/// BLAKE3 of `b"hello"`.
const HELLO_BLAKE3: &str = "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f";
/// SHA-256 of the empty string.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// BLAKE3 of the empty string.
const EMPTY_BLAKE3: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

fn write_temp(contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data.bin");
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(contents).unwrap();
    file.sync_all().unwrap();
    (directory, path)
}

#[test]
fn hashes_known_contents() {
    let contents: &[u8] = b"hello";
    let (_directory, path) = write_temp(contents);

    assert_eq!(sha256_file(&path).unwrap().to_string(), HELLO_SHA256);
    assert_eq!(blake3_file(&path).unwrap().to_string(), HELLO_BLAKE3);

    // Cross-check against the reference crate implementations on the same bytes.
    let mut sha = Sha256::new();
    sha.update(contents);
    assert_eq!(
        sha256_file(&path).unwrap().as_bytes(),
        sha.finalize().as_slice()
    );
    assert_eq!(
        blake3_file(&path).unwrap().as_bytes(),
        blake3::hash(contents).as_bytes()
    );
}

#[test]
fn hashes_empty_file() {
    let (_directory, path) = write_temp(b"");
    assert_eq!(sha256_file(&path).unwrap().to_string(), EMPTY_SHA256);
    assert_eq!(blake3_file(&path).unwrap().to_string(), EMPTY_BLAKE3);
}

#[test]
fn hashes_files_larger_than_the_chunk_size() {
    // 2.5 chunks of 1 MiB, with content that differs between chunks.
    let size = 2 * 1024 * 1024 + 512 * 1024;
    let contents: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
    let (_directory, path) = write_temp(&contents);

    let mut sha = Sha256::new();
    sha.update(&contents);
    let expected_sha = sha.finalize();
    assert_eq!(
        sha256_file(&path).unwrap().as_bytes(),
        expected_sha.as_slice(),
        "streaming SHA-256 must match a single-pass hash"
    );
    assert_eq!(
        blake3_file(&path).unwrap().as_bytes(),
        blake3::hash(&contents).as_bytes(),
        "streaming BLAKE3 must match a single-pass hash"
    );
}

#[test]
fn hash_helpers_report_missing_files_with_the_path() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("nope.gguf");

    for error in [
        sha256_file(&missing).unwrap_err(),
        blake3_file(&missing).unwrap_err(),
    ] {
        assert!(matches!(error, Error::Io { .. }), "{error}");
        assert!(error.to_string().contains("nope.gguf"), "{error}");
    }
}

#[test]
fn hash32_round_trips_through_strings() {
    let hash: Hash32 = HELLO_SHA256.parse().unwrap();
    assert_eq!(hash.to_hex(), HELLO_SHA256);
    assert_eq!(hash.to_string(), HELLO_SHA256);
    let reparsed: Hash32 = hash.to_string().parse().unwrap();
    assert_eq!(reparsed, hash);
}
