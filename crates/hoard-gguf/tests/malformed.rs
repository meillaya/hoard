//! Malformed-input tests: mutations of the synthetic fixtures and hand-built
//! adversarial GGUF files.
//!
//! Every case here must produce a clean [`hoard_gguf::Error`] (or, for
//! unusual-but-readable files, a stable warning) and must never panic. The
//! truncation test runs over every byte boundary of the rich fixture.

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};

use common::RawGguf;
use hoard_gguf::{Error, Gguf, Warning, inspect};

fn warning_codes(gguf: &Gguf<'_>) -> Vec<&'static str> {
    gguf.warnings().iter().map(Warning::code).collect()
}

fn assert_has_warning(gguf: &Gguf<'_>, code: &str) {
    assert!(
        gguf.warnings().iter().any(|warning| warning.code() == code),
        "expected warning {code:?}, got {:?}",
        warning_codes(gguf)
    );
}

fn patch(bytes: &[u8], start: usize, replacement: &[u8]) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    patched[start..start + replacement.len()].copy_from_slice(replacement);
    patched
}

#[test]
fn truncation_at_every_byte_of_the_rich_fixture_is_a_clean_error() {
    let bytes = common::rich_gguf();
    for cut in 0..bytes.len() {
        let prefix = &bytes[..cut];
        let outcome = catch_unwind(AssertUnwindSafe(|| Gguf::parse(prefix)));
        let result = match outcome {
            Ok(result) => result,
            Err(_) => panic!("Gguf::parse panicked on a {cut}-byte prefix"),
        };
        assert!(
            result.is_err(),
            "a {cut}-byte prefix of the rich fixture parsed successfully"
        );
    }
}

#[test]
fn truncation_at_every_byte_of_the_minimal_fixture_is_a_clean_error() {
    let bytes = common::minimal_gguf();
    for cut in 0..bytes.len() {
        let outcome = catch_unwind(AssertUnwindSafe(|| Gguf::parse(&bytes[..cut])));
        let result = outcome.expect("parse panicked");
        assert!(result.is_err(), "a {cut}-byte prefix parsed successfully");
    }
}

#[test]
fn empty_and_short_inputs_are_truncated_errors() {
    for bytes in [
        &b""[..],
        &b"GGUF"[..],
        &b"GGU"[..],
        &b"GGUF\x03\x00\x00\x00"[..],
    ] {
        match Gguf::parse(bytes) {
            Err(error) => assert_eq!(error.code(), "truncated", "{bytes:?}"),
            Ok(_) => panic!("{bytes:?} parsed successfully"),
        }
    }
}

#[test]
fn bad_magic_is_rejected() {
    let bytes = common::rich_gguf();
    for magic in [b"GGML", b"gguf", b"\x00\x00\x00\x00"] {
        let patched = patch(&bytes, 0, magic);
        assert_eq!(Gguf::parse(&patched).unwrap_err().code(), "bad-magic");
    }
}

#[test]
fn unsupported_versions_are_rejected() {
    let bytes = common::rich_gguf();
    for version in [0_u32, 1, 4, 5, 100, 0x0001_0000] {
        let patched = patch(&bytes, 4, &version.to_le_bytes());
        match Gguf::parse(&patched) {
            Err(Error::UnsupportedVersion(found)) => assert_eq!(found, version),
            other => panic!("version {version}: unexpected result {other:?}"),
        }
    }
}

#[test]
fn version_two_files_with_v3_layout_parse() {
    let bytes = common::rich_gguf();
    let patched = patch(&bytes, 4, &2_u32.to_le_bytes());
    let gguf = Gguf::parse(&patched).unwrap();
    assert_eq!(gguf.header().version, 2);
}

#[test]
fn big_endian_version_hint_is_rejected() {
    let bytes = common::rich_gguf();
    for version in [2_u32, 3] {
        let patched = patch(&bytes, 4, &version.to_be_bytes());
        let error = Gguf::parse(&patched).unwrap_err();
        assert_eq!(error.code(), "big-endian");
        assert!(error.to_string().contains("big-endian"), "message: {error}");
    }
}

