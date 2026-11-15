//! Split-model metadata (`split.no`, `split.count`, `split.tensors.count`).
//!
//! Large models are sometimes distributed as several GGUF parts. `llama.cpp`
//! records the part number, the part count, and the total tensor count across
//! all parts in three metadata keys. Those keys are optional, so split
//! metadata is analysed after parsing rather than treated as fatal:
//!
//! * all three keys present with the expected types: a [`SplitInfo`] is
//!   returned (with a warning when `split.no >= split.count`);
//! * only some keys present: `split-metadata-incomplete` warning and no info;
//! * a key present with an unexpected type: [`Error::SplitMetadataType`] from
//!   [`crate::Gguf::split`].

use crate::error::{Error, Result, Warning};
use crate::metadata::{MetadataEntry, ValueType};

/// Metadata key holding the zero-based index of this split part.
pub const SPLIT_NO_KEY: &str = "split.no";

/// Metadata key holding the number of parts this model is split into.
pub const SPLIT_COUNT_KEY: &str = "split.count";

/// Metadata key holding the total tensor count across all parts.
pub const SPLIT_TENSORS_COUNT_KEY: &str = "split.tensors.count";

/// Split information parsed from the `split.*` metadata keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitInfo {
    /// Zero-based index of this part.
    pub no: u16,
    /// Total number of parts.
    pub count: u16,
    /// Total tensor count across all parts.
    pub tensors_count: i32,
}

/// How the `split.*` metadata keys were found (or not found).
#[derive(Debug, Clone, Copy)]
pub(crate) enum SplitAnalysis {
    /// None of the split keys are present.
    None,
    /// Only some of the split keys are present.
    Incomplete,
    /// All split keys are present and well-typed.
    Complete(SplitInfo),
    /// A split key is present with an unexpected value type.
    Invalid {
        key: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
}

impl SplitAnalysis {
    /// Convert the analysis into the result returned by [`crate::Gguf::split`].
    pub(crate) fn result(self) -> Result<Option<SplitInfo>> {
        match self {
            Self::None | Self::Incomplete => Ok(None),
            Self::Complete(info) => Ok(Some(info)),
            Self::Invalid {
                key,
                expected,
                actual,
            } => Err(Error::SplitMetadataType {
                key,
                expected,
                actual,
            }),
        }
    }
}

fn find<'a, 'b>(metadata: &'b [MetadataEntry<'a>], key: &str) -> Option<&'b MetadataEntry<'a>> {
    metadata.iter().find(|entry| entry.key() == key)
}

