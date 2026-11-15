//! The GGUF reader: a memory-mapped file wrapper and the parsed [`Gguf`] view.
//!
//! Parsing is split in two phases. First the header, metadata, and tensor infos
//! are decoded (tensor infos only need the alignment, which comes from
//! metadata). Then the aligned start of the tensor data section is computed and
//! every tensor's absolute range is validated against the file length before
//! the [`Gguf`] value is returned. Because all of that happens up front,
//! inspecting a parsed file cannot fail.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::cursor::Cursor;
use crate::error::{Error, Result, Warning};
use crate::header::{self, DEFAULT_ALIGNMENT, Header};
use crate::metadata::{self, MetadataEntry, MetadataValue};
use crate::split::{self, SplitAnalysis, SplitInfo};
use crate::tensor::{self, TensorBytes, TensorInfo};

/// Metadata key holding the model architecture name.
pub const GENERAL_ARCHITECTURE_KEY: &str = "general.architecture";

/// Metadata key holding a human-readable model name.
pub const GENERAL_NAME_KEY: &str = "general.name";

/// Metadata key holding the quantization format version.
pub const GENERAL_QUANTIZATION_VERSION_KEY: &str = "general.quantization_version";

/// Metadata key holding the tensor data alignment.
pub const GENERAL_ALIGNMENT_KEY: &str = "general.alignment";

/// Alignment must be a multiple of this value; anything else only warns.
const ALIGNMENT_STEP: u32 = 8;

/// A GGUF file memory-mapped read-only.
///
/// The mapping is never mutated, so the bytes returned by [`GgufFile::bytes`]
/// stay valid for the lifetime of the value. Do not modify or truncate the file
/// while it is mapped.
pub struct GgufFile {
    mmap: memmap2::Mmap,
    len: u64,
    path: PathBuf,
}

impl GgufFile {
    /// Open a GGUF file and memory-map it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the file cannot be opened, stat'ed, or mapped,
    /// and [`Error::EmptyFile`] when the file has zero length.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let file = std::fs::File::open(path)?;
        let metadata = file.metadata()?;
        if metadata.len() == 0 {
            return Err(Error::EmptyFile);
        }
        let mmap = map_read_only(&file)?;
        let len = mmap.len() as u64;
        if len == 0 {
            return Err(Error::EmptyFile);
        }
        Ok(Self {
            mmap,
            len,
            path: path.to_path_buf(),
        })
    }

    /// The mapped file contents.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.mmap
    }

    /// Length of the mapped file in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the mapped file is empty (always `false`; empty files are
    /// rejected by [`GgufFile::open`]).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Path the file was opened from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Debug for GgufFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GgufFile")
            .field("path", &self.path)
            .field("len", &self.len)
            .finish()
    }
}

/// Map a file read-only.
///
/// This is the single `unsafe` operation in the crate. It is sound as long as
/// the caller does not observe the mapping after the file is truncated or
/// mutated, which is documented on [`GgufFile`]; `memmap2` itself guarantees
/// the mapping is valid and read-only for as long as the `Mmap` value lives.
#[allow(unsafe_code)]
fn map_read_only(file: &std::fs::File) -> Result<memmap2::Mmap> {
    // SAFETY: `file` is opened read-only and stays open for the duration of the
    // call. `Mmap::map` maps the current file length read-only; the returned
    // `Mmap` keeps the mapping alive. The usual mmap caveat -- external
    // truncation causing SIGBUS -- is documented on the owning type.
    let mmap = unsafe { memmap2::Mmap::map(file)? };
    Ok(mmap)
}

/// A validated, zero-copy view of a GGUF file.
///
/// Construction is a single call to [`Gguf::parse`]. All structural
/// validation -- magic, version, metadata value types, alignment, tensor
/// quantization geometry, bounds, and duplicate names -- happens there, so the
/// accessors below are cheap and infallible. The only fallible accessor is
/// [`Gguf::split`], whose metadata validation is deliberately deferred so that
/// an unusual-but-readable file can still be inspected.
#[derive(Debug)]
pub struct Gguf<'a> {
    bytes: &'a [u8],
    header: Header,
    alignment: u32,
    data_start: u64,
    metadata: Vec<MetadataEntry<'a>>,
    tensors: Vec<TensorInfo<'a>>,
    tensor_bytes: TensorBytes,
    warnings: Vec<Warning>,
    split: SplitAnalysis,
}

