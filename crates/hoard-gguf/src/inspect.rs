//! The `inspect` report: a stable, versioned JSON view of a GGUF file.
//!
//! [`inspect`] turns a parsed [`Gguf`] into an [`InspectReport`], which the
//! `hoard-gguf inspect` command serializes with `serde_json` (pretty-printed
//! with two spaces and a trailing newline). The shape of the report is frozen
//! at [`SCHEMA_VERSION`] 1; see `schemas/inspect-v1.schema.json`.
//!
//! The report deliberately keeps 64-bit counts and sizes as JSON integers even
//! though JSON numbers are IEEE-754 doubles: files larger than 2^53 bytes, or
//! with more than 2^53 elements, would lose precision in strict
//! JavaScript-style consumers. Rust and `serde_json` preserve them exactly, but
//! downstream tools should parse these fields as 64-bit integers (for example
//! with a big-integer JSON reader) if they care about the top bits.

use std::collections::BTreeMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::Serialize;

use crate::file::{
    GENERAL_ARCHITECTURE_KEY, GENERAL_NAME_KEY, GENERAL_QUANTIZATION_VERSION_KEY, Gguf,
};
use crate::header::ByteOrder;
use crate::metadata::{MetadataEntry, MetadataValue, ValueType};
use crate::quant;
use crate::tensor::TensorInfo;

/// Version of the `inspect` JSON document.
pub const SCHEMA_VERSION: u32 = 1;

/// The complete `inspect` report for one GGUF file.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InspectReport {
    /// Schema version, always [`SCHEMA_VERSION`] (currently `1`).
    pub schema: u32,
    /// File-level facts.
    pub file: FileReport,
    /// Model-level metadata.
    pub model: ModelReport,
    /// Split information, or `null` when the file is not part of a split model.
    pub split: Option<SplitReport>,
    /// All metadata entries, in file order.
    pub metadata: MetadataReport,
    /// Tensor infos, sorted by offset then name.
    pub tensors: TensorsReport,
    /// Per-quantization-type summary, largest first.
    pub quant_mix: Vec<QuantMixEntry>,
    /// Byte accounting for the file.
    pub memory: MemoryReport,
    /// Non-fatal diagnostics, in parser emission order.
    pub warnings: Vec<WarningReport>,
}

/// File-level facts from the header and the computed data section start.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FileReport {
    /// Path the file was read from, or `null` when inspecting a byte buffer.
    pub path: Option<String>,
    /// File size in bytes.
    pub size: u64,
    /// GGUF version (2 or 3).
    pub version: u32,
    /// Byte order, always `"little"`.
    pub byte_order: ByteOrder,
    /// Tensor data alignment in bytes.
    pub alignment: u32,
    /// Absolute file offset where the tensor data section starts.
    pub data_start: u64,
}

/// Model-level metadata extracted from well-known keys.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelReport {
    /// `general.architecture`, or `null` when absent.
    pub architecture: Option<String>,
    /// `general.name`, or `null` when absent.
    pub name: Option<String>,
    /// `general.quantization_version`, or `null` when absent.
    pub quantization_version: Option<u32>,
}

/// Split-model information from the `split.*` metadata keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SplitReport {
    /// Zero-based index of this part.
    pub no: u16,
    /// Total number of parts.
    pub count: u16,
    /// Total tensor count across all parts.
    pub tensors_count: i32,
}

/// Metadata overview: total count plus one entry per key/value pair.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MetadataReport {
    /// Number of metadata entries.
    pub count: u64,
    /// Entries, in file order.
    pub entries: Vec<MetadataEntryReport>,
}

/// One metadata entry.
///
/// Scalar entries serialize as `{"key": ..., "type": "u32", "value": ...}`;
/// array entries as
/// `{"key": ..., "type": "array", "array": {...}}`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MetadataEntryReport {
    /// Metadata key.
    pub key: String,
    /// Value type name (`"u8"`, `"string"`, `"array"`, ...).
    #[serde(rename = "type")]
    pub value_type: String,
    /// The value for scalar entries; omitted for arrays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Array details; omitted for scalar entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array: Option<ArrayReport>,
}

