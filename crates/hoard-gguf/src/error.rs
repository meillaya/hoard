//! Errors and non-fatal diagnostics produced while parsing GGUF files.
//!
//! [`Error`] is fatal: a file that produces an error cannot be inspected.
//! [`Warning`] is advisory: warnings are collected while parsing, reported by
//! [`crate::inspect()`], and never change whether a file is accepted.

use std::fmt;

/// `Result` alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A fatal GGUF parsing or I/O error.
///
/// Every variant carries enough context to explain *where* parsing failed.
/// The [`Error::code`] method returns a stable kebab-case identifier that does
/// not change between releases, which is convenient for tests and logs.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file could not be opened, stat'ed, or memory-mapped.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The file has zero length, so it cannot contain a GGUF header.
    #[error("empty file: not a GGUF file")]
    EmptyFile,

    /// The first four bytes are not `GGUF`.
    #[error("not a GGUF file: bad magic (expected \"GGUF\")")]
    BadMagic,

    /// The file is a big-endian GGUF, which this crate deliberately rejects.
    ///
    /// GGUF is little-endian in practice; a swapped version word is the first
    /// sign that a file was written on a big-endian host, so parsing stops
    /// immediately with a precise message.
    #[error("big-endian GGUF is not supported (write little-endian)")]
    BigEndian,

    /// The version word is not 2 or 3.
    #[error("unsupported GGUF version {0} (supported versions: 2, 3)")]
    UnsupportedVersion(u32),

    /// A read ran past the end of the file (or past the end of the buffer).
    #[error("truncated GGUF: unexpected end of data at offset {offset}")]
    Truncated {
        /// Offset of the read that would have needed more bytes.
        offset: u64,
    },

    /// A length-prefixed string is not valid UTF-8.
    #[error("invalid UTF-8 in string at offset {offset}")]
    InvalidUtf8 {
        /// Offset of the first byte of the string payload.
        offset: u64,
    },

    /// A boolean metadata value has a payload other than `0` or `1`.
    #[error("invalid boolean payload {value} at offset {offset} (expected 0 or 1)")]
    InvalidBool {
        /// The offending payload byte.
        value: u8,
        /// Offset of the payload byte.
        offset: u64,
    },

    /// A metadata value type id is outside the known range `0..=12`.
    #[error("unknown metadata value type {0} (expected 0..=12)")]
    UnknownValueType(u32),

    /// The header declares more tensors than [`crate::MAX_TENSORS`].
    #[error("too many tensors: {count} declared (limit {limit})")]
    TooManyTensors {
        /// Declared tensor count.
        count: u64,
        /// Maximum accepted count.
        limit: u64,
    },

    /// The header declares more metadata entries than
    /// [`crate::MAX_METADATA_ENTRIES`].
    #[error("too many metadata entries: {count} declared (limit {limit})")]
    TooManyMetadataEntries {
        /// Declared metadata entry count.
        count: u64,
        /// Maximum accepted count.
        limit: u64,
    },

    /// An array (or nested array) declares more elements than
    /// [`crate::MAX_ARRAY_ELEMENTS`].
    #[error("metadata array too large: {count} elements (limit {limit})")]
    TooManyArrayElements {
        /// Declared element count.
        count: u64,
        /// Maximum accepted count.
        limit: u64,
    },

    /// Nested metadata arrays are deeper than [`crate::MAX_ARRAY_DEPTH`].
    #[error("metadata arrays nested more than {limit} levels deep")]
    ArrayNestingTooDeep {
        /// Maximum accepted nesting depth.
        limit: u32,
    },

    /// A tensor declares more than four dimensions.
    #[error("tensor {name:?}: {count} dimensions (limit {limit})")]
    TooManyDimensions {
        /// Tensor name.
        name: String,
        /// Declared dimension count.
        count: u32,
        /// Maximum accepted dimension count.
        limit: u32,
    },

    /// The product of a tensor's dimensions overflows `u64`.
    #[error("tensor {name:?}: element count overflows u64")]
    TensorElementOverflow {
        /// Tensor name.
        name: String,
    },

    /// A tensor's byte size computation overflows `u64`.
    #[error("tensor {name:?}: byte size overflows u64")]
    TensorByteOverflow {
        /// Tensor name.
        name: String,
    },

    /// The first tensor dimension is not a multiple of the quantization block
    /// size, so the tensor cannot be decoded (error code `quant-block-mismatch`).
    #[error(
        "tensor {name:?}: dimension 0 ({dim0}) is not a multiple of the {block_size}-element block size of {quant} (quant-block-mismatch)"
    )]
    QuantBlockMismatch {
        /// Tensor name.
        name: String,
        /// First (innermost) dimension.
        dim0: u64,
        /// Quantization block size.
        block_size: u32,
        /// Quantization type name.
        quant: &'static str,
    },

    /// A tensor's declared byte range extends past the end of the file.
    #[error(
        "tensor {name:?}: data range {start}..{end} exceeds file length {file_len} (TensorOutOfBounds)"
    )]
    TensorOutOfBounds {
        /// Tensor name.
        name: String,
        /// Absolute file offset of the first data byte.
        start: u64,
        /// Absolute file offset one past the last data byte.
        end: u64,
        /// Length of the file in bytes.
        file_len: u64,
    },

    /// Two tensor infos share the same name.
    #[error("duplicate tensor name {0:?}")]
    DuplicateTensorName(String),

    /// `general.alignment` is present but zero.
    #[error("general.alignment must not be zero")]
    ZeroAlignment,

    /// A metadata value used by the parser has an unexpected value type.
    #[error("metadata key {key:?}: expected {expected}, found {actual}")]
    MetadataTypeMismatch {
        /// Metadata key.
        key: String,
        /// Expected value type name.
        expected: &'static str,
        /// Actual value type name.
        actual: &'static str,
    },

    /// Metadata describing a split model uses an unexpected value type.
    #[error("split metadata key {key:?}: expected {expected}, found {actual}")]
    SplitMetadataType {
        /// Metadata key.
        key: &'static str,
        /// Expected value type name.
        expected: &'static str,
        /// Actual value type name.
        actual: &'static str,
    },

    /// A checked integer operation overflowed while parsing lengths or offsets.
    #[error("arithmetic overflow while computing {context}")]
    ArithmeticOverflow {
        /// What the parser was computing.
        context: &'static str,
    },

    /// The alignment padding pushes the data section past the end of the file.
    #[error("data section starts at {data_start} but the file is only {file_len} bytes")]
    DataStartBeyondFile {
        /// Computed aligned start of the tensor data section.
        data_start: u64,
        /// Length of the file in bytes.
        file_len: u64,
    },
}

