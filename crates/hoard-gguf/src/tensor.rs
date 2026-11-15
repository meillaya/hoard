//! Tensor infos: the descriptors that precede the tensor data section.
//!
//! Each tensor info is laid out as:
//!
//! ```text
//! string name
//! u32    n_dims       (0..=4)
//! u64[n] dims         (innermost dimension first)
//! u32    ggml_type    (quantization type id)
//! u64    offset       (relative to the start of the data section)
//! ```
//!
//! The parser validates dimensions, quantization geometry, and duplicate
//! names. Absolute file offsets are filled in later, once the aligned start of
//! the data section is known.

use std::collections::HashSet;

use crate::cursor::Cursor;
use crate::error::{Error, Result, Warning};
use crate::quant::{self, QuantType};
use crate::{MAX_TENSOR_DIMS, MAX_TENSOR_NAME_LEN, MAX_TENSORS};

/// A parsed tensor descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TensorInfo<'a> {
    name: &'a str,
    shape: Vec<u64>,
    type_id: u32,
    quant: Option<QuantType>,
    elements: u64,
    bytes: Option<u64>,
    offset: u64,
    file_offset: u64,
}

impl<'a> TensorInfo<'a> {
    /// The tensor name, borrowed from the file.
    #[must_use]
    pub const fn name(&self) -> &'a str {
        self.name
    }

    /// Tensor dimensions, innermost first, exactly as stored in the file.
    #[must_use]
    pub fn shape(&self) -> &[u64] {
        &self.shape
    }

    /// Number of dimensions (0 for a scalar tensor).
    #[must_use]
    pub fn n_dims(&self) -> u32 {
        self.shape.len() as u32
    }

    /// The numeric `ggml_type` id stored in the file.
    #[must_use]
    pub const fn type_id(&self) -> u32 {
        self.type_id
    }

    /// The quantization type, or `None` when the id is not known to this crate.
    #[must_use]
    pub const fn quant(&self) -> Option<QuantType> {
        self.quant
    }

    /// Display name of the quantization type (`"F32"`, `"Q8_0"`, ...), or
    /// `"unknown"` when the type id is not known.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self.quant {
            Some(quant) => quant.name,
            None => "unknown",
        }
    }

    /// Total number of elements (the product of the dimensions; 1 for scalars).
    #[must_use]
    pub const fn elements(&self) -> u64 {
        self.elements
    }

    /// Size of the tensor data in bytes, or `None` when the quantization type
    /// is unknown and the size therefore cannot be computed.
    #[must_use]
    pub const fn bytes(&self) -> Option<u64> {
        self.bytes
    }

    /// Offset of the tensor data, relative to the start of the data section.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Absolute offset of the tensor data within the file.
    ///
    /// This is `data_start + offset`; it is filled in once the aligned start of
    /// the data section has been computed.
    #[must_use]
    pub const fn file_offset(&self) -> u64 {
        self.file_offset
    }

    pub(crate) fn set_file_offset(&mut self, file_offset: u64) {
        self.file_offset = file_offset;
    }
}

/// Aggregate tensor byte accounting, returned by [`crate::Gguf::tensor_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TensorBytes {
    /// Sum of the data sizes of all tensors whose quantization type is known.
    pub tensor_bytes: u64,
    /// Whether the size of every tensor is known.
    pub bytes_known: bool,
    /// Number of tensors with an unknown quantization type.
    pub unknown_types: u64,
}

impl TensorBytes {
    /// Sum of the known tensor data sizes, in bytes.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.tensor_bytes
    }

    /// Whether the size of every tensor is known.
    #[must_use]
    pub const fn is_known(&self) -> bool {
        self.bytes_known
    }
}

fn capacity_hint(count: u64) -> usize {
    usize::try_from(count.min(1024)).unwrap_or(1024)
}