/// Details of an array metadata value.
///
/// `sample` holds up to three scalar elements; nested arrays are not recursed
/// into, so their `sample` is empty and `sample_truncated` is `true` whenever
/// the nested array has elements.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ArrayReport {
    /// Element type name (`"string"`, `"f32"`, `"array"`, ...).
    pub element_type: String,
    /// Number of elements in the array.
    pub length: u64,
    /// Up to three leading scalar elements.
    pub sample: Vec<serde_json::Value>,
    /// Whether `sample` omits elements that are present in the array.
    pub sample_truncated: bool,
}

/// Tensor overview: counts plus one entry per tensor.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TensorsReport {
    /// Number of tensors.
    pub count: u64,
    /// Sum of the data sizes of tensors with known quantization types.
    pub total_bytes: u64,
    /// Whether every tensor's data size is known.
    pub bytes_known: bool,
    /// Number of tensors with an unknown quantization type.
    pub unknown_types: u64,
    /// Tensor entries, sorted by offset then name.
    pub entries: Vec<TensorReport>,
}

/// One tensor entry.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TensorReport {
    /// Tensor name.
    pub name: String,
    /// Quantization type name (`"F32"`, `"Q8_0"`, ...) or `"unknown"`.
    #[serde(rename = "type")]
    pub value_type: String,
    /// Numeric `ggml_type` id.
    pub type_id: u32,
    /// Dimensions, innermost first.
    pub shape: Vec<u64>,
    /// Total element count.
    pub elements: u64,
    /// Data size in bytes, or `null` when the quantization type is unknown.
    pub bytes: Option<u64>,
    /// Offset relative to `file.data_start`.
    pub offset: u64,
    /// Absolute file offset of the tensor data.
    pub file_offset: u64,
}

/// One quantization type in the `quant_mix` summary.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct QuantMixEntry {
    /// Quantization type name.
    #[serde(rename = "type")]
    pub value_type: String,
    /// Numeric `ggml_type` id.
    pub type_id: u32,
    /// Number of tensors with this type.
    pub tensors: u64,
    /// Sum of the element counts of those tensors.
    pub elements: u64,
    /// Sum of the data sizes of those tensors.
    pub bytes: u64,
    /// Share of the known tensor bytes, in percent with two decimals.
    pub percent: f64,
}

/// Byte accounting for the file.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
pub struct MemoryReport {
    /// Sum of the known tensor data sizes.
    pub tensor_bytes: u64,
    /// Total file size.
    pub file_bytes: u64,
    /// Bytes that are not tensor data (`file_bytes - tensor_bytes`).
    pub overhead_bytes: u64,
    /// Alignment gaps between tensors, including before the first tensor.
    pub padding_bytes: u64,
}

/// One non-fatal parser diagnostic.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WarningReport {
    /// Stable kebab-case warning code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

/// Build the inspect report for a parsed GGUF file.
///
/// `path` is recorded in `file.path`; pass `None` when inspecting an in-memory
/// buffer. The function is infallible: incomplete, wrongly typed, or
/// out-of-range split metadata is reported as a `null` split section, with the
/// reason in the warnings array (a wrongly typed key is additionally surfaced
/// as an error by [`Gguf::split`]).
#[must_use]
pub fn inspect(gguf: &Gguf<'_>, path: Option<&Path>) -> InspectReport {
    let header = gguf.header();
    InspectReport {
        schema: SCHEMA_VERSION,
        file: FileReport {
            path: path.map(|path| path.to_string_lossy().into_owned()),
            size: gguf.len(),
            version: header.version,
            byte_order: header.byte_order,
            alignment: gguf.alignment(),
            data_start: gguf.data_start(),
        },
        model: ModelReport {
            architecture: metadata_string(gguf, GENERAL_ARCHITECTURE_KEY),
            name: metadata_string(gguf, GENERAL_NAME_KEY),
            quantization_version: metadata_u32(gguf, GENERAL_QUANTIZATION_VERSION_KEY),
        },
        // A split section is reported only when it is usable: complete,
        // well-typed, and with an in-range shard index. Incomplete, mistyped,
        // or out-of-range split metadata is `null` here and reported through
        // the warnings array instead (docs/inspect.md).
        split: gguf
            .split()
            .ok()
            .flatten()
            .filter(|info| info.no < info.count)
            .map(|info| SplitReport {
                no: info.no,
                count: info.count,
                tensors_count: info.tensors_count,
            }),
        metadata: metadata_report(gguf),
        tensors: tensors_report(gguf),
        quant_mix: quant_mix(gguf),
        memory: memory_report(gguf),
        warnings: gguf
            .warnings()
            .iter()
            .map(|warning| WarningReport {
                code: warning.code().to_owned(),
                message: warning.message().to_owned(),
            })
            .collect(),
    }
}

