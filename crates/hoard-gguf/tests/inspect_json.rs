//! Inspect JSON contract tests against the committed synthetic fixtures.
//!
//! The golden byte comparison lives in `tests/golden.rs`; this file checks the
//! structural rules of the version 1 report independently of formatting.

mod common;

use std::path::Path;

use common::RawGguf;
use hoard_gguf::{ByteOrder, Gguf, SCHEMA_VERSION, inspect};

fn report_for(bytes: &[u8], path: Option<&Path>) -> hoard_gguf::InspectReport {
    let gguf = Gguf::parse(bytes).expect("fixture parses");
    inspect(&gguf, path)
}

#[test]
fn minimal_fixture_report_has_no_tensors_and_one_warning() {
    let report = report_for(&common::minimal_gguf(), None);
    assert_eq!(report.schema, 1);
    assert_eq!(report.schema, SCHEMA_VERSION);
    assert_eq!(report.file.path, None);
    assert_eq!(report.file.size, 32);
    assert_eq!(report.file.version, 3);
    assert_eq!(report.file.byte_order, ByteOrder::Little);
    assert_eq!(report.file.alignment, 32);
    assert_eq!(report.file.data_start, 32);
    assert_eq!(report.model.architecture, None);
    assert_eq!(report.model.name, None);
    assert_eq!(report.model.quantization_version, None);
    assert_eq!(report.split, None);
    assert_eq!(report.metadata.count, 0);
    assert_eq!(report.tensors.count, 0);
    assert_eq!(report.tensors.total_bytes, 0);
    assert!(report.tensors.bytes_known);
    assert_eq!(report.tensors.unknown_types, 0);
    assert!(report.quant_mix.is_empty());
    assert_eq!(report.memory.tensor_bytes, 0);
    assert_eq!(report.memory.file_bytes, 32);
    assert_eq!(report.memory.overhead_bytes, 32);
    assert_eq!(report.memory.padding_bytes, 0);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, "missing-architecture");
}

#[test]
fn rich_fixture_report_has_the_contract_sections() {
    let report = report_for(&common::rich_gguf(), Some(Path::new("synthetic-rich.gguf")));
    assert_eq!(report.file.path.as_deref(), Some("synthetic-rich.gguf"));
    assert_eq!(report.file.size, 132_640);
    assert_eq!(report.file.byte_order, ByteOrder::Little);
    assert_eq!(report.file.alignment, 64);
    assert_eq!(report.model.architecture.as_deref(), Some("llama"));
    assert_eq!(report.model.name.as_deref(), Some("synthetic-rich"));
    assert_eq!(report.model.quantization_version, Some(2));
    let split = report.split.expect("split metadata is present");
    assert_eq!((split.no, split.count, split.tensors_count), (1, 2, 48));
    assert_eq!(report.metadata.count, 24);
    assert_eq!(report.metadata.entries.len(), 24);
    assert_eq!(report.tensors.count, 3);
    assert_eq!(report.tensors.total_bytes, 131_496);
    assert!(report.warnings.is_empty());
    assert_eq!(report.quant_mix.len(), 3);
    assert_eq!(report.memory.tensor_bytes, 131_496);
    assert_eq!(
        report.memory.overhead_bytes,
        report.memory.file_bytes - report.memory.tensor_bytes
    );
}

#[test]
fn rich_metadata_entries_are_in_file_order_with_typed_values() {
    let report = report_for(&common::rich_gguf(), None);
    let keys: Vec<&str> = report
        .metadata
        .entries
        .iter()
        .map(|entry| entry.key.as_str())
        .collect();
    assert_eq!(keys[0], "general.architecture");
    assert_eq!(keys[1], "general.name");
    assert_eq!(keys[2], "general.quantization_version");
    assert_eq!(keys[3], "general.alignment");
    assert_eq!(keys[4], "split.no");
    assert_eq!(keys[6], "split.tensors.count");
    assert_eq!(keys[23], "test.array.nested");

    let array = report.metadata.entries[19]
        .array
        .as_ref()
        .expect("test.array.string is an array");
    assert_eq!(array.element_type, "string");
    assert_eq!(array.length, 5);
    assert_eq!(array.sample.len(), 3);
    assert!(array.sample_truncated);
    let nested = report.metadata.entries[23]
        .array
        .as_ref()
        .expect("test.array.nested is an array");
    assert_eq!(nested.element_type, "array");
    assert!(nested.sample.is_empty());
    assert!(nested.sample_truncated);
}