impl<'a> Gguf<'a> {
    /// Parse and fully validate a GGUF file from a byte slice.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] for malformed input, including bad magic, an
    /// unsupported or big-endian version, truncated data, unknown metadata
    /// value types, invalid booleans or UTF-8, out-of-range counts, duplicate
    /// tensor names, misaligned quantization blocks, and tensor ranges that
    /// extend past the end of the file.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let header = header::parse(&mut cursor)?;
        let mut warnings = Vec::new();

        // Both parsers enforce their own caps (MAX_TENSORS and
        // MAX_METADATA_ENTRIES) before allocating anything.
        let metadata =
            metadata::parse_metadata(&mut cursor, header.metadata_kv_count, &mut warnings)?;
        let alignment = alignment_from_metadata(&metadata, &mut warnings)?;
        let mut tensors =
            tensor::parse_tensor_infos(&mut cursor, header.tensor_count, alignment, &mut warnings)?;

        let infos_end = cursor.position();
        let file_len = bytes.len() as u64;
        let data_start = align_up(infos_end, u64::from(alignment))?;
        if data_start > file_len {
            return Err(Error::DataStartBeyondFile {
                data_start,
                file_len,
            });
        }
        let padding = &bytes[infos_end as usize..data_start as usize];
        if padding.iter().any(|byte| *byte != 0) {
            warnings.push(Warning::new(
                Warning::NON_ZERO_PADDING,
                format!(
                    "alignment padding between tensor infos and tensor data is not zero ({} bytes)",
                    padding.len()
                ),
            ));
        }

        let mut unknown_types = 0_u64;
        let mut total_bytes = 0_u64;
        for info in &mut tensors {
            let file_offset =
                data_start
                    .checked_add(info.offset())
                    .ok_or(Error::ArithmeticOverflow {
                        context: "tensor file offset",
                    })?;
            info.set_file_offset(file_offset);
            match info.bytes() {
                Some(len) => {
                    let end = file_offset
                        .checked_add(len)
                        .ok_or(Error::ArithmeticOverflow {
                            context: "tensor data end offset",
                        })?;
                    if end > file_len {
                        return Err(Error::TensorOutOfBounds {
                            name: info.name().to_owned(),
                            start: file_offset,
                            end,
                            file_len,
                        });
                    }
                    total_bytes =
                        total_bytes
                            .checked_add(len)
                            .ok_or(Error::ArithmeticOverflow {
                                context: "total tensor bytes",
                            })?;
                }
                None => {
                    unknown_types =
                        unknown_types
                            .checked_add(1)
                            .ok_or(Error::ArithmeticOverflow {
                                context: "unknown tensor type count",
                            })?;
                }
            }
        }
        let tensor_bytes = TensorBytes {
            tensor_bytes: total_bytes,
            bytes_known: unknown_types == 0,
            unknown_types,
        };

        if let Some(warning) = overlap_warning(&tensors) {
            warnings.push(warning);
        }
        if !metadata
            .iter()
            .any(|entry| entry.key() == GENERAL_ARCHITECTURE_KEY)
        {
            warnings.push(Warning::new(
                Warning::MISSING_ARCHITECTURE,
                format!("no {GENERAL_ARCHITECTURE_KEY:?} metadata entry"),
            ));
        }
        let split = split::analyze(&metadata, &mut warnings);