fn metadata_string(gguf: &Gguf<'_>, key: &str) -> Option<String> {
    gguf.metadata()
        .iter()
        .find(|entry| entry.key() == key)
        .and_then(|entry| entry.value().as_str())
        .map(ToOwned::to_owned)
}

fn metadata_u32(gguf: &Gguf<'_>, key: &str) -> Option<u32> {
    gguf.metadata()
        .iter()
        .find(|entry| entry.key() == key)
        .and_then(|entry| entry.value().as_u32())
}

fn metadata_report(gguf: &Gguf<'_>) -> MetadataReport {
    MetadataReport {
        count: gguf.metadata().len() as u64,
        entries: gguf.metadata().iter().map(metadata_entry_report).collect(),
    }
}

fn metadata_entry_report(entry: &MetadataEntry<'_>) -> MetadataEntryReport {
    let key = entry.key().to_owned();
    match entry.value() {
        MetadataValue::Array(array) => {
            let element_type = array.element_type();
            let length = array.len();
            // Nested arrays are not sampled: recursion would make the report
            // unbounded for adversarial input.
            let (sample, sample_truncated) = if element_type == ValueType::Array {
                (Vec::new(), length > 0)
            } else {
                let sample: Vec<serde_json::Value> =
                    array.values().iter().take(3).map(scalar_json).collect();
                (sample, length > 3)
            };
            MetadataEntryReport {
                key,
                value_type: ValueType::Array.name().to_owned(),
                value: None,
                array: Some(ArrayReport {
                    element_type: element_type.name().to_owned(),
                    length,
                    sample,
                    sample_truncated,
                }),
            }
        }
        value => MetadataEntryReport {
            key,
            value_type: value.value_type().name().to_owned(),
            value: Some(scalar_json(value)),
            array: None,
        },
    }
}

fn scalar_json(value: &MetadataValue<'_>) -> serde_json::Value {
    match value {
        MetadataValue::U8(value) => serde_json::Value::from(*value),
        MetadataValue::I8(value) => serde_json::Value::from(*value),
        MetadataValue::U16(value) => serde_json::Value::from(*value),
        MetadataValue::I16(value) => serde_json::Value::from(*value),
        MetadataValue::U32(value) => serde_json::Value::from(*value),
        MetadataValue::I32(value) => serde_json::Value::from(*value),
        MetadataValue::F32(value) => number_json(f64::from(*value)),
        MetadataValue::Bool(value) => serde_json::Value::Bool(*value),
        MetadataValue::String(value) => serde_json::Value::String((*value).to_owned()),
        // Arrays are never sampled, but stay total rather than panicking.
        MetadataValue::Array(_) => serde_json::Value::Null,
        MetadataValue::U64(value) => serde_json::Value::from(*value),
        MetadataValue::I64(value) => serde_json::Value::from(*value),
        MetadataValue::F64(value) => number_json(*value),
    }
}

fn number_json(value: f64) -> serde_json::Value {
    // JSON has no NaN or infinity; those become `null`.
    serde_json::Number::from_f64(value).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

fn sorted_tensors<'a>(gguf: &'a Gguf<'_>) -> Vec<&'a TensorInfo<'a>> {
    let mut tensors: Vec<&TensorInfo<'_>> = gguf.tensors().iter().collect();
    tensors.sort_by(|left, right| {
        left.offset()
            .cmp(&right.offset())
            .then_with(|| left.name().cmp(right.name()))
    });
    tensors
}