#[test]
fn unknown_metadata_value_type_is_rejected() {
    for type_id in [13_u32, 14, 99, u32::MAX] {
        let mut raw = RawGguf::new();
        raw.meta("k", type_id, &[]);
        let bytes = raw.build();
        match Gguf::parse(&bytes) {
            Err(Error::UnknownValueType(found)) => assert_eq!(found, type_id),
            other => panic!("type {type_id}: unexpected result {other:?}"),
        }
    }
}

#[test]
fn invalid_utf8_in_keys_and_string_values_is_rejected() {
    // Invalid UTF-8 key.
    let mut raw = RawGguf::new();
    raw.meta_raw_key(&[0xff, 0xfe], 4, &1_u32.to_le_bytes());
    assert_eq!(
        Gguf::parse(&raw.build()).unwrap_err().code(),
        "invalid-utf8"
    );

    // Invalid UTF-8 string value.
    let mut raw = RawGguf::new();
    let mut payload = 1_u64.to_le_bytes().to_vec();
    payload.push(0xff);
    raw.meta("k", 8, &payload);
    assert_eq!(
        Gguf::parse(&raw.build()).unwrap_err().code(),
        "invalid-utf8"
    );
}

#[test]
fn invalid_bool_payload_is_rejected() {
    for payload in [2_u8, 3, 0xff] {
        let mut raw = RawGguf::new();
        raw.meta("flag", 7, &[payload]);
        match Gguf::parse(&raw.build()) {
            Err(Error::InvalidBool { value, .. }) => assert_eq!(value, payload),
            other => panic!("payload {payload}: unexpected result {other:?}"),
        }
    }
}

#[test]
fn huge_header_counts_are_rejected() {
    let bytes = common::rich_gguf();
    let tensor_count = patch(&bytes, 8, &u64::MAX.to_le_bytes());
    assert_eq!(
        Gguf::parse(&tensor_count).unwrap_err().code(),
        "too-many-tensors"
    );
    let metadata_count = patch(&bytes, 16, &u64::MAX.to_le_bytes());
    assert_eq!(
        Gguf::parse(&metadata_count).unwrap_err().code(),
        "too-many-metadata-entries"
    );
}

#[test]
fn huge_string_length_is_truncation() {
    let mut raw = RawGguf::new();
    // Write the key length directly, without the bytes it announces.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(Gguf::parse(&bytes).unwrap_err().code(), "truncated");
    // The builder path must produce the same result for a huge value length.
    raw.meta("k", 8, &u64::MAX.to_le_bytes());
    assert_eq!(Gguf::parse(&raw.build()).unwrap_err().code(), "truncated");
}

#[test]
fn huge_array_lengths_are_rejected() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0_u32.to_le_bytes()); // element type: u8
    payload.extend_from_slice(&(hoard_gguf::MAX_ARRAY_ELEMENTS + 1).to_le_bytes());
    let mut raw = RawGguf::new();
    raw.meta("arr", 9, &payload);
    assert_eq!(
        Gguf::parse(&raw.build()).unwrap_err().code(),
        "too-many-array-elements"
    );
}

