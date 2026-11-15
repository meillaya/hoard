//! The GGML quantization type table.
//!
//! Tensor infos in a GGUF file refer to quantization types by numeric id. This
//! module mirrors the `ggml_type` enum from `llama.cpp` (commit
//! `de32b268c4ad55d45f3b7b16c029f4e077f4ba74`, `gguf-py/gguf/constants.py`)
//! so that tensor byte sizes can be computed without looking at the tensor
//! data.
//!
//! Ids that were removed from `llama.cpp` (4, 5, 31, 32, 33, 36, 37, 38) are
//! intentionally absent: seeing one is reported as the
//! [`crate::Warning::UNKNOWN_QUANTIZATION_TYPE`] warning, not an error.

use std::fmt;

/// A GGML quantization type: numeric id, display name, and block geometry.
///
/// `block_size` elements are encoded in `type_size` bytes, so a tensor with
/// `elements` elements occupies `elements / block_size * type_size` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantType {
    /// Numeric `ggml_type` id as stored in the tensor info.
    pub id: u32,
    /// Upper-case display name, such as `F32`, `Q8_0`, or `Q4_K`.
    pub name: &'static str,
    /// Number of elements per quantized block.
    pub block_size: u32,
    /// Number of bytes per quantized block.
    pub type_size: u32,
}

impl QuantType {
    /// Number of bytes used by a tensor with `elements` elements, or `None`
    /// when the element count is not a multiple of the block size.
    #[must_use]
    pub const fn encoded_bytes(&self, elements: u64) -> Option<u64> {
        let block = self.block_size as u64;
        if !elements.is_multiple_of(block) {
            return None;
        }
        match (elements / block).checked_mul(self.type_size as u64) {
            Some(bytes) => Some(bytes),
            None => None,
        }
    }
}

impl fmt::Display for QuantType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// The known quantization types, sorted by ascending [`QuantType::id`].
pub const QUANT_TYPES: &[QuantType] = &[
    QuantType {
        id: 0,
        name: "F32",
        block_size: 1,
        type_size: 4,
    },
    QuantType {
        id: 1,
        name: "F16",
        block_size: 1,
        type_size: 2,
    },
    QuantType {
        id: 2,
        name: "Q4_0",
        block_size: 32,
        type_size: 18,
    },
    QuantType {
        id: 3,
        name: "Q4_1",
        block_size: 32,
        type_size: 20,
    },
    QuantType {
        id: 6,
        name: "Q5_0",
        block_size: 32,
        type_size: 22,
    },
    QuantType {
        id: 7,
        name: "Q5_1",
        block_size: 32,
        type_size: 24,
    },
    QuantType {
        id: 8,
        name: "Q8_0",
        block_size: 32,
        type_size: 34,
    },
    QuantType {
        id: 9,
        name: "Q8_1",
        block_size: 32,
        type_size: 36,
    },
    QuantType {
        id: 10,
        name: "Q2_K",
        block_size: 256,
        type_size: 84,
    },
    QuantType {
        id: 11,
        name: "Q3_K",
        block_size: 256,
        type_size: 110,
    },
    QuantType {
        id: 12,
        name: "Q4_K",
        block_size: 256,
        type_size: 144,
    },
    QuantType {
        id: 13,
        name: "Q5_K",
        block_size: 256,
        type_size: 176,
    },
    QuantType {
        id: 14,
        name: "Q6_K",
        block_size: 256,
        type_size: 210,
    },
    QuantType {
        id: 15,
        name: "Q8_K",
        block_size: 256,
        type_size: 292,
    },
    QuantType {
        id: 16,
        name: "IQ2_XXS",
        block_size: 256,
        type_size: 66,
    },
    QuantType {
        id: 17,
        name: "IQ2_XS",
        block_size: 256,
        type_size: 74,
    },
    QuantType {
        id: 18,
        name: "IQ3_XXS",
        block_size: 256,
        type_size: 98,
    },
    QuantType {
        id: 19,
        name: "IQ1_S",
        block_size: 256,
        type_size: 50,
    },
    QuantType {
        id: 20,
        name: "IQ4_NL",
        block_size: 32,
        type_size: 18,
    },
    QuantType {
        id: 21,
        name: "IQ3_S",
        block_size: 256,
        type_size: 110,
    },
    QuantType {
        id: 22,
        name: "IQ2_S",
        block_size: 256,
        type_size: 82,
    },
    QuantType {
        id: 23,
        name: "IQ4_XS",
        block_size: 256,
        type_size: 136,
    },
    QuantType {
        id: 24,
        name: "I8",
        block_size: 1,
        type_size: 1,
    },
    QuantType {
        id: 25,
        name: "I16",
        block_size: 1,
        type_size: 2,
    },
    QuantType {
        id: 26,
        name: "I32",
        block_size: 1,
        type_size: 4,
    },
    QuantType {
        id: 27,
        name: "I64",
        block_size: 1,
        type_size: 8,
    },
    QuantType {
        id: 28,
        name: "F64",
        block_size: 1,
        type_size: 8,
    },
    QuantType {
        id: 29,
        name: "IQ1_M",
        block_size: 256,
        type_size: 56,
    },
    QuantType {
        id: 30,
        name: "BF16",
        block_size: 1,
        type_size: 2,
    },
    QuantType {
        id: 34,
        name: "TQ1_0",
        block_size: 256,
        type_size: 54,
    },
    QuantType {
        id: 35,
        name: "TQ2_0",
        block_size: 256,
        type_size: 66,
    },
    QuantType {
        id: 39,
        name: "MXFP4",
        block_size: 32,
        type_size: 17,
    },
    QuantType {
        id: 40,
        name: "NVFP4",
        block_size: 64,
        type_size: 36,
    },
    QuantType {
        id: 41,
        name: "Q1_0",
        block_size: 128,
        type_size: 18,
    },
    QuantType {
        id: 42,
        name: "Q2_0",
        block_size: 64,
        type_size: 18,
    },
    QuantType {
        id: 43,
        name: "F8_E4M3",
        block_size: 1,
        type_size: 1,
    },
];