fn tensors_report(gguf: &Gguf<'_>) -> TensorsReport {
    let aggregate = gguf.tensor_bytes();
    TensorsReport {
        count: gguf.tensors().len() as u64,
        total_bytes: aggregate.tensor_bytes,
        bytes_known: aggregate.bytes_known,
        unknown_types: aggregate.unknown_types,
        entries: sorted_tensors(gguf)
            .into_iter()
            .map(|info| TensorReport {
                name: info.name().to_owned(),
                value_type: info.type_name().to_owned(),
                type_id: info.type_id(),
                shape: info.shape().to_vec(),
                elements: info.elements(),
                bytes: info.bytes(),
                offset: info.offset(),
                file_offset: info.file_offset(),
            })
            .collect(),
    }
}

fn quant_mix(gguf: &Gguf<'_>) -> Vec<QuantMixEntry> {
    #[derive(Default)]
    struct Aggregate {
        tensors: u64,
        elements: u64,
        bytes: u64,
    }

    let mut by_type: BTreeMap<u32, Aggregate> = BTreeMap::new();
    for info in gguf.tensors() {
        let Some(_) = info.quant() else { continue };
        let Some(bytes) = info.bytes() else { continue };
        let aggregate = by_type.entry(info.type_id()).or_default();
        aggregate.tensors = aggregate.tensors.saturating_add(1);
        aggregate.elements = aggregate.elements.saturating_add(info.elements());
        aggregate.bytes = aggregate.bytes.saturating_add(bytes);
    }

    let total: u128 = by_type
        .values()
        .map(|aggregate| u128::from(aggregate.bytes))
        .sum();
    let mut entries: Vec<QuantMixEntry> = by_type
        .into_iter()
        .map(|(type_id, aggregate)| {
            // Percent with two decimals, computed with integer basis points so
            // the JSON does not depend on floating-point rounding.
            let basis_points = if total == 0 {
                0
            } else {
                let scaled = u128::from(aggregate.bytes) * 10_000 + total / 2;
                (scaled / total) as u64
            };
            QuantMixEntry {
                value_type: quant::name_of(type_id).to_owned(),
                type_id,
                tensors: aggregate.tensors,
                elements: aggregate.elements,
                bytes: aggregate.bytes,
                percent: basis_points as f64 / 100.0,
            }
        })
        .collect();
    entries.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.type_id.cmp(&right.type_id))
    });
    entries
}

fn memory_report(gguf: &Gguf<'_>) -> MemoryReport {
    let tensor_bytes = gguf.tensor_bytes().tensor_bytes;
    let file_bytes = gguf.len();
    MemoryReport {
        tensor_bytes,
        file_bytes,
        overhead_bytes: file_bytes.saturating_sub(tensor_bytes),
        padding_bytes: padding_bytes(gguf),
    }
}

