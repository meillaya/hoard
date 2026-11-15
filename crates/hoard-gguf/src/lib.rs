//! `hoard-gguf` -- strict, zero-copy GGUF parsing and inspection.
//!
//! GGUF is the model container format used by `llama.cpp`. This crate parses
//! version 2 and version 3 GGUF files (little-endian only) into a validated,
//! zero-copy [`Gguf`] view, and renders the versioned [`InspectReport`] JSON
//! document used by the `hoard-gguf` command line tool.
//!
//! # Design
//!
//! * **Strict.** Every length read from the file is checked against the bytes
//!   that remain, every count has an upper bound, and all size arithmetic is
//!   checked. Adversarial input produces an [`Error`], never a panic.
//! * **Zero-copy.** Metadata keys, string values, and tensor names are borrowed
//!   from the mapped file; parsing allocates only for the metadata entry list,
//!   array values, and tensor infos.
//! * **Validate once.** [`Gguf::parse`] rejects anything structurally broken
//!   (bad magic, unsupported/big-endian version, truncation, unknown metadata
//!   value types, duplicate tensor names, quantization block mismatches, tensor
//!   ranges past the end of the file). After that, accessors are infallible;
//!   only [`Gguf::split`] can still fail because split metadata validation is
//!   deliberately deferred.
//! * **Warnings never reject.** Unusual-but-readable files (unknown
//!   quantization ids, unaligned tensor offsets, overlapping tensors, non-zero
//!   padding, incomplete split metadata, ...) are accepted and reported through
//!   [`Gguf::warnings`] and the `warnings` array of the inspect report. Warning
//!   codes are stable kebab-case strings.
//!
//! # Example
//!
//! ```
//! use hoard_gguf::Gguf;
//!
//! // A minimal GGUF v3 file: header, zero metadata entries, zero tensors, and
//! // eight zero padding bytes up to the default 32-byte alignment.
//! let bytes = [
//!     b'G', b'G', b'U', b'F', // magic
//!     3, 0, 0, 0, // version
//!     0, 0, 0, 0, 0, 0, 0, 0, // tensor count
//!     0, 0, 0, 0, 0, 0, 0, 0, // metadata count
//!     0, 0, 0, 0, 0, 0, 0, 0, // padding to offset 32
//! ];
//! let gguf = Gguf::parse(&bytes)?;
//! assert_eq!(gguf.header().version, 3);
//! assert_eq!(gguf.alignment(), 32);
//! assert_eq!(gguf.data_start(), 32);
//! assert!(gguf.tensors().is_empty());
//!
//! let report = hoard_gguf::inspect(&gguf, None);
//! assert_eq!(report.schema, hoard_gguf::SCHEMA_VERSION);
//! assert_eq!(report.file.size, 32);
//! # Ok::<(), hoard_gguf::Error>(())
//! ```
//!
//! # Features
//!
//! * `cli` (default) -- builds the `hoard-gguf` binary. The library itself has
//!   no CLI dependencies.
//! * `testing` -- exposes the `fixture` builder used to regenerate the
//!   committed synthetic fixtures and to build malformed files in tests.
//!
//! # Limits
//!
//! | Limit | Value |
//! | --- | --- |
//! | [`MAX_TENSORS`] | 2^20 |
//! | [`MAX_METADATA_ENTRIES`] | 2^20 |
//! | [`MAX_ARRAY_ELEMENTS`] | 2^20 |
//! | [`MAX_ARRAY_DEPTH`] | 32 |
//! | [`MAX_TENSOR_DIMS`] | 4 |
//! | [`MAX_METADATA_KEY_LEN`] | 65535 bytes |
//! | [`MAX_TENSOR_NAME_LEN`] | 64 bytes |
//!
//! # Inspect JSON
//!
//! The inspect report keeps 64-bit counts and sizes as JSON integers. JSON
//! numbers are IEEE-754 doubles, so consumers that decode with a JavaScript
//! number (or any other double-backed JSON parser) lose precision above 2^53.
//! Rust's `serde_json` preserves the integers exactly; tools that care about
//! the top bits should use a 64-bit or big-integer JSON decoder.
//!
//! # Warning codes
//!
//! | Code | Meaning |
//! | --- | --- |
//! | `unusual-alignment` | `general.alignment` is not a multiple of 8 |
//! | `missing-architecture` | no `general.architecture` metadata entry |
//! | `split-metadata-incomplete` | only some `split.*` keys are present |
//! | `split-index-out-of-range` | `split.no` is not smaller than `split.count` |
//! | `metadata-key-not-ascii` | a metadata key contains non-ASCII bytes |
//! | `metadata-key-too-long` | a metadata key is longer than 65535 bytes |
//! | `duplicate-metadata-key` | two metadata entries share a key |
//! | `tensor-name-too-long` | a tensor name is longer than 64 bytes |
//! | `empty-tensor-name` | a tensor name is empty |
//! | `scalar-tensor` | a tensor has zero dimensions (one element) |
//! | `unknown-quantization-type` | a tensor uses an unknown `ggml_type` id |
//! | `unaligned-tensor-offset` | a tensor offset is not a multiple of the alignment |
//! | `overlapping-tensors` | two tensor byte ranges overlap |
//! | `non-zero-padding` | alignment padding before the data section is not zero |