/// Look up a quantization type by its numeric `ggml_type` id.
///
/// Returns `None` for removed ids (4, 5, 31, 32, 33, 36, 37, 38) and for any
/// id added to `llama.cpp` after this crate's table was updated.
#[must_use]
pub fn from_id(id: u32) -> Option<QuantType> {
    match QUANT_TYPES.binary_search_by_key(&id, |quant| quant.id) {
        Ok(index) => Some(QUANT_TYPES[index]),
        Err(_) => None,
    }
}

pub(crate) fn name_of(id: u32) -> &'static str {
    match from_id(id) {
        Some(quant) => quant.name,
        None => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected `(id, name, block_size, type_size)` tuples straight from the
    /// `llama.cpp` constants file this crate mirrors.
    const EXPECTED: &[(u32, &str, u32, u32)] = &[
        (0, "F32", 1, 4),
        (1, "F16", 1, 2),
        (2, "Q4_0", 32, 18),
        (3, "Q4_1", 32, 20),
        (6, "Q5_0", 32, 22),
        (7, "Q5_1", 32, 24),
        (8, "Q8_0", 32, 34),
        (9, "Q8_1", 32, 36),
        (10, "Q2_K", 256, 84),
        (11, "Q3_K", 256, 110),
        (12, "Q4_K", 256, 144),
        (13, "Q5_K", 256, 176),
        (14, "Q6_K", 256, 210),
        (15, "Q8_K", 256, 292),
        (16, "IQ2_XXS", 256, 66),
        (17, "IQ2_XS", 256, 74),
        (18, "IQ3_XXS", 256, 98),
        (19, "IQ1_S", 256, 50),
        (20, "IQ4_NL", 32, 18),
        (21, "IQ3_S", 256, 110),
        (22, "IQ2_S", 256, 82),
        (23, "IQ4_XS", 256, 136),
        (24, "I8", 1, 1),
        (25, "I16", 1, 2),
        (26, "I32", 1, 4),
        (27, "I64", 1, 8),
        (28, "F64", 1, 8),
        (29, "IQ1_M", 256, 56),
        (30, "BF16", 1, 2),
        (34, "TQ1_0", 256, 54),
        (35, "TQ2_0", 256, 66),
        (39, "MXFP4", 32, 17),
        (40, "NVFP4", 64, 36),
        (41, "Q1_0", 128, 18),
        (42, "Q2_0", 64, 18),
        (43, "F8_E4M3", 1, 1),
    ];

    const REMOVED: &[u32] = &[4, 5, 31, 32, 33, 36, 37, 38];

    #[test]
    fn table_matches_llama_cpp_exactly() {
        assert_eq!(QUANT_TYPES.len(), EXPECTED.len());
        assert_eq!(QUANT_TYPES.len(), 36);
        for (quant, expected) in QUANT_TYPES.iter().zip(EXPECTED) {
            assert_eq!(
                (quant.id, quant.name, quant.block_size, quant.type_size),
                *expected,
                "mismatch for quant type {}",
                expected.1
            );
        }
    }

    #[test]
    fn ids_are_ascending_and_unique() {
        for pair in QUANT_TYPES.windows(2) {
            assert!(
                pair[0].id < pair[1].id,
                "ids not ascending: {} then {}",
                pair[0].id,
                pair[1].id
            );
        }
        let names: std::collections::HashSet<_> = QUANT_TYPES.iter().map(|q| q.name).collect();
        assert_eq!(names.len(), QUANT_TYPES.len());
    }

    #[test]
    fn lookups_handle_known_removed_and_unknown_ids() {
        assert_eq!(from_id(0).unwrap().name, "F32");
        assert_eq!(from_id(43).unwrap().name, "F8_E4M3");
        for &id in REMOVED {
            assert!(from_id(id).is_none(), "removed id {id} must be unknown");
        }
        for id in [44, 45, 99, u32::MAX] {
            assert!(from_id(id).is_none(), "id {id} must be unknown");
        }
        assert_eq!(name_of(0), "F32");
        assert_eq!(name_of(4), "unknown");
        assert_eq!(name_of(u32::MAX), "unknown");
    }

    #[test]
    fn encoded_bytes_uses_block_geometry() {
        let f32_type = from_id(0).unwrap();
        assert_eq!(f32_type.encoded_bytes(64 * 512), Some(131_072));
        assert_eq!(f32_type.encoded_bytes(0), Some(0));

        let q8_0 = from_id(8).unwrap();
        assert_eq!(q8_0.encoded_bytes(128), Some(136));
        assert_eq!(q8_0.encoded_bytes(32), Some(34));
        assert_eq!(q8_0.encoded_bytes(31), None);

        let q4_k = from_id(12).unwrap();
        assert_eq!(q4_k.encoded_bytes(512), Some(288));

        // Overflowing multiplication yields None rather than panicking.
        assert_eq!(q8_0.encoded_bytes(u64::MAX - (u64::MAX % 32)), None);
    }

    #[test]
    fn display_prints_the_type_name() {
        assert_eq!(from_id(8).unwrap().to_string(), "Q8_0");
    }
}