fn padding_bytes(gguf: &Gguf<'_>) -> u64 {
    let mut padding = 0_u64;
    let mut previous_end = 0_u64;
    let mut previous_known = true;
    for info in sorted_tensors(gguf) {
        if previous_known {
            padding = padding.saturating_add(info.offset().saturating_sub(previous_end));
        }
        match info.bytes() {
            Some(bytes) => {
                previous_end = info.offset().saturating_add(bytes);
                previous_known = true;
            }
            None => previous_known = false,
        }
    }
    padding
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{self, Builder, TensorSpec, Value};
    use crate::{Gguf, Warning};

    fn report_for(bytes: &[u8], path: Option<&Path>) -> InspectReport {
        let gguf = Gguf::parse(bytes).unwrap();
        inspect(&gguf, path)
    }

    fn rich_report() -> InspectReport {
        report_for(&fixture::rich(), Some(Path::new("synthetic-rich.gguf")))
    }

    fn entry<'a>(report: &'a InspectReport, key: &str) -> &'a MetadataEntryReport {
        report
            .metadata
            .entries
            .iter()
            .find(|entry| entry.key == key)
            .unwrap_or_else(|| panic!("no metadata entry {key:?}"))
    }

    #[test]
    fn minimal_report_has_stable_top_level_shape() {
        let report = report_for(&fixture::minimal(), None);
        assert_eq!(report.schema, SCHEMA_VERSION);
        assert_eq!(report.schema, 1);
        assert_eq!(report.file.path, None);
        assert_eq!(report.file.size, 32);
        assert_eq!(report.file.version, 3);
        assert_eq!(report.file.byte_order, ByteOrder::Little);
        assert_eq!(report.file.alignment, 32);
        assert_eq!(report.file.data_start, 32);
        assert_eq!(report.model.architecture, None);
        assert_eq!(report.model.name, None);
        assert_eq!(report.model.quantization_version, None);
        assert!(report.split.is_none());
        assert_eq!(report.metadata.count, 0);
        assert!(report.metadata.entries.is_empty());
        assert_eq!(report.tensors.count, 0);
        assert_eq!(report.tensors.total_bytes, 0);
        assert!(report.tensors.bytes_known);
        assert_eq!(report.tensors.unknown_types, 0);
        assert!(report.tensors.entries.is_empty());
        assert!(report.quant_mix.is_empty());
        assert_eq!(report.memory.tensor_bytes, 0);
        assert_eq!(report.memory.file_bytes, 32);
        assert_eq!(report.memory.overhead_bytes, 32);
        assert_eq!(report.memory.padding_bytes, 0);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].code, Warning::MISSING_ARCHITECTURE);
    }

    #[test]
    fn rich_report_matches_the_fixture() {
        let report = rich_report();
        assert_eq!(report.file.path.as_deref(), Some("synthetic-rich.gguf"));
        assert_eq!(report.file.version, 3);
        assert_eq!(report.file.alignment, 64);
        assert_eq!(report.file.data_start % 64, 0);
        assert_eq!(report.model.architecture.as_deref(), Some("llama"));
        assert_eq!(report.model.name.as_deref(), Some("synthetic-rich"));
        assert_eq!(report.model.quantization_version, Some(2));
        assert_eq!(
            report.split,
            Some(SplitReport {
                no: 1,
                count: 2,
                tensors_count: 48,
            })
        );
        assert_eq!(report.metadata.count, 24);
        assert_eq!(report.metadata.entries.len(), 24);
        assert!(report.warnings.is_empty());

        assert_eq!(report.tensors.count, 3);
        assert_eq!(report.tensors.total_bytes, 131_072 + 136 + 288);
        assert!(report.tensors.bytes_known);
        assert_eq!(report.tensors.unknown_types, 0);
        let names: Vec<&str> = report
            .tensors
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "token_embd.weight",
                "blk.0.attn_q.weight",
                "blk.0.ffn_down.weight"
            ]
        );
        let embedding = &report.tensors.entries[0];
        assert_eq!(embedding.value_type, "F32");
        assert_eq!(embedding.type_id, 0);
        assert_eq!(embedding.shape, [64, 512]);
        assert_eq!(embedding.elements, 32_768);
        assert_eq!(embedding.bytes, Some(131_072));
        assert_eq!(embedding.offset, 0);
        assert_eq!(embedding.file_offset, report.file.data_start);
    }

    #[test]
    fn rich_metadata_entries_cover_all_value_types() {
        let report = rich_report();
        assert_eq!(entry(&report, "test.u8").value_type, "u8");
        assert_eq!(entry(&report, "test.i8").value_type, "i8");
        assert_eq!(entry(&report, "test.u16").value_type, "u16");
        assert_eq!(entry(&report, "test.i16").value_type, "i16");
        assert_eq!(entry(&report, "test.u32").value_type, "u32");
        assert_eq!(entry(&report, "test.i32").value_type, "i32");
        assert_eq!(entry(&report, "test.f32").value_type, "f32");
        assert_eq!(entry(&report, "test.bool").value_type, "bool");
        assert_eq!(entry(&report, "test.string").value_type, "string");
        assert_eq!(entry(&report, "test.u64").value_type, "u64");
        assert_eq!(entry(&report, "test.i64").value_type, "i64");
        assert_eq!(entry(&report, "test.f64").value_type, "f64");
        assert_eq!(entry(&report, "test.array.string").value_type, "array");

        let types: std::collections::HashSet<&str> = report
            .metadata
            .entries
            .iter()
            .map(|entry| entry.value_type.as_str())
            .collect();
        for name in [
            "u8", "i8", "u16", "i16", "u32", "i32", "f32", "bool", "string", "array", "u64", "i64",
            "f64",
        ] {
            assert!(types.contains(name), "missing metadata type {name}");
        }

        assert_eq!(
            entry(&report, "test.u8").value,
            Some(serde_json::json!(200))
        );
        assert_eq!(
            entry(&report, "test.i8").value,
            Some(serde_json::json!(-100))
        );
        assert_eq!(entry(&report, "test.bool").value, Some(true.into()));
        assert_eq!(entry(&report, "test.string").value, Some("hello".into()));
    }

    #[test]
    fn array_entries_report_element_type_length_and_sample() {
        let report = rich_report();
        let strings = entry(&report, "test.array.string");
        let array = strings.array.as_ref().unwrap();
        assert_eq!(array.element_type, "string");
        assert_eq!(array.length, 5);
        assert_eq!(
            array.sample,
            [
                serde_json::Value::from("a"),
                serde_json::Value::from("bb"),
                serde_json::Value::from("ccc")
            ]
        );
        assert!(array.sample_truncated);

        let floats = entry(&report, "test.array.f32");
        let array = floats.array.as_ref().unwrap();
        assert_eq!(array.element_type, "f32");
        assert_eq!(array.length, 3);
        assert!(!array.sample_truncated);

        let nested = entry(&report, "test.array.nested");
        let array = nested.array.as_ref().unwrap();
        assert_eq!(array.element_type, "array");
        assert_eq!(array.length, 2);
        assert!(array.sample.is_empty());
        assert!(array.sample_truncated);
    }

    #[test]
    fn quant_mix_is_sorted_and_percent_uses_basis_points() {
        let report = rich_report();
        let types: Vec<&str> = report
            .quant_mix
            .iter()
            .map(|entry| entry.value_type.as_str())
            .collect();
        assert_eq!(types, ["F32", "Q4_K", "Q8_0"]);
        assert_eq!(report.quant_mix[0].type_id, 0);
        assert_eq!(report.quant_mix[0].tensors, 1);
        assert_eq!(report.quant_mix[0].elements, 32_768);
        assert_eq!(report.quant_mix[0].bytes, 131_072);
        assert_eq!(report.quant_mix[0].percent, 99.68);
        assert_eq!(report.quant_mix[1].percent, 0.22);
        assert_eq!(report.quant_mix[2].percent, 0.1);
        let total: u64 = report.quant_mix.iter().map(|entry| entry.bytes).sum();
        assert_eq!(total, report.tensors.total_bytes);
    }

    #[test]
    fn memory_report_counts_padding_between_tensors() {
        let report = rich_report();
        // F32 ends at 131072, Q8_0 starts there (no gap); Q8_0 ends at
        // 131208, Q4_K starts at 131264 (56 bytes of alignment padding).
        assert_eq!(report.memory.padding_bytes, 56);
        assert_eq!(report.memory.tensor_bytes, 131_496);
        assert_eq!(
            report.memory.overhead_bytes,
            report.memory.file_bytes - report.memory.tensor_bytes
        );
        assert_eq!(
            report.memory.file_bytes,
            report.file.data_start + report.memory.tensor_bytes + report.memory.padding_bytes,
            // The fixture ends exactly at the last tensor end, so padding is
            // the only gap in the data section.
        );
    }

    #[test]
    fn unknown_quantization_type_is_reflected_in_the_report() {
        let bytes = Builder::new()
            .tensor(TensorSpec::new("future", &[32], 4))
            .build();
        let report = report_for(&bytes, None);
        assert_eq!(report.tensors.count, 1);
        assert!(!report.tensors.bytes_known);
        assert_eq!(report.tensors.unknown_types, 1);
        assert_eq!(report.tensors.total_bytes, 0);
        let tensor = &report.tensors.entries[0];
        assert_eq!(tensor.value_type, "unknown");
        assert_eq!(tensor.type_id, 4);
        assert_eq!(tensor.bytes, None);
        assert!(report.quant_mix.is_empty());
        assert_eq!(report.memory.tensor_bytes, 0);
    }

    #[test]
    fn non_finite_metadata_floats_become_json_null() {
        let bytes = Builder::new()
            .metadata("test.nan", Value::F32(f32::NAN))
            .build();
        let report = report_for(&bytes, None);
        let entry = entry(&report, "test.nan");
        assert_eq!(entry.value_type, "f32");
        assert_eq!(entry.value, Some(serde_json::Value::Null));
    }

    #[test]
    fn serialized_json_has_the_contract_field_names() {
        let report = rich_report();
        let json = serde_json::to_value(&report).unwrap();
        let object = json.as_object().unwrap();
        let top_level: Vec<&str> = object.keys().map(String::as_str).collect();
        assert_eq!(
            top_level,
            [
                "file",
                "memory",
                "metadata",
                "model",
                "quant_mix",
                "schema",
                "split",
                "tensors",
                "warnings"
            ]
        );
        assert_eq!(json["schema"], 1);
        assert_eq!(json["file"]["byte_order"], "little");
        assert_eq!(json["file"]["path"], "synthetic-rich.gguf");
        assert_eq!(json["split"]["no"], 1);
        assert_eq!(json["split"]["tensors_count"], 48);

        // Scalar metadata entry: key, type, value in that order.
        let scalar = serde_json::to_value(entry(&report, "test.u8")).unwrap();
        assert_eq!(
            scalar,
            serde_json::json!({"key": "test.u8", "type": "u8", "value": 200})
        );
        let scalar_object = scalar.as_object().unwrap();
        assert!(scalar_object.get("array").is_none());

        // Array metadata entry: key, type, array (no value key).
        let array = serde_json::to_value(entry(&report, "test.array.string")).unwrap();
        let array_object = array.as_object().unwrap();
        assert!(array_object.get("value").is_none());
        assert!(array_object.get("array").is_some());

        // Tensor entries serialize `type` and nullable `bytes`.
        let tensor = serde_json::to_value(&report.tensors.entries[0]).unwrap();
        assert_eq!(tensor["type"], "F32");
        assert_eq!(tensor["shape"], serde_json::json!([64, 512]));
        assert_eq!(tensor["bytes"], 131_072);
        assert_eq!(tensor["file_offset"], report.file.data_start);
    }

    #[test]
    fn serialized_json_preserves_u64_integers() {
        let bytes = Builder::new()
            .metadata("test.big", Value::U64(u64::MAX))
            .build();
        let report = report_for(&bytes, None);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("18446744073709551615"));
        assert_eq!(entry(&report, "test.big").value, Some(u64::MAX.into()));
    }

    #[test]
    fn report_is_infallible_for_files_with_wrongly_typed_split_metadata() {
        let bytes = Builder::new()
            .metadata("split.no", Value::String("one".into()))
            .metadata("split.count", Value::U16(2))
            .metadata("split.tensors.count", Value::I32(48))
            .build();
        let report = report_for(&bytes, None);
        assert!(report.split.is_none());
    }

    #[test]
    fn out_of_range_split_index_is_absent_from_the_report() {
        let bytes = Builder::new()
            .metadata("split.no", Value::U16(2))
            .metadata("split.count", Value::U16(2))
            .metadata("split.tensors.count", Value::I32(96))
            .build();
        let report = report_for(&bytes, None);
        assert!(report.split.is_none());
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.code == Warning::SPLIT_INDEX_OUT_OF_RANGE)
        );
    }

    #[test]
    fn in_range_split_index_is_reported() {
        let bytes = Builder::new()
            .metadata("split.no", Value::U16(0))
            .metadata("split.count", Value::U16(2))
            .metadata("split.tensors.count", Value::I32(96))
            .build();
        let report = report_for(&bytes, None);
        let split = report.split.expect("in-range split metadata is reported");
        assert_eq!((split.no, split.count, split.tensors_count), (0, 2, 96));
    }
}