/// Parse `count` tensor infos from the cursor.
///
/// `alignment` is only used for the `unaligned-tensor-offset` warning; offsets
/// are validated against the data section start by the caller.
pub(crate) fn parse_tensor_infos<'a>(
    cursor: &mut Cursor<'a>,
    count: u64,
    alignment: u32,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<TensorInfo<'a>>> {
    if count > MAX_TENSORS {
        return Err(Error::TooManyTensors {
            count,
            limit: MAX_TENSORS,
        });
    }
    // Minimum encoded size of one tensor info: empty name (8), no dimensions
    // (4), type id (4), offset (8).
    const MIN_TENSOR_INFO_LEN: u64 = 8 + 4 + 4 + 8;
    let min_bytes = count
        .checked_mul(MIN_TENSOR_INFO_LEN)
        .ok_or(Error::ArithmeticOverflow {
            context: "tensor info count",
        })?;
    if min_bytes > cursor.remaining() {
        return Err(Error::Truncated {
            offset: cursor.position(),
        });
    }

    let mut tensors = Vec::with_capacity(capacity_hint(count));
    let mut seen_names: HashSet<&'a str> = HashSet::new();
    for _ in 0..count {
        let name = cursor.read_string()?;
        if name.len() > MAX_TENSOR_NAME_LEN {
            warnings.push(Warning::new(
                Warning::TENSOR_NAME_TOO_LONG,
                format!(
                    "tensor name {name:?} is {} bytes long (limit {MAX_TENSOR_NAME_LEN})",
                    name.len()
                ),
            ));
        }
        if name.is_empty() {
            warnings.push(Warning::new(
                Warning::EMPTY_TENSOR_NAME,
                "tensor name is empty",
            ));
        }
        if !seen_names.insert(name) {
            return Err(Error::DuplicateTensorName(name.to_owned()));
        }

        let n_dims = cursor.read_u32()?;
        if n_dims > MAX_TENSOR_DIMS {
            return Err(Error::TooManyDimensions {
                name: name.to_owned(),
                count: n_dims,
                limit: MAX_TENSOR_DIMS,
            });
        }
        let mut shape = Vec::with_capacity(n_dims as usize);
        for _ in 0..n_dims {
            shape.push(cursor.read_u64()?);
        }
        let type_id = cursor.read_u32()?;
        let offset = cursor.read_u64()?;

        if shape.is_empty() {
            warnings.push(Warning::new(
                Warning::SCALAR_TENSOR,
                format!("tensor {name:?} has zero dimensions and one element"),
            ));
        }
        let elements = shape
            .iter()
            .try_fold(1_u64, |product, dim| product.checked_mul(*dim))
            .ok_or_else(|| Error::TensorElementOverflow {
                name: name.to_owned(),
            })?;

        let quant = quant::from_id(type_id);
        let bytes = match quant {
            Some(quant_type) => {
                // The innermost dimension is what quantization blocks lay out
                // along; a scalar tensor has a nominal innermost size of 1.
                let dim0 = shape.first().copied().unwrap_or(1);
                if dim0 % u64::from(quant_type.block_size) != 0 {
                    return Err(Error::QuantBlockMismatch {
                        name: name.to_owned(),
                        dim0,
                        block_size: quant_type.block_size,
                        quant: quant_type.name,
                    });
                }
                Some(quant_type.encoded_bytes(elements).ok_or_else(|| {
                    Error::TensorByteOverflow {
                        name: name.to_owned(),
                    }
                })?)
            }
            None => {
                warnings.push(Warning::new(
                    Warning::UNKNOWN_QUANTIZATION_TYPE,
                    format!("tensor {name:?} uses unknown quantization type id {type_id}"),
                ));
                None
            }
        };

        if u64::from(alignment) == 0 || offset % u64::from(alignment) != 0 {
            warnings.push(Warning::new(
                Warning::UNALIGNED_TENSOR_OFFSET,
                format!(
                    "tensor {name:?} offset {offset} is not a multiple of alignment {alignment}"
                ),
            ));
        }

        tensors.push(TensorInfo {
            name,
            shape,
            type_id,
            quant,
            elements,
            bytes,
            offset,
            file_offset: 0,
        });
    }
    Ok(tensors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    fn encode_tensor(out: &mut Vec<u8>, name: &str, dims: &[u64], type_id: u32, offset: u64) {
        encode_string(out, name);
        out.extend_from_slice(&(dims.len() as u32).to_le_bytes());
        for dim in dims {
            out.extend_from_slice(&dim.to_le_bytes());
        }
        out.extend_from_slice(&type_id.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
    }

    fn parse(
        bytes: &[u8],
        count: u64,
        alignment: u32,
    ) -> Result<(Vec<TensorInfo<'_>>, Vec<Warning>)> {
        let mut cursor = Cursor::new(bytes);
        let mut warnings = Vec::new();
        let tensors = parse_tensor_infos(&mut cursor, count, alignment, &mut warnings)?;
        Ok((tensors, warnings))
    }

    #[test]
    fn parses_tensor_infos_and_computes_sizes() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "token_embd.weight", &[64, 512], 0, 0);
        encode_tensor(&mut bytes, "blk.0.attn_q.weight", &[32, 4], 8, 131_072);
        encode_tensor(&mut bytes, "blk.0.ffn_down.weight", &[256, 2], 12, 131_264);

        let (tensors, warnings) = parse(&bytes, 3, 32).unwrap();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(tensors.len(), 3);

        let f32_tensor = &tensors[0];
        assert_eq!(f32_tensor.name(), "token_embd.weight");
        assert_eq!(f32_tensor.shape(), [64, 512]);
        assert_eq!(f32_tensor.n_dims(), 2);
        assert_eq!(f32_tensor.type_id(), 0);
        assert_eq!(f32_tensor.quant().unwrap().name, "F32");
        assert_eq!(f32_tensor.type_name(), "F32");
        assert_eq!(f32_tensor.elements(), 32_768);
        assert_eq!(f32_tensor.bytes(), Some(131_072));
        assert_eq!(f32_tensor.offset(), 0);
        assert_eq!(f32_tensor.file_offset(), 0);

        assert_eq!(tensors[1].bytes(), Some(136)); // 4 blocks of 34 bytes
        assert_eq!(tensors[2].bytes(), Some(288)); // 2 blocks of 144 bytes
    }

    #[test]
    fn scalar_tensor_has_one_element_and_a_warning() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "scale", &[], 0, 0);
        let (tensors, warnings) = parse(&bytes, 1, 32).unwrap();
        assert!(tensors[0].shape().is_empty());
        assert_eq!(tensors[0].n_dims(), 0);
        assert_eq!(tensors[0].elements(), 1);
        assert_eq!(tensors[0].bytes(), Some(4));
        assert_eq!(
            warnings.iter().map(Warning::code).collect::<Vec<_>>(),
            [Warning::SCALAR_TENSOR]
        );
    }

    #[test]
    fn name_warnings_are_emitted() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "", &[4], 0, 0);
        encode_tensor(&mut bytes, &"n".repeat(65), &[4], 0, 0);
        let (_tensors, warnings) = parse(&bytes, 2, 32).unwrap();
        assert_eq!(
            warnings.iter().map(Warning::code).collect::<Vec<_>>(),
            [Warning::EMPTY_TENSOR_NAME, Warning::TENSOR_NAME_TOO_LONG]
        );
    }

    #[test]
    fn unaligned_offsets_are_warnings() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "a", &[32], 0, 4);
        let (_, warnings) = parse(&bytes, 1, 32).unwrap();
        assert_eq!(
            warnings.iter().map(Warning::code).collect::<Vec<_>>(),
            [Warning::UNALIGNED_TENSOR_OFFSET]
        );
    }

    #[test]
    fn duplicate_names_are_errors() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "same", &[32], 0, 0);
        encode_tensor(&mut bytes, "same", &[32], 0, 128);
        match parse(&bytes, 2, 32).unwrap_err() {
            Error::DuplicateTensorName(name) => assert_eq!(name, "same"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn five_dimensions_are_an_error() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "a", &[2, 2, 2, 2, 2], 0, 0);
        match parse(&bytes, 1, 32).unwrap_err() {
            Error::TooManyDimensions { name, count, limit } => {
                assert_eq!(name, "a");
                assert_eq!(count, 5);
                assert_eq!(limit, 4);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn four_dimensions_are_fine() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "a", &[2, 2, 2, 2], 0, 0);
        let (tensors, _) = parse(&bytes, 1, 32).unwrap();
        assert_eq!(tensors[0].elements(), 16);
    }

    #[test]
    fn quantization_block_mismatch_is_an_error() {
        // Q8_0 blocks are 32 elements wide, but dim 0 is 16.
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "bad", &[16, 2], 8, 0);
        match parse(&bytes, 1, 32).unwrap_err() {
            Error::QuantBlockMismatch {
                name,
                dim0,
                block_size,
                quant,
            } => {
                assert_eq!(name, "bad");
                assert_eq!(dim0, 16);
                assert_eq!(block_size, 32);
                assert_eq!(quant, "Q8_0");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn zero_length_first_dimension_is_allowed() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "empty", &[32, 0], 8, 0);
        let (tensors, warnings) = parse(&bytes, 1, 32).unwrap();
        assert_eq!(tensors[0].elements(), 0);
        assert_eq!(tensors[0].bytes(), Some(0));
        assert!(warnings.is_empty());
    }

    #[test]
    fn unknown_quantization_type_is_a_warning_without_size() {
        for type_id in [4_u32, 5, 31, 99] {
            let mut bytes = Vec::new();
            encode_tensor(&mut bytes, "future", &[32], type_id, 0);
            let (tensors, warnings) = parse(&bytes, 1, 32).unwrap();
            assert_eq!(tensors[0].quant(), None);
            assert_eq!(tensors[0].type_name(), "unknown");
            assert_eq!(tensors[0].type_id(), type_id);
            assert_eq!(tensors[0].bytes(), None);
            assert_eq!(
                warnings.iter().map(Warning::code).collect::<Vec<_>>(),
                [Warning::UNKNOWN_QUANTIZATION_TYPE]
            );
        }
    }

    #[test]
    fn element_count_overflow_is_an_error() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "huge", &[u64::MAX, 2], 0, 0);
        match parse(&bytes, 1, 32).unwrap_err() {
            Error::TensorElementOverflow { name } => assert_eq!(name, "huge"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn huge_tensor_counts_are_rejected_before_allocating() {
        let mut cursor = Cursor::new(&[]);
        let mut warnings = Vec::new();
        match parse_tensor_infos(&mut cursor, u64::MAX, 32, &mut warnings).unwrap_err() {
            Error::TooManyTensors { count, limit } => {
                assert_eq!(count, u64::MAX);
                assert_eq!(limit, MAX_TENSORS);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn truncation_anywhere_is_a_clean_error() {
        let mut bytes = Vec::new();
        encode_tensor(&mut bytes, "alpha", &[32, 4], 8, 128);
        encode_tensor(&mut bytes, "beta", &[256], 12, 1024);
        for cut in 0..bytes.len() {
            assert!(
                parse(&bytes[..cut], 2, 32).is_err(),
                "cut={cut} unexpectedly parsed"
            );
        }
    }

    #[test]
    fn aggregate_tensor_bytes_helpers() {
        let aggregate = TensorBytes {
            tensor_bytes: 512,
            bytes_known: false,
            unknown_types: 2,
        };
        assert_eq!(aggregate.total(), 512);
        assert!(!aggregate.is_known());
    }
}