#[test]
fn rich_tensors_are_sorted_by_offset_and_have_file_offsets() {
    let report = report_for(&common::rich_gguf(), None);
    let entries = &report.tensors.entries;
    assert_eq!(entries.len(), 3);
    let offsets: Vec<u64> = entries.iter().map(|entry| entry.offset).collect();
    let mut sorted = offsets.clone();
    sorted.sort_unstable();
    assert_eq!(offsets, sorted);
    for entry in entries {
        assert_eq!(entry.file_offset, report.file.data_start + entry.offset);
        assert_eq!(entry.bytes, Some(entry.elements_for_report()));
    }
}

#[test]
fn quant_mix_percentages_use_two_decimal_basis_points() {
    let report = report_for(&common::rich_gguf(), None);
    let percent: Vec<f64> = report.quant_mix.iter().map(|entry| entry.percent).collect();
    assert_eq!(percent, [99.68, 0.22, 0.1]);
    let bytes: Vec<u64> = report.quant_mix.iter().map(|entry| entry.bytes).collect();
    assert_eq!(bytes, [131_072, 288, 136]);
    for entry in &report.quant_mix {
        let decimals = (entry.percent * 100.0).round() / 100.0;
        assert_eq!(entry.percent, decimals);
    }
}

#[test]
fn compact_json_starts_with_the_schema_version() {
    let bytes = common::rich_gguf();
    let gguf = Gguf::parse(&bytes).unwrap();
    let report = inspect(&gguf, Some(Path::new("synthetic-rich.gguf")));
    let compact = serde_json::to_string(&report).unwrap();
    assert!(
        compact.starts_with("{\"schema\":1,\"file\":{"),
        "unexpected serialization start: {}",
        &compact[..40.min(compact.len())]
    );
    // 64-bit values are serialized as JSON integers, not floats or strings.
    assert!(compact.contains("\"total_bytes\":131496"));
    assert!(compact.contains("\"u64\",\"value\":18446744073709551600"));
}

#[test]
fn u64_metadata_round_trips_without_precision_loss() {
    let mut raw = RawGguf::new();
    raw.meta("test.big", 10, &u64::MAX.to_le_bytes());
    let report = report_for(&raw.build(), None);
    let value = report.metadata.entries[0].value.as_ref().unwrap();
    assert_eq!(value.as_u64(), Some(u64::MAX));
}

#[test]
fn unknown_quantization_types_report_null_bytes() {
    let mut raw = RawGguf::new();
    raw.tensor("future", &[32], 33, 0); // 33 is a removed ggml_type id
    let report = report_for(&raw.build(), None);
    assert_eq!(report.tensors.unknown_types, 1);
    assert!(!report.tensors.bytes_known);
    assert_eq!(report.tensors.entries[0].bytes, None);
    assert_eq!(report.tensors.entries[0].value_type, "unknown");
    assert!(report.quant_mix.is_empty());
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"bytes\":null"));
}

#[test]
fn report_warnings_are_code_message_pairs() {
    let report = report_for(&common::minimal_gguf(), None);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, "missing-architecture");
    assert!(!report.warnings[0].message.is_empty());
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["warnings"][0]["code"], "missing-architecture");
    assert!(json["warnings"][0]["message"].is_string());
}

#[test]
fn schema_function_exposes_the_report_contract() {
    let schema = hoard_gguf::inspect_schema();
    let properties = schema["properties"].as_object().expect("properties");
    for field in [
        "schema",
        "file",
        "model",
        "split",
        "metadata",
        "tensors",
        "quant_mix",
        "memory",
        "warnings",
    ] {
        assert!(properties.contains_key(field), "missing {field}");
    }
    assert_eq!(schema["properties"]["schema"]["type"], "integer");
    assert_eq!(schema["properties"]["file"]["$ref"], "#/$defs/FileReport");
}

/// Helper so the tensor assertion above stays readable.
trait ElementsForReport {
    fn elements_for_report(&self) -> u64;
}

impl ElementsForReport for hoard_gguf::TensorReport {
    fn elements_for_report(&self) -> u64 {
        match self.value_type.as_str() {
            "F32" => self.elements * 4,
            "Q8_0" => self.elements / 32 * 34,
            "Q4_K" => self.elements / 256 * 144,
            other => panic!("unexpected quantization type {other}"),
        }
    }
}