#![deny(missing_docs)]

pub mod cursor;
pub mod error;
pub mod file;
#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
pub mod fixture;
pub mod header;
pub mod inspect;
pub mod metadata;
pub mod quant;
pub mod schema;
pub mod split;
pub mod tensor;

pub use error::{Error, Result, Warning};
pub use file::{Gguf, GgufFile};
pub use header::{ByteOrder, Header};
pub use inspect::{
    ArrayReport, InspectReport, MemoryReport, MetadataEntryReport, MetadataReport, ModelReport,
    QuantMixEntry, SCHEMA_VERSION, SplitReport, TensorReport, TensorsReport, WarningReport,
    inspect,
};
pub use metadata::{ArrayValue, MetadataEntry, MetadataValue, ValueType};
pub use quant::{QUANT_TYPES, QuantType};
pub use schema::inspect_schema;
pub use split::SplitInfo;
pub use tensor::{TensorBytes, TensorInfo};

/// Maximum number of tensors accepted in one file (2^20).
pub const MAX_TENSORS: u64 = 1 << 20;

/// Maximum number of metadata entries accepted in one file (2^20).
pub const MAX_METADATA_ENTRIES: u64 = 1 << 20;

/// Maximum number of elements accepted in one metadata array (2^20).
///
/// Real GGUF files store tokenizer and merge tables as metadata arrays of at
/// most a few hundred thousand elements; the cap bounds the memory a crafted
/// file can make the parser allocate.
pub const MAX_ARRAY_ELEMENTS: u64 = 1 << 20;

/// Maximum nesting depth accepted for metadata arrays (32).
///
/// Arrays may contain arrays; the parser decodes them iteratively, but the
/// resulting tree is recursively owned, so an unbounded depth would abort the
/// process while dropping it. Real files nest one or two levels.
pub const MAX_ARRAY_DEPTH: u32 = 32;

/// Maximum number of dimensions accepted for one tensor.
pub const MAX_TENSOR_DIMS: u32 = 4;

/// Maximum accepted metadata key length, in bytes.
pub const MAX_METADATA_KEY_LEN: usize = 65_535;

/// Maximum accepted tensor name length, in bytes.
pub const MAX_TENSOR_NAME_LEN: usize = 64;

/// Crate version, as recorded by Cargo.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_the_package_version() {
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
        assert!(!super::version().is_empty());
    }

    #[test]
    fn limits_have_the_contracted_values() {
        assert_eq!(super::MAX_TENSORS, 1 << 20);
        assert_eq!(super::MAX_METADATA_ENTRIES, 1 << 20);
        assert_eq!(super::MAX_ARRAY_ELEMENTS, 1 << 20);
        assert_eq!(super::MAX_ARRAY_DEPTH, 32);
        assert_eq!(super::MAX_TENSOR_DIMS, 4);
        assert_eq!(super::MAX_METADATA_KEY_LEN, 65_535);
        assert_eq!(super::MAX_TENSOR_NAME_LEN, 64);
    }
}