impl Error {
    /// Stable kebab-case identifier for this error variant.
    ///
    /// These codes are part of the crate's public interface and only change
    /// with a major version bump. The one code mentioned explicitly by the GGUF
    /// tooling contract is `quant-block-mismatch`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::EmptyFile => "empty-file",
            Self::BadMagic => "bad-magic",
            Self::BigEndian => "big-endian",
            Self::UnsupportedVersion(_) => "unsupported-version",
            Self::Truncated { .. } => "truncated",
            Self::InvalidUtf8 { .. } => "invalid-utf8",
            Self::InvalidBool { .. } => "invalid-bool",
            Self::UnknownValueType(_) => "unknown-value-type",
            Self::TooManyTensors { .. } => "too-many-tensors",
            Self::TooManyMetadataEntries { .. } => "too-many-metadata-entries",
            Self::TooManyArrayElements { .. } => "too-many-array-elements",
            Self::ArrayNestingTooDeep { .. } => "metadata-array-depth",
            Self::TooManyDimensions { .. } => "too-many-dimensions",
            Self::TensorElementOverflow { .. } => "tensor-element-overflow",
            Self::TensorByteOverflow { .. } => "tensor-byte-overflow",
            Self::QuantBlockMismatch { .. } => "quant-block-mismatch",
            Self::TensorOutOfBounds { .. } => "tensor-out-of-bounds",
            Self::DuplicateTensorName(_) => "duplicate-tensor-name",
            Self::ZeroAlignment => "zero-alignment",
            Self::MetadataTypeMismatch { .. } => "metadata-type-mismatch",
            Self::SplitMetadataType { .. } => "split-metadata-type",
            Self::ArithmeticOverflow { .. } => "arithmetic-overflow",
            Self::DataStartBeyondFile { .. } => "data-start-beyond-file",
        }
    }
}