/// Analyse the split metadata, pushing `split-metadata-incomplete` and
/// `split-index-out-of-range` warnings when appropriate.
pub(crate) fn analyze(
    metadata: &[MetadataEntry<'_>],
    warnings: &mut Vec<Warning>,
) -> SplitAnalysis {
    let no_entry = find(metadata, SPLIT_NO_KEY);
    let count_entry = find(metadata, SPLIT_COUNT_KEY);
    let tensors_count_entry = find(metadata, SPLIT_TENSORS_COUNT_KEY);

    let mut found: Vec<&'static str> = Vec::new();
    let mut missing: Vec<&'static str> = Vec::new();
    for (key, entry) in [
        (SPLIT_NO_KEY, &no_entry),
        (SPLIT_COUNT_KEY, &count_entry),
        (SPLIT_TENSORS_COUNT_KEY, &tensors_count_entry),
    ] {
        if entry.is_some() {
            found.push(key);
        } else {
            missing.push(key);
        }
    }
    if found.is_empty() {
        return SplitAnalysis::None;
    }
    if !missing.is_empty() {
        warnings.push(Warning::new(
            Warning::SPLIT_METADATA_INCOMPLETE,
            format!("split metadata is incomplete: found {found:?} but missing {missing:?}"),
        ));
        return SplitAnalysis::Incomplete;
    }

    // All three keys are present: `found`/`missing` describes them exactly, so
    // the option unwraps below cannot fail.
    let (Some(no_entry), Some(count_entry), Some(tensors_count_entry)) =
        (no_entry, count_entry, tensors_count_entry)
    else {
        return SplitAnalysis::Incomplete;
    };

    let Some(no) = no_entry.value().as_u16() else {
        return invalid(SPLIT_NO_KEY, ValueType::U16.name(), no_entry.value_type());
    };
    let Some(count) = count_entry.value().as_u16() else {
        return invalid(
            SPLIT_COUNT_KEY,
            ValueType::U16.name(),
            count_entry.value_type(),
        );
    };
    let Some(tensors_count) = tensors_count_entry.value().as_i32() else {
        return invalid(
            SPLIT_TENSORS_COUNT_KEY,
            ValueType::I32.name(),
            tensors_count_entry.value_type(),
        );
    };

    if no >= count {
        warnings.push(Warning::new(
            Warning::SPLIT_INDEX_OUT_OF_RANGE,
            format!("split.no is {no} but split.count is {count} (valid indices are 0..{count})"),
        ));
    }

    SplitAnalysis::Complete(SplitInfo {
        no,
        count,
        tensors_count,
    })
}

fn invalid(key: &'static str, expected: &'static str, actual: ValueType) -> SplitAnalysis {
    SplitAnalysis::Invalid {
        key,
        expected,
        actual: actual.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Cursor;
    use crate::metadata::parse_metadata;

    fn encode_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    fn encode_entry(out: &mut Vec<u8>, key: &str, type_id: u32, payload: &[u8]) {
        encode_string(out, key);
        out.extend_from_slice(&type_id.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn analyze_bytes(bytes: &[u8], count: u64) -> (SplitAnalysis, Vec<Warning>) {
        let mut cursor = Cursor::new(bytes);
        let mut warnings = Vec::new();
        let metadata = parse_metadata(&mut cursor, count, &mut warnings).unwrap();
        let analysis = analyze(&metadata, &mut warnings);
        (analysis, warnings)
    }

    fn complete_entries(no: u16, count: u16, tensors: i32) -> Vec<u8> {
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, SPLIT_NO_KEY, 2, &no.to_le_bytes());
        encode_entry(&mut bytes, SPLIT_COUNT_KEY, 2, &count.to_le_bytes());
        encode_entry(
            &mut bytes,
            SPLIT_TENSORS_COUNT_KEY,
            5,
            &tensors.to_le_bytes(),
        );
        bytes
    }

    #[test]
    fn complete_split_metadata_is_parsed() {
        let bytes = complete_entries(1, 2, 48);
        let (analysis, warnings) = analyze_bytes(&bytes, 3);
        assert!(warnings.is_empty());
        let info = analysis.result().unwrap().unwrap();
        assert_eq!(
            info,
            SplitInfo {
                no: 1,
                count: 2,
                tensors_count: 48
            }
        );
    }

    #[test]
    fn no_split_metadata_is_none() {
        let mut bytes = Vec::new();
        let mut payload = Vec::new();
        encode_string(&mut payload, "llama");
        encode_entry(&mut bytes, "general.architecture", 8, &payload);
        let (analysis, warnings) = analyze_bytes(&bytes, 1);
        assert!(warnings.is_empty());
        assert_eq!(analysis.result().unwrap(), None);
    }

    #[test]
    fn partial_split_metadata_warns_and_returns_none() {
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, SPLIT_NO_KEY, 2, &1_u16.to_le_bytes());
        encode_entry(&mut bytes, SPLIT_COUNT_KEY, 2, &2_u16.to_le_bytes());
        let (analysis, warnings) = analyze_bytes(&bytes, 2);
        assert!(analysis.result().unwrap().is_none());
        assert_eq!(
            warnings.iter().map(Warning::code).collect::<Vec<_>>(),
            [Warning::SPLIT_METADATA_INCOMPLETE]
        );
        assert!(warnings[0].message().contains("split.tensors.count"));
        assert!(warnings[0].message().contains("split.no"));
    }

    #[test]
    fn split_index_out_of_range_warns_but_keeps_info() {
        let bytes = complete_entries(2, 2, 96);
        let (analysis, warnings) = analyze_bytes(&bytes, 3);
        let info = analysis.result().unwrap().unwrap();
        assert_eq!(info.no, 2);
        assert_eq!(
            warnings.iter().map(Warning::code).collect::<Vec<_>>(),
            [Warning::SPLIT_INDEX_OUT_OF_RANGE]
        );
    }

    #[test]
    fn wrong_split_key_types_produce_errors() {
        // split.no as a u32 instead of u16.
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, SPLIT_NO_KEY, 4, &0_u32.to_le_bytes());
        encode_entry(&mut bytes, SPLIT_COUNT_KEY, 2, &2_u16.to_le_bytes());
        encode_entry(
            &mut bytes,
            SPLIT_TENSORS_COUNT_KEY,
            5,
            &48_i32.to_le_bytes(),
        );
        let (analysis, warnings) = analyze_bytes(&bytes, 3);
        assert!(warnings.is_empty());
        match analysis.result().unwrap_err() {
            Error::SplitMetadataType {
                key,
                expected,
                actual,
            } => {
                assert_eq!(key, "split.no");
                assert_eq!(expected, "u16");
                assert_eq!(actual, "u32");
            }
            other => panic!("unexpected error: {other}"),
        }

        // split.count as a string.
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, SPLIT_NO_KEY, 2, &0_u16.to_le_bytes());
        let mut payload = Vec::new();
        encode_string(&mut payload, "two");
        encode_entry(&mut bytes, SPLIT_COUNT_KEY, 8, &payload);
        encode_entry(
            &mut bytes,
            SPLIT_TENSORS_COUNT_KEY,
            5,
            &48_i32.to_le_bytes(),
        );
        let (analysis, _) = analyze_bytes(&bytes, 3);
        assert_eq!(analysis.result().unwrap_err().code(), "split-metadata-type");

        // split.tensors.count as a u64 instead of i32.
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, SPLIT_NO_KEY, 2, &0_u16.to_le_bytes());
        encode_entry(&mut bytes, SPLIT_COUNT_KEY, 2, &2_u16.to_le_bytes());
        encode_entry(
            &mut bytes,
            SPLIT_TENSORS_COUNT_KEY,
            10,
            &48_u64.to_le_bytes(),
        );
        let (analysis, _) = analyze_bytes(&bytes, 3);
        match analysis.result().unwrap_err() {
            Error::SplitMetadataType { key, .. } => assert_eq!(key, "split.tensors.count"),
            other => panic!("unexpected error: {other}"),
        }
    }
}