#[test]
fn more_than_four_dimensions_are_rejected() {
    let mut raw = RawGguf::new();
    raw.tensor("five", &[2, 2, 2, 2, 2], 0, 0);
    match Gguf::parse(&raw.build()) {
        Err(Error::TooManyDimensions { name, count, .. }) => {
            assert_eq!(name, "five");
            assert_eq!(count, 5);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn quantization_block_mismatch_is_rejected() {
    let mut raw = RawGguf::new();
    raw.tensor("bad", &[16, 2], 8, 0); // Q8_0 needs dim0 % 32 == 0
    let error = Gguf::parse(&raw.build()).unwrap_err();
    assert_eq!(error.code(), "quant-block-mismatch");
    assert!(error.to_string().contains("Q8_0"), "message: {error}");
}

#[test]
fn duplicate_tensor_names_are_rejected() {
    let mut raw = RawGguf::new();
    raw.tensor("dup", &[4], 0, 0);
    raw.tensor("dup", &[4], 0, 16);
    assert_eq!(
        Gguf::parse(&raw.build()).unwrap_err().code(),
        "duplicate-tensor-name"
    );
}

#[test]
fn tensor_data_past_the_end_of_the_file_is_rejected() {
    let mut raw = RawGguf::new();
    raw.tensor("big", &[4], 0, 0);
    raw.data_len(8); // declared F32 [4] needs 16 bytes
    let error = Gguf::parse(&raw.build()).unwrap_err();
    assert_eq!(error.code(), "tensor-out-of-bounds");
    assert!(error.to_string().contains("TensorOutOfBounds"));
}

#[test]
fn data_start_past_the_file_is_rejected() {
    // A file that stops exactly after the 24-byte header: the first aligned
    // data start (32) is past the end of the file.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    let error = Gguf::parse(&bytes).unwrap_err();
    assert_eq!(error.code(), "data-start-beyond-file");
    assert!(error.to_string().contains("32"));
}

#[test]
fn zero_alignment_is_rejected() {
    let mut raw = RawGguf::new();
    raw.meta(
        hoard_gguf::file::GENERAL_ALIGNMENT_KEY,
        4,
        &0_u32.to_le_bytes(),
    );
    assert_eq!(
        Gguf::parse(&raw.build()).unwrap_err().code(),
        "zero-alignment"
    );
}

#[test]
fn wrongly_typed_alignment_is_rejected() {
    let mut raw = RawGguf::new();
    raw.meta(
        hoard_gguf::file::GENERAL_ALIGNMENT_KEY,
        8,
        &common::string_payload("64"),
    );
    assert_eq!(
        Gguf::parse(&raw.build()).unwrap_err().code(),
        "metadata-type-mismatch"
    );
}

#[test]
fn unaligned_and_overlapping_offsets_warn_but_parse() {
    let mut raw = RawGguf::new();
    // Two F32 [4] tensors (16 bytes each) both at offset 4: unaligned and
    // overlapping.
    raw.tensor("a", &[4], 0, 4);
    raw.tensor("b", &[4], 0, 4);
    raw.data_len(20);
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_has_warning(&gguf, Warning::UNALIGNED_TENSOR_OFFSET);
    assert_has_warning(&gguf, Warning::OVERLAPPING_TENSORS);
    assert_has_warning(&gguf, Warning::MISSING_ARCHITECTURE);
    assert_eq!(gguf.tensors().len(), 2);
}

#[test]
fn non_zero_padding_warns() {
    let mut bytes = common::minimal_gguf();
    bytes[31] = 0xff;
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_has_warning(&gguf, Warning::NON_ZERO_PADDING);
}

#[test]
fn scalar_tensor_warns_and_counts_one_element() {
    let mut raw = RawGguf::new();
    raw.tensor("scale", &[], 0, 0);
    raw.data_len(4);
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_has_warning(&gguf, Warning::SCALAR_TENSOR);
    assert_eq!(gguf.tensors()[0].elements(), 1);
    assert_eq!(gguf.tensors()[0].bytes(), Some(4));
}

#[test]
fn unknown_quantization_type_warns_and_reports_null_bytes() {
    let mut raw = RawGguf::new();
    raw.tensor("future", &[32], 31, 0); // 31 is a removed ggml_type id
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_has_warning(&gguf, Warning::UNKNOWN_QUANTIZATION_TYPE);
    assert_eq!(gguf.tensors()[0].bytes(), None);
    assert!(!gguf.tensor_bytes().bytes_known);
    let report = inspect(&gguf, None);
    assert_eq!(report.tensors.unknown_types, 1);
    assert_eq!(report.tensors.entries[0].bytes, None);
    assert_eq!(report.tensors.entries[0].value_type, "unknown");
    assert!(report.quant_mix.is_empty());
}

#[test]
fn naming_warnings_cover_metadata_keys_and_tensor_names() {
    let long_key = "k".repeat(hoard_gguf::MAX_METADATA_KEY_LEN + 1);
    let mut raw = RawGguf::new();
    raw.meta("café", 4, &1_u32.to_le_bytes());
    raw.meta(&long_key, 4, &2_u32.to_le_bytes());
    raw.meta("dup", 4, &3_u32.to_le_bytes());
    raw.meta("dup", 4, &4_u32.to_le_bytes());
    raw.tensor("", &[4], 0, 0);
    raw.tensor(
        &"n".repeat(hoard_gguf::MAX_TENSOR_NAME_LEN + 1),
        &[4],
        0,
        16,
    );
    raw.data_len(32);
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_has_warning(&gguf, Warning::METADATA_KEY_NOT_ASCII);
    assert_has_warning(&gguf, Warning::METADATA_KEY_TOO_LONG);
    assert_has_warning(&gguf, Warning::DUPLICATE_METADATA_KEY);
    assert_has_warning(&gguf, Warning::EMPTY_TENSOR_NAME);
    assert_has_warning(&gguf, Warning::TENSOR_NAME_TOO_LONG);
}

#[test]
fn split_metadata_partial_warns() {
    let mut raw = RawGguf::new();
    raw.meta("split.no", 2, &0_u16.to_le_bytes());
    raw.meta("split.count", 2, &2_u16.to_le_bytes());
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_has_warning(&gguf, Warning::SPLIT_METADATA_INCOMPLETE);
    assert_eq!(gguf.split().unwrap(), None);
}

#[test]
fn split_metadata_wrong_type_errors_from_the_split_accessor() {
    let mut raw = RawGguf::new();
    raw.meta("split.no", 8, &common::string_payload("1"));
    raw.meta("split.count", 2, &2_u16.to_le_bytes());
    raw.meta("split.tensors.count", 5, &48_i32.to_le_bytes());
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    let error = gguf.split().unwrap_err();
    assert_eq!(error.code(), "split-metadata-type");
    // `inspect` stays infallible and reports a null split section.
    assert!(inspect(&gguf, None).split.is_none());
}

#[test]
fn split_index_out_of_range_warns() {
    let mut raw = RawGguf::new();
    raw.meta("split.no", 2, &2_u16.to_le_bytes());
    raw.meta("split.count", 2, &2_u16.to_le_bytes());
    raw.meta("split.tensors.count", 5, &96_i32.to_le_bytes());
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_has_warning(&gguf, Warning::SPLIT_INDEX_OUT_OF_RANGE);
    // The library accessor still returns the parsed values ...
    assert_eq!(gguf.split().unwrap().unwrap().no, 2);
    // ... but the report omits unusable split metadata (docs/inspect.md).
    assert!(inspect(&gguf, None).split.is_none());
}

#[test]
fn unusual_alignment_warns_but_is_used() {
    let mut raw = RawGguf::new();
    raw.meta(
        hoard_gguf::file::GENERAL_ALIGNMENT_KEY,
        4,
        &12_u32.to_le_bytes(),
    );
    raw.alignment(12);
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_has_warning(&gguf, Warning::UNUSUAL_ALIGNMENT);
    assert_eq!(gguf.alignment(), 12);
    assert_eq!(gguf.data_start() % 12, 0);
}

#[test]
fn trailing_bytes_after_the_last_tensor_are_allowed() {
    let mut raw = RawGguf::new();
    raw.tensor("t", &[4], 0, 0);
    raw.data_len(16);
    raw.trailing(&[0xde, 0xad, 0xbe, 0xef]);
    let bytes = raw.build();
    let gguf = Gguf::parse(&bytes).unwrap();
    assert_eq!(gguf.tensors().len(), 1);
    assert_eq!(
        warning_codes(&gguf),
        [Warning::MISSING_ARCHITECTURE],
        "trailing bytes must not produce warnings"
    );
}
