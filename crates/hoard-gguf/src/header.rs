//! The GGUF header: magic, version, and item counts.
//!
//! Layout of the 24-byte fixed header (all integers little-endian):
//!
//! ```text
//! offset 0  magic      4 bytes  b"GGUF"
//! offset 4  version    4 bytes  2 or 3
//! offset 8  tensors    8 bytes  tensor count
//! offset 16 metadata   8 bytes  metadata key/value count
//! offset 24 metadata entries, then tensor infos
//! ```

use serde::{Deserialize, Serialize};

use crate::cursor::Cursor;
use crate::error::{Error, Result};

/// The four magic bytes that start every GGUF file.
pub const MAGIC: [u8; 4] = *b"GGUF";

/// Alignment used when `general.alignment` is absent from the metadata.
pub const DEFAULT_ALIGNMENT: u32 = 32;

/// Minimum GGUF version accepted by this parser.
pub const MIN_SUPPORTED_VERSION: u32 = 2;

/// Maximum GGUF version accepted by this parser.
pub const MAX_SUPPORTED_VERSION: u32 = 3;

/// Byte order of a GGUF file.
///
/// Only little-endian files are supported, so the enum currently has a single
/// variant; it exists so the inspect JSON can state the byte order explicitly
/// and so a future big-endian reader can extend it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ByteOrder {
    /// Little-endian, the only byte order `hoard-gguf` accepts.
    Little,
}

impl ByteOrder {
    /// Lower-case name used in the inspect JSON (`"little"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Little => "little",
        }
    }
}

impl std::fmt::Display for ByteOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The parsed GGUF header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// GGUF format version, 2 or 3.
    pub version: u32,
    /// Number of tensor infos in the file.
    pub tensor_count: u64,
    /// Number of metadata key/value pairs in the file.
    pub metadata_kv_count: u64,
    /// Byte order of the file (always [`ByteOrder::Little`]).
    pub byte_order: ByteOrder,
}

/// Parse the fixed header and validate the magic, byte order, and version.
///
/// Big-endian files are rejected before the version check so that the error
/// message can say what is wrong instead of reporting an absurd version number.
pub(crate) fn parse(cursor: &mut Cursor<'_>) -> Result<Header> {
    let magic = cursor.take(MAGIC.len() as u64)?;
    if magic != MAGIC {
        return Err(Error::BadMagic);
    }
    let version = cursor.read_u32()?;
    if version & 0xFFFF == 0 {
        let swapped = version.swap_bytes();
        if swapped == 2 || swapped == 3 {
            return Err(Error::BigEndian);
        }
    }
    if !(MIN_SUPPORTED_VERSION..=MAX_SUPPORTED_VERSION).contains(&version) {
        return Err(Error::UnsupportedVersion(version));
    }
    let tensor_count = cursor.read_u64()?;
    let metadata_kv_count = cursor.read_u64()?;
    Ok(Header {
        version,
        tensor_count,
        metadata_kv_count,
        byte_order: ByteOrder::Little,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_bytes(version: u32, tensors: u64, metadata: u64) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&tensors.to_le_bytes());
        bytes.extend_from_slice(&metadata.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_minimal_v3_header() {
        let bytes = header_bytes(3, 7, 11);
        let mut cursor = Cursor::new(&bytes);
        let header = parse(&mut cursor).unwrap();
        assert_eq!(header.version, 3);
        assert_eq!(header.tensor_count, 7);
        assert_eq!(header.metadata_kv_count, 11);
        assert_eq!(header.byte_order, ByteOrder::Little);
        assert_eq!(cursor.position(), 24);
        assert_eq!(header.byte_order.as_str(), "little");
        assert_eq!(header.byte_order.to_string(), "little");
    }

    #[test]
    fn accepts_versions_two_and_three() {
        for version in [2_u32, 3] {
            let bytes = header_bytes(version, 0, 0);
            let header = parse(&mut Cursor::new(&bytes)).unwrap();
            assert_eq!(header.version, version);
        }
    }

    #[test]
    fn rejects_v1_and_unknown_versions() {
        for version in [0_u32, 1, 4, 5, 100, 0x0001_0000] {
            let bytes = header_bytes(version, 0, 0);
            match parse(&mut Cursor::new(&bytes)).unwrap_err() {
                Error::UnsupportedVersion(found) => assert_eq!(found, version),
                other => panic!("unexpected error for version {version}: {other}"),
            }
        }
    }

    #[test]
    fn detects_big_endian_version_hint() {
        // A big-endian writer stores version 3 as 00 00 00 03; read as
        // little-endian that is 0x03000000.
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&3_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        let error = parse(&mut Cursor::new(&bytes)).unwrap_err();
        assert_eq!(error.code(), "big-endian");
        assert!(error.to_string().contains("big-endian"));
        assert_eq!(error, Error::BigEndian);
    }

    #[test]
    fn big_endian_v2_is_also_detected() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&2_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u64.to_be_bytes());
        bytes.extend_from_slice(&0_u64.to_be_bytes());
        assert_eq!(
            parse(&mut Cursor::new(&bytes)).unwrap_err(),
            Error::BigEndian
        );
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = b"GGML".to_vec();
        bytes.extend_from_slice(&[0_u8; 20]);
        assert_eq!(
            parse(&mut Cursor::new(&bytes)).unwrap_err(),
            Error::BadMagic
        );
    }

    #[test]
    fn truncated_header_is_a_clean_error() {
        let full = header_bytes(3, 1, 1);
        for cut in 0..full.len() {
            let error = parse(&mut Cursor::new(&full[..cut])).unwrap_err();
            assert_eq!(error.code(), "truncated", "cut={cut}");
        }
    }

    #[test]
    fn magic_only_buffer_is_truncated() {
        assert_eq!(
            parse(&mut Cursor::new(MAGIC.as_slice()))
                .unwrap_err()
                .code(),
            "truncated"
        );
        assert_eq!(
            parse(&mut Cursor::new(&[])).unwrap_err().code(),
            "truncated"
        );
    }
}
