//! A little-endian, bounds-checked read cursor over a byte slice.
//!
//! Every length read from the input file is untrusted, so all reads go through
//! [`Cursor::take`], which fails with [`Error::Truncated`] instead of panicking
//! when the requested range extends past the end of the buffer.

use crate::error::{Error, Result};

/// A bounds-checked cursor over a borrowed byte slice.
///
/// The cursor keeps a byte offset; integer reads are little-endian by
/// construction because GGUF files are little-endian.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Create a cursor positioned at the start of `bytes`.
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// The underlying byte slice.
    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Current position, in bytes from the start of the buffer.
    #[must_use]
    pub fn position(&self) -> u64 {
        self.pos as u64
    }

    /// Number of bytes left before the end of the buffer.
    #[must_use]
    pub fn remaining(&self) -> u64 {
        (self.bytes.len() - self.pos) as u64
    }

    /// Whether the cursor has reached the end of the buffer.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pos == self.bytes.len()
    }

    /// Read exactly `len` bytes and advance the cursor.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than `len` bytes remain, with the
    /// offset at which the read started.
    pub fn take(&mut self, len: u64) -> Result<&'a [u8]> {
        if len > self.remaining() {
            return Err(Error::Truncated {
                offset: self.position(),
            });
        }
        // `len <= remaining` guarantees the cast and the addition are in
        // bounds: `pos + len <= bytes.len()`.
        let len = len as usize;
        let start = self.pos;
        self.pos += len;
        Ok(&self.bytes[start..self.pos])
    }

    /// Read one byte.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] at the end of the buffer.
    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Read a signed byte.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] at the end of the buffer.
    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    /// Read a little-endian `u16`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than two bytes remain.
    pub fn read_u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take_array()?))
    }

    /// Read a little-endian `i16`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than two bytes remain.
    pub fn read_i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.take_array()?))
    }

    /// Read a little-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than four bytes remain.
    pub fn read_u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take_array()?))
    }

    /// Read a little-endian `i32`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than four bytes remain.
    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take_array()?))
    }

    /// Read a little-endian `f32`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than four bytes remain.
    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take_array()?))
    }

    /// Read a little-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than eight bytes remain.
    pub fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take_array()?))
    }

    /// Read a little-endian `i64`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than eight bytes remain.
    pub fn read_i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take_array()?))
    }

    /// Read a little-endian `f64`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if fewer than eight bytes remain.
    pub fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take_array()?))
    }

    /// Read a boolean encoded as one byte that must be exactly `0` or `1`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] at the end of the buffer, or
    /// [`Error::InvalidBool`] for any payload other than 0 or 1.
    pub fn read_bool(&mut self) -> Result<bool> {
        let offset = self.position();
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(Error::InvalidBool { value, offset }),
        }
    }

    /// Read a GGUF string: a `u64` byte length followed by UTF-8 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Truncated`] if the length prefix or the payload runs
    /// past the end of the buffer, and [`Error::InvalidUtf8`] if the payload is
    /// not valid UTF-8.
    pub fn read_string(&mut self) -> Result<&'a str> {
        let len = self.read_u64()?;
        let offset = self.position();
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8 { offset })
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.take(N as u64)?;
        let mut array = [0_u8; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_integers_and_floats() {
        let bytes = [
            0x01, // u8
            0xff, // i8
            0x34, 0x12, // u16
            0xfe, 0xff, // i16
            0x78, 0x56, 0x34, 0x12, // u32
            0xfb, 0xff, 0xff, 0xff, // i32 = -5
            0x00, 0x00, 0x80, 0x3f, // f32 = 1.0
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // u64
            0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // i64 = -2
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // f64 = 0.0
        ];
        let mut cursor = Cursor::new(&bytes);
        assert_eq!(cursor.read_u8().unwrap(), 0x01);
        assert_eq!(cursor.read_i8().unwrap(), -1);
        assert_eq!(cursor.read_u16().unwrap(), 0x1234);
        assert_eq!(cursor.read_i16().unwrap(), -2);
        assert_eq!(cursor.read_u32().unwrap(), 0x1234_5678);
        assert_eq!(cursor.read_i32().unwrap(), -5);
        assert_eq!(cursor.read_f32().unwrap(), 1.0);
        assert_eq!(cursor.read_u64().unwrap(), 0x0102_0304_0506_0708);
        assert_eq!(cursor.read_i64().unwrap(), -2);
        assert_eq!(cursor.read_f64().unwrap(), 0.0);
        assert!(cursor.is_empty());
        assert_eq!(cursor.position(), bytes.len() as u64);
    }

    #[test]
    fn take_advances_and_returns_slices() {
        let bytes = *b"hello, world";
        let mut cursor = Cursor::new(&bytes);
        assert_eq!(cursor.take(5).unwrap(), b"hello");
        assert_eq!(cursor.position(), 5);
        assert_eq!(cursor.remaining(), 7);
        assert_eq!(cursor.take(0).unwrap(), b"");
        assert_eq!(cursor.remaining(), 7);
        assert_eq!(cursor.take(7).unwrap(), b", world");
        assert!(cursor.is_empty());
    }

    #[test]
    fn every_truncation_point_is_a_clean_error() {
        let bytes = [1_u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        for cut in 0..bytes.len() {
            let truncated = &bytes[..cut];
            let mut cursor = Cursor::new(truncated);
            // Consuming everything in one-byte steps must never panic; it must
            // fail exactly at the end.
            for (offset, expected) in bytes.iter().enumerate().take(cut) {
                assert_eq!(cursor.position(), offset as u64);
                assert_eq!(cursor.read_u8().unwrap(), *expected);
            }
            assert_eq!(
                cursor.read_u8().unwrap_err().code(),
                "truncated",
                "cut={cut}"
            );
        }
    }

    #[test]
    fn take_overflow_is_truncation_not_panic() {
        let bytes = [0_u8; 4];
        let mut cursor = Cursor::new(&bytes);
        let error = cursor.take(u64::MAX).unwrap_err();
        match error {
            Error::Truncated { offset } => assert_eq!(offset, 0),
            other => panic!("unexpected error: {other}"),
        }
        assert_eq!(cursor.position(), 0, "failed reads must not advance");
        let error = cursor.take(5).unwrap_err();
        assert!(matches!(error, Error::Truncated { offset: 0 }));
    }

    #[test]
    fn truncated_length_prefix_and_payload_have_precise_offsets() {
        // u64 length = 4, then only two payload bytes.
        let mut bytes = 4_u64.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"ab");
        let mut cursor = Cursor::new(&bytes);
        let error = cursor.read_string().unwrap_err();
        match error {
            Error::Truncated { offset } => assert_eq!(offset, 8),
            other => panic!("unexpected error: {other}"),
        }

        // A truncated length prefix reports the offset of the prefix.
        let mut cursor = Cursor::new(&[0_u8]);
        match cursor.read_u64().unwrap_err() {
            Error::Truncated { offset } => assert_eq!(offset, 0),
            other => panic!("unexpected error: {other}"),
        }
        match cursor.read_u32().unwrap_err() {
            Error::Truncated { offset } => assert_eq!(offset, 0),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn strings_are_borrowed_from_the_input() {
        let mut bytes = 5_u64.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"hello");
        let mut cursor = Cursor::new(&bytes);
        let value = cursor.read_string().unwrap();
        assert_eq!(value, "hello");
        // Zero-copy: the &str points into the input buffer.
        let start = value.as_ptr() as usize;
        let buffer_start = bytes.as_ptr() as usize;
        assert_eq!(start, buffer_start + 8);
        assert!(cursor.is_empty());
    }

    #[test]
    fn invalid_utf8_string_is_an_error() {
        let mut bytes = 2_u64.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe]);
        let mut cursor = Cursor::new(&bytes);
        match cursor.read_string().unwrap_err() {
            Error::InvalidUtf8 { offset } => assert_eq!(offset, 8),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn bool_payload_must_be_zero_or_one() {
        for (payload, expected) in [(0_u8, false), (1, true)] {
            let bytes = [payload];
            let mut cursor = Cursor::new(&bytes);
            assert_eq!(cursor.read_bool().unwrap(), expected);
        }
        for invalid in [2_u8, 3, 0xff] {
            let bytes = [invalid];
            let mut cursor = Cursor::new(&bytes);
            match cursor.read_bool().unwrap_err() {
                Error::InvalidBool { value, offset } => {
                    assert_eq!(value, invalid);
                    assert_eq!(offset, 0);
                }
                other => panic!("unexpected error: {other}"),
            }
        }
    }

    #[test]
    fn empty_cursor_reports_truncation_and_zero_remaining() {
        let mut cursor = Cursor::new(&[]);
        assert!(cursor.is_empty());
        assert_eq!(cursor.remaining(), 0);
        assert_eq!(cursor.position(), 0);
        assert!(matches!(
            cursor.read_u8(),
            Err(Error::Truncated { offset: 0 })
        ));
    }
}