/// Compares errors structurally.
///
/// I/O errors compare equal when their [`std::io::ErrorKind`] matches, since
/// the underlying OS error is not comparable.
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            (Self::EmptyFile, Self::EmptyFile)
            | (Self::BadMagic, Self::BadMagic)
            | (Self::BigEndian, Self::BigEndian)
            | (Self::ZeroAlignment, Self::ZeroAlignment) => true,
            (Self::UnsupportedVersion(left), Self::UnsupportedVersion(right)) => left == right,
            (Self::Truncated { offset: left }, Self::Truncated { offset: right }) => left == right,
            (Self::InvalidUtf8 { offset: left }, Self::InvalidUtf8 { offset: right }) => {
                left == right
            }
            (
                Self::InvalidBool {
                    value: left_value,
                    offset: left_offset,
                },
                Self::InvalidBool {
                    value: right_value,
                    offset: right_offset,
                },
            ) => left_value == right_value && left_offset == right_offset,
            (Self::UnknownValueType(left), Self::UnknownValueType(right)) => left == right,
            (
                Self::TooManyTensors {
                    count: left_count,
                    limit: left_limit,
                },
                Self::TooManyTensors {
                    count: right_count,
                    limit: right_limit,
                },
            )
            | (
                Self::TooManyMetadataEntries {
                    count: left_count,
                    limit: left_limit,
                },
                Self::TooManyMetadataEntries {
                    count: right_count,
                    limit: right_limit,
                },
            )
            | (
                Self::TooManyArrayElements {
                    count: left_count,
                    limit: left_limit,
                },
                Self::TooManyArrayElements {
                    count: right_count,
                    limit: right_limit,
                },
            ) => left_count == right_count && left_limit == right_limit,
            (
                Self::ArrayNestingTooDeep { limit: left },
                Self::ArrayNestingTooDeep { limit: right },
            ) => left == right,
            (
                Self::TooManyDimensions {
                    name: left_name,
                    count: left_count,
                    limit: left_limit,
                },
                Self::TooManyDimensions {
                    name: right_name,
                    count: right_count,
                    limit: right_limit,
                },
            ) => left_name == right_name && left_count == right_count && left_limit == right_limit,
            (
                Self::TensorElementOverflow { name: left },
                Self::TensorElementOverflow { name: right },
            )
            | (Self::TensorByteOverflow { name: left }, Self::TensorByteOverflow { name: right }) => {
                left == right
            }
            (
                Self::QuantBlockMismatch {
                    name: left_name,
                    dim0: left_dim0,
                    block_size: left_block,
                    quant: left_quant,
                },
                Self::QuantBlockMismatch {
                    name: right_name,
                    dim0: right_dim0,
                    block_size: right_block,
                    quant: right_quant,
                },
            ) => {
                left_name == right_name
                    && left_dim0 == right_dim0
                    && left_block == right_block
                    && left_quant == right_quant
            }
            (
                Self::TensorOutOfBounds {
                    name: left_name,
                    start: left_start,
                    end: left_end,
                    file_len: left_len,
                },
                Self::TensorOutOfBounds {
                    name: right_name,
                    start: right_start,
                    end: right_end,
                    file_len: right_len,
                },
            ) => {
                left_name == right_name
                    && left_start == right_start
                    && left_end == right_end
                    && left_len == right_len
            }
            (Self::DuplicateTensorName(left), Self::DuplicateTensorName(right)) => left == right,
            (
                Self::MetadataTypeMismatch {
                    key: left_key,
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::MetadataTypeMismatch {
                    key: right_key,
                    expected: right_expected,
                    actual: right_actual,
                },
            ) => {
                left_key == right_key
                    && left_expected == right_expected
                    && left_actual == right_actual
            }
            (
                Self::SplitMetadataType {
                    key: left_key,
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::SplitMetadataType {
                    key: right_key,
                    expected: right_expected,
                    actual: right_actual,
                },
            ) => {
                left_key == right_key
                    && left_expected == right_expected
                    && left_actual == right_actual
            }
            (
                Self::ArithmeticOverflow { context: left },
                Self::ArithmeticOverflow { context: right },
            ) => left == right,
            (
                Self::DataStartBeyondFile {
                    data_start: left_data_start,
                    file_len: left_len,
                },
                Self::DataStartBeyondFile {
                    data_start: right_data_start,
                    file_len: right_len,
                },
            ) => left_data_start == right_data_start && left_len == right_len,
            _ => false,
        }
    }
}