        Ok(Self {
            bytes,
            header,
            alignment,
            data_start,
            metadata,
            tensors,
            tensor_bytes,
            warnings,
            split,
        })
    }

    /// The parsed file header.
    #[must_use]
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The tensor data alignment (`general.alignment`, default 32).
    #[must_use]
    pub fn alignment(&self) -> u32 {
        self.alignment
    }

    /// Absolute file offset where the tensor data section starts.
    #[must_use]
    pub fn data_start(&self) -> u64 {
        self.data_start
    }

    /// All metadata entries, in file order.
    #[must_use]
    pub fn metadata(&self) -> &[MetadataEntry<'a>] {
        &self.metadata
    }

    /// All tensor infos, in file order.
    ///
    /// The `inspect` report sorts tensors by offset; this accessor preserves
    /// the order the tensors appear in the file.
    #[must_use]
    pub fn tensors(&self) -> &[TensorInfo<'a>] {
        &self.tensors
    }

    /// Look up a tensor by name.
    #[must_use]
    pub fn tensor(&self, name: &str) -> Option<&TensorInfo<'a>> {
        self.tensors.iter().find(|info| info.name() == name)
    }

    /// Split metadata, if the file describes itself as part of a split model.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SplitMetadataType`] when a `split.*` key is present but
    /// has an unexpected value type. Files with only some of the keys produce a
    /// `split-metadata-incomplete` warning and return `Ok(None)`.
    pub fn split(&self) -> Result<Option<SplitInfo>> {
        self.split.result()
    }

    /// Non-fatal diagnostics collected while parsing, in emission order.
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Aggregate size of the tensor data.
    #[must_use]
    pub fn tensor_bytes(&self) -> TensorBytes {
        self.tensor_bytes
    }

    /// The full file contents this view borrows from.
    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Length of the file in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Whether the file is empty (always `false`; empty files fail to parse).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

