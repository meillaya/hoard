//! CLI tests for the `hoard-gguf` binary.
//!
//! These tests use the committed synthetic fixture for deterministic output and
//! the downloaded real fixtures (when present) for end-to-end coverage.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_hoard-gguf")
}

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn real_fixture(name: &str) -> Option<PathBuf> {
    let path = manifest_dir().join("../../fixtures/real").join(name);
    path.is_file().then_some(path)
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .current_dir(manifest_dir())
        .args(args)
        .output()
        .expect("failed to run hoard-gguf")
}

fn parse_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn inspect_json_reports_synthetic_fixture() {
    let output = run(&["inspect", "tests/fixtures/synthetic-rich.gguf", "--json"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_json(&output);

    assert_eq!(report["schema"], 1);
    assert_eq!(report["file"]["path"], "tests/fixtures/synthetic-rich.gguf");
    assert_eq!(report["file"]["version"], 3);
    assert_eq!(report["file"]["byte_order"], "little");
    assert_eq!(report["file"]["alignment"], 64);
    assert_eq!(report["split"]["no"], 1);
    assert_eq!(report["split"]["count"], 2);
    assert_eq!(report["tensors"]["count"], 3);
    assert_eq!(report["metadata"]["count"].as_u64(), Some(24));
    assert!(report["warnings"].is_array());
}

#[test]
fn inspect_human_output_is_readable() {
    let output = run(&["inspect", "tests/fixtures/synthetic-rich.gguf"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GGUF v3"), "{stdout}");
    assert!(stdout.contains("synthetic-rich.gguf"), "{stdout}");
    assert!(stdout.contains("quantization"), "{stdout}");
    assert!(stdout.contains("metadata (24 keys)"), "{stdout}");
}

#[test]
fn inspect_tensor_table_lists_entries() {
    let output = run(&["inspect", "tests/fixtures/synthetic-rich.gguf", "--tensors"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tensors by offset"), "{stdout}");
    assert!(stdout.contains("token_embd.weight"), "{stdout}");
}

#[test]
fn schema_command_matches_committed_schema() {
    let output = run(&["schema"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("schema output is UTF-8");
    let committed =
        std::fs::read_to_string(manifest_dir().join("../../schemas/inspect-v1.schema.json"))
            .expect("committed inspect schema");
    assert_eq!(stdout.trim_end(), committed.trim_end());
}

#[test]
fn missing_file_fails_cleanly() {
    let output = run(&["inspect", "tests/fixtures/does-not-exist.gguf"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to open"), "{stderr}");
}

#[test]
fn real_fixture_numbers() {
    let Some(path) = real_fixture("stories260K.gguf") else {
        eprintln!("skipping: run scripts/fetch-fixtures.sh to download the real corpus");
        return;
    };
    let output = Command::new(bin())
        .args(["inspect", path.to_str().unwrap(), "--json"])
        .output()
        .expect("failed to run hoard-gguf");
    assert!(output.status.success());
    let report = parse_json(&output);
    assert_eq!(report["tensors"]["count"], 48);
    assert_eq!(report["metadata"]["count"], 19);
    assert_eq!(report["model"]["architecture"], "llama");
    assert_eq!(report["model"]["name"], "llama");
    assert_eq!(report["tensors"]["total_bytes"], 1_171_200u64);
    assert_eq!(report["memory"]["tensor_bytes"], 1_171_200u64);
    assert_eq!(report["file"]["data_start"], 14_176u64);
    assert_eq!(report["file"]["alignment"], 32);
    assert_eq!(report["split"], Value::Null);
    let mix = report["quant_mix"].as_array().expect("quant_mix");
    assert_eq!(mix.len(), 1);
    assert_eq!(mix[0]["type"], "F32");
    assert_eq!(mix[0]["percent"], 100.0);
}

#[test]
fn real_big_endian_fixture_is_rejected_clearly() {
    let Some(path) = real_fixture("stories260K-be.gguf") else {
        eprintln!("skipping: run scripts/fetch-fixtures.sh to download the real corpus");
        return;
    };
    let output = Command::new(bin())
        .args(["inspect", path.to_str().unwrap(), "--json"])
        .output()
        .expect("failed to run hoard-gguf");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("big-endian"), "{stderr}");
}

#[test]
fn truncated_file_is_rejected() {
    let Some(path) = real_fixture("stories260K.gguf") else {
        eprintln!("skipping: run scripts/fetch-fixtures.sh to download the real corpus");
        return;
    };
    let bytes = std::fs::read(path).expect("fixture is readable");
    let truncated = tempfile::NamedTempFile::new().expect("temp file");
    std::fs::write(truncated.path(), &bytes[..bytes.len() / 2]).expect("write truncated fixture");
    let output = Command::new(bin())
        .args(["inspect", truncated.path().to_str().unwrap(), "--json"])
        .output()
        .expect("failed to run hoard-gguf");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to parse"), "{stderr}");
}
