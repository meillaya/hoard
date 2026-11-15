//! CLI tests for the `hoard` binary (T5).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_hoard")
}

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir().join("../..")
}

fn real_fixture(name: &str) -> Option<PathBuf> {
    let path = repo_root().join("fixtures/real").join(name);
    path.is_file().then_some(path)
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("failed to run hoard")
}

#[test]
fn hash_json_reports_size_and_digests() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"hello\n").unwrap();
    let output = run(&["hash", file.path().to_str().unwrap(), "--json"]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("hash JSON");
    assert_eq!(report["schema"], 1);
    assert_eq!(report["size"], 6);
    assert_eq!(
        report["sha256"],
        "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
    );
    assert_eq!(report["blake3"].as_str().unwrap().len(), 64);
}

#[test]
fn hash_matches_library_for_real_fixture() {
    let Some(path) = real_fixture("stories260K.gguf") else {
        eprintln!("skipping: run scripts/fetch-fixtures.sh");
        return;
    };
    let output = run(&["hash", path.to_str().unwrap(), "--json"]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("hash JSON");
    assert_eq!(
        report["sha256"],
        "270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d"
    );
    let library = hoard_core::hash::blake3_file(&path).expect("blake3");
    assert_eq!(report["blake3"], library.to_string());
}

#[test]
fn manifest_validate_and_fmt_check_accept_committed_fixture() {
    let manifest = "fixtures/manifests/stories260k.json";
    let validate = run(&["manifest", "validate", manifest]);
    assert!(
        validate.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&validate.stderr)
    );

    let check = run(&["manifest", "fmt", manifest, "--check"]);
    assert!(check.status.success());

    let fmt = run(&["manifest", "fmt", manifest]);
    assert!(fmt.status.success());
    let committed = std::fs::read(repo_root().join(manifest)).unwrap();
    assert_eq!(fmt.stdout, committed);
}

#[test]
fn manifest_hash_is_artifact_id() {
    let manifest = "fixtures/manifests/stories260k.json";
    let output = run(&["manifest", "hash", manifest]);
    assert!(output.status.success());
    let printed = String::from_utf8(output.stdout).unwrap();
    let printed = printed.trim();
    assert!(printed.starts_with("blake3:"), "{printed}");
    let parsed = hoard_core::Manifest::from_path(&repo_root().join(manifest)).unwrap();
    assert_eq!(printed, parsed.hash().to_string());
}

#[test]
fn manifest_rejects_non_canonical_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    // Same content, different key order/indentation: valid JSON, not canonical.
    std::fs::write(
        &path,
        r#"{"schema":1,"files":[{"path":"a.gguf","role":"weights","size":1,"sha256":"0000000000000000000000000000000000000000000000000000000000000000","blake3":"0000000000000000000000000000000000000000000000000000000000000000"}],"source":{"kind":"huggingface","repo":"owner/name","repoType":"model","revision":"0000000000000000000000000000000000000000"}}"#,
    )
    .unwrap();

    let validate = run(&["manifest", "validate", path.to_str().unwrap()]);
    assert!(
        validate.status.success(),
        "semantics are valid; ordering is not"
    );

    let check = run(&["manifest", "fmt", path.to_str().unwrap(), "--check"]);
    assert_eq!(check.status.code(), Some(1));

    let fmt = run(&["manifest", "fmt", path.to_str().unwrap()]);
    assert!(fmt.status.success());
    let canonical = String::from_utf8(fmt.stdout).unwrap();
    assert!(canonical.ends_with('\n'));
    assert!(canonical.starts_with("{\n  \"files\":"), "{canonical}");
}

#[test]
fn manifest_validate_rejects_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    std::fs::write(&path, r#"{"schema":1,"source":{},"files":[],"extra":true}"#).unwrap();
    let output = run(&["manifest", "validate", path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty());
}

#[test]
fn manifest_schema_matches_committed_schema() {
    let output = run(&["manifest", "schema"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let committed =
        std::fs::read_to_string(repo_root().join("schemas/manifest-v1.schema.json")).unwrap();
    assert_eq!(stdout.trim_end(), committed.trim_end());
}