impl Eq for Error {}

/// A non-fatal diagnostic emitted while parsing a GGUF file.
///
/// Warnings describe unusual-but-tolerable files: unaligned tensors, non-zero
/// padding, unknown quantization ids, incomplete split metadata, and so on.
/// They never change whether a file is accepted and are reported in the
/// `inspect` JSON document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    code: &'static str,
    message: String,
}

impl Warning {
    /// Alignment is not a multiple of 8 (`unusual-alignment`).
    pub const UNUSUAL_ALIGNMENT: &'static str = "unusual-alignment";
    /// No `general.architecture` metadata entry (`missing-architecture`).
    pub const MISSING_ARCHITECTURE: &'static str = "missing-architecture";
    /// Only some of the `split.*` metadata keys are present
    /// (`split-metadata-incomplete`).
    pub const SPLIT_METADATA_INCOMPLETE: &'static str = "split-metadata-incomplete";
    /// `split.no` is not smaller than `split.count` (`split-index-out-of-range`).
    pub const SPLIT_INDEX_OUT_OF_RANGE: &'static str = "split-index-out-of-range";
    /// A metadata key contains non-ASCII bytes (`metadata-key-not-ascii`).
    pub const METADATA_KEY_NOT_ASCII: &'static str = "metadata-key-not-ascii";
    /// A metadata key is longer than 65535 bytes (`metadata-key-too-long`).
    pub const METADATA_KEY_TOO_LONG: &'static str = "metadata-key-too-long";
    /// Two metadata entries share a key (`duplicate-metadata-key`).
    pub const DUPLICATE_METADATA_KEY: &'static str = "duplicate-metadata-key";
    /// A tensor name is longer than 64 bytes (`tensor-name-too-long`).
    pub const TENSOR_NAME_TOO_LONG: &'static str = "tensor-name-too-long";
    /// A tensor name is empty (`empty-tensor-name`).
    pub const EMPTY_TENSOR_NAME: &'static str = "empty-tensor-name";
    /// A tensor has zero dimensions and therefore one element (`scalar-tensor`).
    pub const SCALAR_TENSOR: &'static str = "scalar-tensor";
    /// A tensor uses a quantization type id that this crate does not know
    /// (`unknown-quantization-type`).
    pub const UNKNOWN_QUANTIZATION_TYPE: &'static str = "unknown-quantization-type";
    /// A tensor's offset is not a multiple of the file alignment
    /// (`unaligned-tensor-offset`).
    pub const UNALIGNED_TENSOR_OFFSET: &'static str = "unaligned-tensor-offset";
    /// Tensor byte ranges overlap (`overlapping-tensors`).
    pub const OVERLAPPING_TENSORS: &'static str = "overlapping-tensors";
    /// Padding between the tensor infos and the data section is not all zero
    /// (`non-zero-padding`).
    pub const NON_ZERO_PADDING: &'static str = "non-zero-padding";

    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Stable kebab-case warning code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Human-readable warning message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable_kebab_case() {
        let errors = [
            Error::EmptyFile,
            Error::BadMagic,
            Error::BigEndian,
            Error::UnsupportedVersion(1),
            Error::Truncated { offset: 3 },
            Error::InvalidUtf8 { offset: 0 },
            Error::InvalidBool {
                value: 2,
                offset: 7,
            },
            Error::UnknownValueType(99),
            Error::TooManyTensors { count: 2, limit: 1 },
            Error::TooManyMetadataEntries { count: 2, limit: 1 },
            Error::TooManyArrayElements { count: 2, limit: 1 },
            Error::ArrayNestingTooDeep { limit: 32 },
            Error::TooManyDimensions {
                name: "t".into(),
                count: 5,
                limit: 4,
            },
            Error::TensorElementOverflow { name: "t".into() },
            Error::TensorByteOverflow { name: "t".into() },
            Error::QuantBlockMismatch {
                name: "t".into(),
                dim0: 3,
                block_size: 32,
                quant: "Q8_0",
            },
            Error::TensorOutOfBounds {
                name: "t".into(),
                start: 0,
                end: 9,
                file_len: 4,
            },
            Error::DuplicateTensorName("t".into()),
            Error::ZeroAlignment,
            Error::MetadataTypeMismatch {
                key: "k".into(),
                expected: "u32",
                actual: "string",
            },
            Error::SplitMetadataType {
                key: "split.no",
                expected: "u16",
                actual: "string",
            },
            Error::ArithmeticOverflow { context: "x" },
            Error::DataStartBeyondFile {
                data_start: 100,
                file_len: 10,
            },
        ];
        for error in errors {
            let code = error.code();
            assert!(!code.is_empty(), "{error}");
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "code {code:?} is not kebab-case"
            );
            assert!(!error.to_string().is_empty());
        }
        assert_eq!(
            Error::QuantBlockMismatch {
                name: "t".into(),
                dim0: 3,
                block_size: 32,
                quant: "Q8_0",
            }
            .code(),
            "quant-block-mismatch"
        );
        // The exact message required by the format contract.
        assert!(
            Error::BigEndian
                .to_string()
                .contains("big-endian GGUF is not supported (write little-endian)")
        );
    }

    #[test]
    fn warning_codes_are_stable_kebab_case() {
        let warnings = [
            Warning::UNUSUAL_ALIGNMENT,
            Warning::MISSING_ARCHITECTURE,
            Warning::SPLIT_METADATA_INCOMPLETE,
            Warning::SPLIT_INDEX_OUT_OF_RANGE,
            Warning::METADATA_KEY_NOT_ASCII,
            Warning::METADATA_KEY_TOO_LONG,
            Warning::DUPLICATE_METADATA_KEY,
            Warning::TENSOR_NAME_TOO_LONG,
            Warning::EMPTY_TENSOR_NAME,
            Warning::SCALAR_TENSOR,
            Warning::UNKNOWN_QUANTIZATION_TYPE,
            Warning::UNALIGNED_TENSOR_OFFSET,
            Warning::OVERLAPPING_TENSORS,
            Warning::NON_ZERO_PADDING,
        ];
        for code in warnings {
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "code {code:?} is not kebab-case"
            );
        }
        assert_eq!(warnings.len(), 14);
        assert_eq!(Warning::UNUSUAL_ALIGNMENT, "unusual-alignment");
        assert_eq!(
            Warning::UNKNOWN_QUANTIZATION_TYPE,
            "unknown-quantization-type"
        );
        assert_eq!(Warning::SCALAR_TENSOR, "scalar-tensor");
        assert_eq!(
            Warning::SPLIT_INDEX_OUT_OF_RANGE,
            "split-index-out-of-range"
        );
        let warning = Warning::new(Warning::SCALAR_TENSOR, "a tensor is a scalar");
        assert_eq!(warning.code(), "scalar-tensor");
        assert_eq!(warning.message(), "a tensor is a scalar");
        assert_eq!(warning.to_string(), "scalar-tensor: a tensor is a scalar");
    }
}