fn alignment_from_metadata(
    metadata: &[MetadataEntry<'_>],
    warnings: &mut Vec<Warning>,
) -> Result<u32> {
    let Some(entry) = metadata
        .iter()
        .find(|entry| entry.key() == GENERAL_ALIGNMENT_KEY)
    else {
        return Ok(DEFAULT_ALIGNMENT);
    };
    let MetadataValue::U32(alignment) = entry.value() else {
        return Err(Error::MetadataTypeMismatch {
            key: GENERAL_ALIGNMENT_KEY.to_owned(),
            expected: "u32",
            actual: entry.value_type().name(),
        });
    };
    let alignment = *alignment;
    if alignment == 0 {
        return Err(Error::ZeroAlignment);
    }
    if alignment % ALIGNMENT_STEP != 0 {
        warnings.push(Warning::new(
            Warning::UNUSUAL_ALIGNMENT,
            format!(
                "{GENERAL_ALIGNMENT_KEY} is {alignment}, which is not a multiple of {ALIGNMENT_STEP}"
            ),
        ));
    }
    Ok(alignment)
}

/// Round `value` up to the next multiple of `alignment`.
fn align_up(value: u64, alignment: u64) -> Result<u64> {
    if alignment == 0 {
        return Err(Error::ZeroAlignment);
    }
    let remainder = value % alignment;
    if remainder == 0 {
        return Ok(value);
    }
    value
        .checked_add(alignment - remainder)
        .ok_or(Error::ArithmeticOverflow {
            context: "data section alignment",
        })
}

/// Detect overlapping tensor byte ranges in `O(n log n)`.
///
/// Tensors are sorted by offset and only adjacent pairs are compared: if any
/// two ranges overlap, some adjacent pair in offset order must overlap as well,
/// because starts are non-decreasing after sorting.
fn overlap_warning(tensors: &[TensorInfo<'_>]) -> Option<Warning> {
    let mut ranges: Vec<(u64, u64, &str)> = tensors
        .iter()
        .filter_map(|info| {
            info.bytes().map(|len| {
                (
                    info.offset(),
                    info.offset().saturating_add(len),
                    info.name(),
                )
            })
        })
        .collect();
    if ranges.len() < 2 {
        return None;
    }
    ranges.sort_unstable_by(|left, right| left.0.cmp(&right.0).then_with(|| left.2.cmp(right.2)));
    let mut overlapping_pairs = 0_u64;
    let mut first_pair: Option<(&str, &str)> = None;
    for pair in ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            overlapping_pairs = overlapping_pairs.saturating_add(1);
            if first_pair.is_none() {
                first_pair = Some((pair[0].2, pair[1].2));
            }
        }
    }
    if overlapping_pairs == 0 {
        return None;
    }
    let (first, second) = first_pair.unwrap_or(("", ""));
    Some(Warning::new(
        Warning::OVERLAPPING_TENSORS,
        format!(
            "{overlapping_pairs} adjacent tensor byte range(s) overlap (first: {first:?} and {second:?})"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{self, Builder, TensorSpec, Value};

    fn parse(bytes: &[u8]) -> Gguf<'_> {
        Gguf::parse(bytes).unwrap()
    }

    #[test]
    fn parses_the_minimal_fixture() {
        let bytes = fixture::minimal();
        assert_eq!(bytes.len(), 32);
        let gguf = parse(&bytes);
        assert_eq!(gguf.header().version, 3);
        assert_eq!(gguf.header().tensor_count, 0);
        assert_eq!(gguf.header().metadata_kv_count, 0);
        assert_eq!(gguf.alignment(), 32);
        assert_eq!(gguf.data_start(), 32);
        assert_eq!(gguf.len(), 32);
        assert!(!gguf.is_empty());
        assert!(gguf.metadata().is_empty());
        assert!(gguf.tensors().is_empty());
        assert!(gguf.tensor("anything").is_none());
        assert_eq!(gguf.tensor_bytes().tensor_bytes, 0);
        assert!(gguf.tensor_bytes().bytes_known);
        assert_eq!(gguf.tensor_bytes().unknown_types, 0);
        assert_eq!(gguf.split().unwrap(), None);
        // No architecture metadata.
        assert_eq!(
            gguf.warnings()
                .iter()
                .map(Warning::code)
                .collect::<Vec<_>>(),
            [Warning::MISSING_ARCHITECTURE]
        );
        assert_eq!(gguf.bytes(), bytes.as_slice());
    }

    #[test]
    fn parses_the_rich_fixture() {
        let bytes = fixture::rich();
        let gguf = parse(&bytes);
        assert_eq!(gguf.header().version, 3);
        assert_eq!(gguf.header().tensor_count, 3);
        assert_eq!(gguf.tensors().len(), 3);
        assert_eq!(gguf.alignment(), 64);
        assert_eq!(gguf.data_start() % 64, 0);
        assert_eq!(gguf.metadata().len(), 24);
        assert!(gguf.warnings().is_empty(), "{:?}", gguf.warnings());

        let tensors: Vec<&str> = gguf.tensors().iter().map(TensorInfo::name).collect();
        assert_eq!(
            tensors,
            [
                "token_embd.weight",
                "blk.0.attn_q.weight",
                "blk.0.ffn_down.weight"
            ]
        );
        let embedding = gguf.tensor("token_embd.weight").unwrap();
        assert_eq!(embedding.bytes(), Some(131_072));
        assert_eq!(
            embedding.file_offset(),
            gguf.data_start() + embedding.offset()
        );
        assert_eq!(gguf.tensor("nope"), None);

        assert_eq!(gguf.tensor_bytes().tensor_bytes, 131_072 + 136 + 288);
        assert!(gguf.tensor_bytes().bytes_known);
        assert_eq!(
            gguf.split().unwrap(),
            Some(SplitInfo {
                no: 1,
                count: 2,
                tensors_count: 48
            })
        );
    }

    #[test]
    fn version_two_files_parse() {
        let bytes = Builder::new().version(2).build();
        let gguf = parse(&bytes);
        assert_eq!(gguf.header().version, 2);
        assert_eq!(gguf.data_start(), 32);
    }

    #[test]
    fn unusual_alignment_warns_but_is_used() {
        let bytes = Builder::new()
            .metadata(GENERAL_ALIGNMENT_KEY, Value::U32(12))
            .tensor(TensorSpec::new("t", &[12], 0))
            .build();
        let gguf = parse(&bytes);
        assert_eq!(gguf.alignment(), 12);
        assert_eq!(gguf.data_start() % 12, 0);
        let codes: Vec<&str> = gguf.warnings().iter().map(Warning::code).collect();
        assert_eq!(
            codes,
            [Warning::UNUSUAL_ALIGNMENT, Warning::MISSING_ARCHITECTURE]
        );
    }

    #[test]
    fn zero_alignment_is_an_error() {
        let bytes = Builder::new()
            .metadata(GENERAL_ALIGNMENT_KEY, Value::U32(0))
            .build();
        assert_eq!(Gguf::parse(&bytes).unwrap_err(), Error::ZeroAlignment);
    }

    #[test]
    fn alignment_with_wrong_type_is_an_error() {
        let bytes = Builder::new()
            .metadata(GENERAL_ALIGNMENT_KEY, Value::String("64".into()))
            .build();
        match Gguf::parse(&bytes).unwrap_err() {
            Error::MetadataTypeMismatch {
                key,
                expected,
                actual,
            } => {
                assert_eq!(key, GENERAL_ALIGNMENT_KEY);
                assert_eq!(expected, "u32");
                assert_eq!(actual, "string");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn non_zero_padding_is_a_warning() {
        let mut bytes = fixture::minimal();
        // Bytes 24..32 are padding before the (empty) data section.
        bytes[24] = 1;
        let gguf = parse(&bytes);
        assert_eq!(
            gguf.warnings()
                .iter()
                .map(Warning::code)
                .collect::<Vec<_>>(),
            [Warning::NON_ZERO_PADDING, Warning::MISSING_ARCHITECTURE]
        );
    }

    #[test]
    fn data_start_past_the_file_is_an_error() {
        let bytes = fixture::minimal();
        // Truncating to the end of the fixed header leaves data_start (32) past
        // the file length (24).
        let error = Gguf::parse(&bytes[..24]).unwrap_err();
        match error {
            Error::DataStartBeyondFile {
                data_start,
                file_len,
            } => {
                assert_eq!(data_start, 32);
                assert_eq!(file_len, 24);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn tensor_data_past_the_file_is_an_error() {
        let bytes = Builder::new()
            .tensor(TensorSpec::new("big", &[64, 512], 0).data_len(16))
            .build();
        match Gguf::parse(&bytes).unwrap_err() {
            Error::TensorOutOfBounds {
                name,
                file_len,
                start,
                end,
            } => {
                assert_eq!(name, "big");
                assert_eq!(start, end - 131_072);
                assert_eq!(end, file_len + 131_072 - 16);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn overlapping_tensors_are_a_warning() {
        let bytes = Builder::new()
            .tensor(TensorSpec::new("a", &[4], 0).offset(0))
            .tensor(TensorSpec::new("b", &[4], 0).offset(0))
            .build();
        let gguf = parse(&bytes);
        assert_eq!(
            gguf.warnings()
                .iter()
                .map(Warning::code)
                .collect::<Vec<_>>(),
            [Warning::OVERLAPPING_TENSORS, Warning::MISSING_ARCHITECTURE]
        );
    }

    #[test]
    fn unaligned_tensor_offsets_are_a_warning() {
        let bytes = Builder::new()
            .tensor(TensorSpec::new("a", &[4], 0).offset(4))
            .build();
        let gguf = parse(&bytes);
        assert_eq!(
            gguf.warnings()
                .iter()
                .map(Warning::code)
                .collect::<Vec<_>>(),
            [
                Warning::UNALIGNED_TENSOR_OFFSET,
                Warning::MISSING_ARCHITECTURE
            ]
        );
    }

    #[test]
    fn trailing_bytes_are_allowed_silently() {
        let mut bytes = fixture::minimal();
        bytes.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let gguf = parse(&bytes);
        assert_eq!(gguf.len(), 36);
        assert_eq!(gguf.data_start(), 32);
        assert_eq!(
            gguf.warnings()
                .iter()
                .map(Warning::code)
                .collect::<Vec<_>>(),
            [Warning::MISSING_ARCHITECTURE]
        );
    }

    #[test]
    fn duplicate_tensor_names_are_errors() {
        let bytes = Builder::new()
            .tensor(TensorSpec::new("dup", &[4], 0))
            .tensor(TensorSpec::new("dup", &[4], 0))
            .build();
        assert_eq!(
            Gguf::parse(&bytes).unwrap_err(),
            Error::DuplicateTensorName("dup".into())
        );
    }

    #[test]
    fn unknown_quantization_types_are_reported_not_rejected() {
        let bytes = Builder::new()
            .tensor(TensorSpec::new("future", &[32], 4))
            .build();
        let gguf = parse(&bytes);
        let aggregate = gguf.tensor_bytes();
        assert!(!aggregate.bytes_known);
        assert_eq!(aggregate.unknown_types, 1);
        assert_eq!(aggregate.tensor_bytes, 0);
        let tensor = &gguf.tensors()[0];
        assert_eq!(tensor.bytes(), None);
        assert_eq!(tensor.type_name(), "unknown");
        let codes: Vec<&str> = gguf.warnings().iter().map(Warning::code).collect();
        assert_eq!(
            codes,
            [
                Warning::UNKNOWN_QUANTIZATION_TYPE,
                Warning::MISSING_ARCHITECTURE
            ]
        );
    }

    #[test]
    fn split_metadata_partial_warns_and_returns_none() {
        let bytes = Builder::new().metadata("split.no", Value::U16(1)).build();
        let gguf = parse(&bytes);
        assert_eq!(gguf.split().unwrap(), None);
        assert!(
            gguf.warnings()
                .iter()
                .any(|warning| warning.code() == Warning::SPLIT_METADATA_INCOMPLETE)
        );
    }

    #[test]
    fn split_metadata_wrong_type_errors_from_split_accessor() {
        let bytes = Builder::new()
            .metadata("split.no", Value::String("1".into()))
            .metadata("split.count", Value::U16(2))
            .metadata("split.tensors.count", Value::I32(48))
            .build();
        // Parsing itself succeeds.
        let gguf = parse(&bytes);
        let error = gguf.split().unwrap_err();
        assert_eq!(error.code(), "split-metadata-type");
        assert!(error.to_string().contains("split.no"));
    }

    #[test]
    fn metadata_lookup_preserves_file_order() {
        let bytes = fixture::rich();
        let gguf = parse(&bytes);
        let keys: Vec<&str> = gguf.metadata().iter().map(MetadataEntry::key).collect();
        assert_eq!(keys[0], "general.architecture");
        assert_eq!(keys[1], "general.name");
        assert_eq!(gguf.metadata()[0].value().as_str(), Some("llama"));
    }

    #[test]
    fn gguf_file_round_trips_through_the_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.gguf");
        std::fs::write(&path, fixture::minimal()).unwrap();
        let file = GgufFile::open(&path).unwrap();
        assert_eq!(file.len(), 32);
        assert!(!file.is_empty());
        assert_eq!(file.path(), path.as_path());
        assert_eq!(file.bytes(), fixture::minimal().as_slice());
        let gguf = Gguf::parse(file.bytes()).unwrap();
        assert_eq!(gguf.data_start(), 32);
        assert!(format!("{file:?}").contains("tiny.gguf"));
    }

    #[test]
    fn empty_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.gguf");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(GgufFile::open(&path).unwrap_err().code(), "empty-file");
    }

    #[test]
    fn missing_files_are_io_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.gguf");
        assert_eq!(GgufFile::open(&path).unwrap_err().code(), "io");
    }

    #[test]
    fn align_up_rounds_to_the_next_multiple() {
        assert_eq!(align_up(0, 32).unwrap(), 0);
        assert_eq!(align_up(1, 32).unwrap(), 32);
        assert_eq!(align_up(32, 32).unwrap(), 32);
        assert_eq!(align_up(33, 32).unwrap(), 64);
        assert_eq!(align_up(24, 64).unwrap(), 64);
        assert_eq!(align_up(1, 0).unwrap_err(), Error::ZeroAlignment);
        match align_up(u64::MAX, 32) {
            Err(Error::ArithmeticOverflow { .. }) => {}
            other => panic!("expected overflow, got {other:?}"),
        }
    }
}
