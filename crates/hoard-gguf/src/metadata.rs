//! GGUF metadata: value types, entries, and the metadata parser.
//!
//! Metadata is a list of key/value pairs stored before the tensor infos.
//! Keys and strings are borrowed from the input buffer (zero-copy). Arrays are
//! materialized as [`MetadataValue`] lists; nested arrays are supported and are
//! decoded with an explicit stack so that deeply nested input cannot overflow
//! the thread stack.
//!
//! Value type ids (the `u32` written after each key):
//!
//! | id | type | id | type | id | type |
//! |----|------|----|------|----|------|
//! | 0  | u8   | 5  | i32  | 10 | u64  |
//! | 1  | i8   | 6  | f32  | 11 | i64  |
//! | 2  | u16  | 7  | bool | 12 | f64  |
//! | 3  | i16  | 8  | string |   |     |
//! | 4  | u32  | 9  | array |    |     |

use std::collections::HashSet;
use std::fmt;

use crate::cursor::Cursor;
use crate::error::{Error, Result, Warning};
use crate::{MAX_ARRAY_DEPTH, MAX_ARRAY_ELEMENTS, MAX_METADATA_ENTRIES, MAX_METADATA_KEY_LEN};

/// The type of a metadata value, matching the GGUF `gguf_meta_value_type` ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ValueType {
    /// Unsigned 8-bit integer (id 0).
    U8 = 0,
    /// Signed 8-bit integer (id 1).
    I8 = 1,
    /// Unsigned 16-bit integer (id 2).
    U16 = 2,
    /// Signed 16-bit integer (id 3).
    I16 = 3,
    /// Unsigned 32-bit integer (id 4).
    U32 = 4,
    /// Signed 32-bit integer (id 5).
    I32 = 5,
    /// 32-bit IEEE float (id 6).
    F32 = 6,
    /// Boolean (id 7).
    Bool = 7,
    /// UTF-8 string (id 8).
    String = 8,
    /// Array of values (id 9).
    Array = 9,
    /// Unsigned 64-bit integer (id 10).
    U64 = 10,
    /// Signed 64-bit integer (id 11).
    I64 = 11,
    /// 64-bit IEEE float (id 12).
    F64 = 12,
}

impl ValueType {
    /// Map a GGUF type id to a value type, or `None` if the id is unknown.
    #[must_use]
    pub const fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            0 => Self::U8,
            1 => Self::I8,
            2 => Self::U16,
            3 => Self::I16,
            4 => Self::U32,
            5 => Self::I32,
            6 => Self::F32,
            7 => Self::Bool,
            8 => Self::String,
            9 => Self::Array,
            10 => Self::U64,
            11 => Self::I64,
            12 => Self::F64,
            _ => return None,
        })
    }

    /// The GGUF type id for this value type.
    #[must_use]
    pub const fn id(self) -> u32 {
        self as u32
    }

    /// Lower-case name used in the inspect JSON (`"u8"`, `"f32"`, `"array"`...).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::I8 => "i8",
            Self::U16 => "u16",
            Self::I16 => "i16",
            Self::U32 => "u32",
            Self::I32 => "i32",
            Self::F32 => "f32",
            Self::Bool => "bool",
            Self::String => "string",
            Self::Array => "array",
            Self::U64 => "u64",
            Self::I64 => "i64",
            Self::F64 => "f64",
        }
    }

    /// Whether this type stores a single scalar rather than an array.
    #[must_use]
    pub const fn is_scalar(self) -> bool {
        !matches!(self, Self::Array)
    }

    /// Smallest number of bytes one encoded value of this type can occupy.
    ///
    /// Used to reject absurd array lengths before allocating for them.
    pub(crate) const fn min_encoded_len(self) -> u64 {
        match self {
            Self::U8 | Self::I8 | Self::Bool => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
            // A string is at least its 8-byte length prefix.
            Self::String => 8,
            // An array is at least its 4-byte element type and 8-byte count.
            Self::Array => 12,
        }
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A parsed metadata value.
///
/// Strings are borrowed directly from the input buffer; arrays are materialized
/// as a list of values, which may themselves be arrays.
#[derive(Debug, Clone, PartialEq)]
pub enum MetadataValue<'a> {
    /// Unsigned 8-bit integer.
    U8(u8),
    /// Signed 8-bit integer.
    I8(i8),
    /// Unsigned 16-bit integer.
    U16(u16),
    /// Signed 16-bit integer.
    I16(i16),
    /// Unsigned 32-bit integer.
    U32(u32),
    /// Signed 32-bit integer.
    I32(i32),
    /// 32-bit IEEE float.
    F32(f32),
    /// Boolean.
    Bool(bool),
    /// Borrowed UTF-8 string.
    String(&'a str),
    /// Array of values.
    Array(ArrayValue<'a>),
    /// Unsigned 64-bit integer.
    U64(u64),
    /// Signed 64-bit integer.
    I64(i64),
    /// 64-bit IEEE float.
    F64(f64),
}

impl<'a> MetadataValue<'a> {
    /// The type of this value.
    #[must_use]
    pub const fn value_type(&self) -> ValueType {
        match self {
            Self::U8(_) => ValueType::U8,
            Self::I8(_) => ValueType::I8,
            Self::U16(_) => ValueType::U16,
            Self::I16(_) => ValueType::I16,
            Self::U32(_) => ValueType::U32,
            Self::I32(_) => ValueType::I32,
            Self::F32(_) => ValueType::F32,
            Self::Bool(_) => ValueType::Bool,
            Self::String(_) => ValueType::String,
            Self::Array(_) => ValueType::Array,
            Self::U64(_) => ValueType::U64,
            Self::I64(_) => ValueType::I64,
            Self::F64(_) => ValueType::F64,
        }
    }

    /// Borrow the value as a string, if it is one.
    #[must_use]
    pub const fn as_str(&self) -> Option<&'a str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow the value as an array, if it is one.
    #[must_use]
    pub const fn as_array(&self) -> Option<&ArrayValue<'a>> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    /// The value as a `u8`, if it is one.
    #[must_use]
    pub const fn as_u8(&self) -> Option<u8> {
        match self {
            Self::U8(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as an `i8`, if it is one.
    #[must_use]
    pub const fn as_i8(&self) -> Option<i8> {
        match self {
            Self::I8(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as a `u16`, if it is one.
    #[must_use]
    pub const fn as_u16(&self) -> Option<u16> {
        match self {
            Self::U16(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as an `i16`, if it is one.
    #[must_use]
    pub const fn as_i16(&self) -> Option<i16> {
        match self {
            Self::I16(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as a `u32`, if it is one.
    #[must_use]
    pub const fn as_u32(&self) -> Option<u32> {
        match self {
            Self::U32(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as an `i32`, if it is one.
    #[must_use]
    pub const fn as_i32(&self) -> Option<i32> {
        match self {
            Self::I32(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as a `u64`, if it is one.
    #[must_use]
    pub const fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U64(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as an `i64`, if it is one.
    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I64(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as an `f32`, if it is one.
    #[must_use]
    pub const fn as_f32(&self) -> Option<f32> {
        match self {
            Self::F32(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as an `f64`, if it is one.
    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::F64(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as a boolean, if it is one.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

/// A parsed metadata array: an element type plus the decoded elements.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayValue<'a> {
    element_type: ValueType,
    values: Vec<MetadataValue<'a>>,
}

impl<'a> ArrayValue<'a> {
    /// The declared element type of the array.
    #[must_use]
    pub const fn element_type(&self) -> ValueType {
        self.element_type
    }

    /// Number of elements in the array.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.values.len() as u64
    }

    /// Whether the array has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The decoded elements, in file order.
    #[must_use]
    pub fn values(&self) -> &[MetadataValue<'a>] {
        &self.values
    }

    /// Iterate over the decoded elements.
    pub fn iter(&self) -> std::slice::Iter<'_, MetadataValue<'a>> {
        self.values.iter()
    }
}

/// One metadata key/value pair.
#[derive(Debug, Clone, PartialEq)]
pub struct MetadataEntry<'a> {
    key: &'a str,
    value: MetadataValue<'a>,
}

impl<'a> MetadataEntry<'a> {
    /// The raw metadata key, borrowed from the file.
    #[must_use]
    pub const fn key(&self) -> &'a str {
        self.key
    }

    /// The parsed value.
    #[must_use]
    pub const fn value(&self) -> &MetadataValue<'a> {
        &self.value
    }

    /// The type of the parsed value.
    #[must_use]
    pub const fn value_type(&self) -> ValueType {
        self.value.value_type()
    }
}

/// A scalar decoder that excludes [`ValueType::Array`].
///
/// Splitting scalars from arrays lets the array decoder handle nesting with an
/// explicit stack, without the recursive call that could overflow the stack.
#[derive(Debug, Clone, Copy)]
enum ScalarType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    Bool,
    String,
    U64,
    I64,
    F64,
}

impl ScalarType {
    fn from_value_type(value_type: ValueType) -> Option<Self> {
        Some(match value_type {
            ValueType::U8 => Self::U8,
            ValueType::I8 => Self::I8,
            ValueType::U16 => Self::U16,
            ValueType::I16 => Self::I16,
            ValueType::U32 => Self::U32,
            ValueType::I32 => Self::I32,
            ValueType::F32 => Self::F32,
            ValueType::Bool => Self::Bool,
            ValueType::String => Self::String,
            ValueType::U64 => Self::U64,
            ValueType::I64 => Self::I64,
            ValueType::F64 => Self::F64,
            ValueType::Array => return None,
        })
    }
}

/// Parse `count` metadata entries from the cursor.
pub(crate) fn parse_metadata<'a>(
    cursor: &mut Cursor<'a>,
    count: u64,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<MetadataEntry<'a>>> {
    if count > MAX_METADATA_ENTRIES {
        return Err(Error::TooManyMetadataEntries {
            count,
            limit: MAX_METADATA_ENTRIES,
        });
    }
    // Minimum encoded size of one entry: empty key (8), type id (4), and the
    // smallest possible value payload (1). Reject absurd counts that cannot
    // possibly fit before allocating a vector for them.
    const MIN_ENTRY_LEN: u64 = 8 + 4 + 1;
    let min_bytes = count
        .checked_mul(MIN_ENTRY_LEN)
        .ok_or(Error::ArithmeticOverflow {
            context: "metadata entry count",
        })?;
    if min_bytes > cursor.remaining() {
        return Err(Error::Truncated {
            offset: cursor.position(),
        });
    }

    let mut entries = Vec::with_capacity(capacity_hint(count));
    let mut seen_keys: HashSet<&'a str> = HashSet::new();
    for _ in 0..count {
        let key = cursor.read_string()?;
        if !key.is_ascii() {
            warnings.push(Warning::new(
                Warning::METADATA_KEY_NOT_ASCII,
                format!("metadata key {key:?} is not ASCII"),
            ));
        }
        if key.len() > MAX_METADATA_KEY_LEN {
            warnings.push(Warning::new(
                Warning::METADATA_KEY_TOO_LONG,
                format!(
                    "metadata key {key:?} is {} bytes long (limit {MAX_METADATA_KEY_LEN})",
                    key.len()
                ),
            ));
        }
        if !seen_keys.insert(key) {
            warnings.push(Warning::new(
                Warning::DUPLICATE_METADATA_KEY,
                format!("metadata key {key:?} appears more than once"),
            ));
        }
        let type_id = cursor.read_u32()?;
        let value_type = ValueType::from_id(type_id).ok_or(Error::UnknownValueType(type_id))?;
        let value = parse_value(cursor, value_type)?;
        entries.push(MetadataEntry { key, value });
    }
    Ok(entries)
}

fn capacity_hint(count: u64) -> usize {
    usize::try_from(count.min(1024)).unwrap_or(1024)
}

fn parse_value<'a>(cursor: &mut Cursor<'a>, value_type: ValueType) -> Result<MetadataValue<'a>> {
    if value_type == ValueType::Array {
        return parse_array(cursor);
    }
    // `Array` was handled above; every other type has a scalar decoder.
    let scalar =
        ScalarType::from_value_type(value_type).ok_or(Error::UnknownValueType(value_type.id()))?;
    parse_scalar(cursor, scalar)
}

fn parse_scalar<'a>(cursor: &mut Cursor<'a>, scalar: ScalarType) -> Result<MetadataValue<'a>> {
    Ok(match scalar {
        ScalarType::U8 => MetadataValue::U8(cursor.read_u8()?),
        ScalarType::I8 => MetadataValue::I8(cursor.read_i8()?),
        ScalarType::U16 => MetadataValue::U16(cursor.read_u16()?),
        ScalarType::I16 => MetadataValue::I16(cursor.read_i16()?),
        ScalarType::U32 => MetadataValue::U32(cursor.read_u32()?),
        ScalarType::I32 => MetadataValue::I32(cursor.read_i32()?),
        ScalarType::F32 => MetadataValue::F32(cursor.read_f32()?),
        ScalarType::Bool => MetadataValue::Bool(cursor.read_bool()?),
        ScalarType::String => MetadataValue::String(cursor.read_string()?),
        ScalarType::U64 => MetadataValue::U64(cursor.read_u64()?),
        ScalarType::I64 => MetadataValue::I64(cursor.read_i64()?),
        ScalarType::F64 => MetadataValue::F64(cursor.read_f64()?),
    })
}

/// One in-progress array on the explicit decoding stack.
struct ArrayFrame<'a> {
    element_type: ValueType,
    remaining: u64,
    values: Vec<MetadataValue<'a>>,
}

fn read_array_header(cursor: &mut Cursor<'_>) -> Result<(ValueType, u64)> {
    let element_type_id = cursor.read_u32()?;
    let element_type =
        ValueType::from_id(element_type_id).ok_or(Error::UnknownValueType(element_type_id))?;
    let count = cursor.read_u64()?;
    if count > MAX_ARRAY_ELEMENTS {
        return Err(Error::TooManyArrayElements {
            count,
            limit: MAX_ARRAY_ELEMENTS,
        });
    }
    // Every element takes at least `min_encoded_len` bytes, so a count that
    // cannot fit in the rest of the buffer is truncation, not corruption.
    let min_bytes =
        count
            .checked_mul(element_type.min_encoded_len())
            .ok_or(Error::ArithmeticOverflow {
                context: "metadata array byte length",
            })?;
    if min_bytes > cursor.remaining() {
        return Err(Error::Truncated {
            offset: cursor.position(),
        });
    }
    Ok((element_type, count))
}

fn parse_array<'a>(cursor: &mut Cursor<'a>) -> Result<MetadataValue<'a>> {
    let (element_type, count) = read_array_header(cursor)?;
    let mut stack: Vec<ArrayFrame<'a>> = vec![ArrayFrame {
        element_type,
        remaining: count,
        values: Vec::with_capacity(capacity_hint(count)),
    }];
    loop {
        let frame = stack.last_mut().expect("array decoding stack is non-empty");
        if frame.remaining == 0 {
            let done = stack.pop().expect("array decoding stack is non-empty");
            let value = MetadataValue::Array(ArrayValue {
                element_type: done.element_type,
                values: done.values,
            });
            match stack.last_mut() {
                Some(parent) => {
                    parent.values.push(value);
                    parent.remaining -= 1;
                }
                None => return Ok(value),
            }
            continue;
        }
        let element_type = stack
            .last()
            .expect("array decoding stack is non-empty")
            .element_type;
        if element_type == ValueType::Array {
            if stack.len() >= MAX_ARRAY_DEPTH as usize {
                return Err(Error::ArrayNestingTooDeep {
                    limit: MAX_ARRAY_DEPTH,
                });
            }
            let (nested_type, nested_count) = read_array_header(cursor)?;
            stack.push(ArrayFrame {
                element_type: nested_type,
                remaining: nested_count,
                values: Vec::with_capacity(capacity_hint(nested_count)),
            });
        } else {
            let scalar = ScalarType::from_value_type(element_type)
                .ok_or(Error::UnknownValueType(element_type.id()))?;
            let value = parse_scalar(cursor, scalar)?;
            let frame = stack.last_mut().expect("array decoding stack is non-empty");
            frame.values.push(value);
            frame.remaining -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    fn encode_entry(out: &mut Vec<u8>, key: &str, type_id: u32, payload: &[u8]) {
        encode_string(out, key);
        out.extend_from_slice(&type_id.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn parse_entries(bytes: &[u8], count: u64) -> Result<Vec<MetadataEntry<'_>>> {
        let mut cursor = Cursor::new(bytes);
        let mut warnings = Vec::new();
        parse_metadata(&mut cursor, count, &mut warnings)
    }

    #[test]
    fn parses_every_scalar_value_type() {
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "u8", 0, &[200]);
        encode_entry(&mut bytes, "i8", 1, &(-100_i8).to_le_bytes());
        encode_entry(&mut bytes, "u16", 2, &50_000_u16.to_le_bytes());
        encode_entry(&mut bytes, "i16", 3, &(-20_000_i16).to_le_bytes());
        encode_entry(&mut bytes, "u32", 4, &3_000_000_000_u32.to_le_bytes());
        encode_entry(&mut bytes, "i32", 5, &(-2_000_000_000_i32).to_le_bytes());
        encode_entry(&mut bytes, "f32", 6, &1.5_f32.to_le_bytes());
        encode_entry(&mut bytes, "bool", 7, &[1]);
        let mut string_payload = Vec::new();
        encode_string(&mut string_payload, "hello");
        encode_entry(&mut bytes, "string", 8, &string_payload);
        encode_entry(&mut bytes, "u64", 10, &u64::MAX.to_le_bytes());
        encode_entry(&mut bytes, "i64", 11, &i64::MIN.to_le_bytes());
        encode_entry(&mut bytes, "f64", 12, &(-2.5_f64).to_le_bytes());

        let entries = parse_entries(&bytes, 12).unwrap();
        assert_eq!(entries.len(), 12);
        assert_eq!(entries[0].key(), "u8");
        assert_eq!(entries[0].value(), &MetadataValue::U8(200));
        assert_eq!(entries[0].value_type(), ValueType::U8);
        assert_eq!(entries[1].value(), &MetadataValue::I8(-100));
        assert_eq!(entries[2].value(), &MetadataValue::U16(50_000));
        assert_eq!(entries[3].value(), &MetadataValue::I16(-20_000));
        assert_eq!(entries[4].value(), &MetadataValue::U32(3_000_000_000));
        assert_eq!(entries[5].value(), &MetadataValue::I32(-2_000_000_000));
        assert_eq!(entries[6].value(), &MetadataValue::F32(1.5));
        assert_eq!(entries[7].value(), &MetadataValue::Bool(true));
        assert_eq!(entries[8].value(), &MetadataValue::String("hello"));
        assert_eq!(entries[9].value(), &MetadataValue::U64(u64::MAX));
        assert_eq!(entries[10].value(), &MetadataValue::I64(i64::MIN));
        assert_eq!(entries[11].value(), &MetadataValue::F64(-2.5));

        // Accessors are type-safe: mismatched access yields None.
        assert_eq!(entries[0].value().as_u8(), Some(200));
        assert_eq!(entries[0].value().as_u32(), None);
        assert_eq!(entries[8].value().as_str(), Some("hello"));
        assert_eq!(entries[7].value().as_bool(), Some(true));
        assert_eq!(entries[6].value().as_f32(), Some(1.5));
        assert_eq!(entries[11].value().as_f64(), Some(-2.5));
        assert_eq!(entries[5].value().as_i32(), Some(-2_000_000_000));
        assert_eq!(entries[2].value().as_u16(), Some(50_000));
        assert_eq!(entries[3].value().as_i16(), Some(-20_000));
        assert_eq!(entries[1].value().as_i8(), Some(-100));
        assert_eq!(entries[9].value().as_u64(), Some(u64::MAX));
        assert_eq!(entries[10].value().as_i64(), Some(i64::MIN));
    }

    #[test]
    fn value_type_ids_and_names_round_trip() {
        let expected = [
            (0, "u8"),
            (1, "i8"),
            (2, "u16"),
            (3, "i16"),
            (4, "u32"),
            (5, "i32"),
            (6, "f32"),
            (7, "bool"),
            (8, "string"),
            (9, "array"),
            (10, "u64"),
            (11, "i64"),
            (12, "f64"),
        ];
        for (id, name) in expected {
            let value_type = ValueType::from_id(id).unwrap();
            assert_eq!(value_type.id(), id);
            assert_eq!(value_type.name(), name);
            assert_eq!(value_type.to_string(), name);
            assert_eq!(value_type.is_scalar(), id != 9);
        }
        assert!(ValueType::from_id(13).is_none());
        assert!(ValueType::from_id(u32::MAX).is_none());
    }

    #[test]
    fn parses_arrays_of_scalars_and_strings() {
        let mut bytes = Vec::new();
        // array<u32> [10, 20, 30]
        let mut payload = Vec::new();
        payload.extend_from_slice(&4_u32.to_le_bytes());
        payload.extend_from_slice(&3_u64.to_le_bytes());
        for value in [10_u32, 20, 30] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        encode_entry(&mut bytes, "u32s", 9, &payload);

        // array<string> ["a", "bb"]
        let mut payload = Vec::new();
        payload.extend_from_slice(&8_u32.to_le_bytes());
        payload.extend_from_slice(&2_u64.to_le_bytes());
        encode_string(&mut payload, "a");
        encode_string(&mut payload, "bb");
        encode_entry(&mut bytes, "strings", 9, &payload);

        let entries = parse_entries(&bytes, 2).unwrap();
        let u32s = entries[0].value().as_array().unwrap();
        assert_eq!(u32s.element_type(), ValueType::U32);
        assert_eq!(u32s.len(), 3);
        assert!(!u32s.is_empty());
        let values: Vec<u32> = u32s.iter().map(|v| v.as_u32().unwrap()).collect();
        assert_eq!(values, [10, 20, 30]);

        let strings = entries[1].value().as_array().unwrap();
        assert_eq!(strings.element_type(), ValueType::String);
        let values: Vec<&str> = strings.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(values, ["a", "bb"]);

        // Empty arrays are legal.
        let mut empty = Vec::new();
        let mut payload = Vec::new();
        payload.extend_from_slice(&6_u32.to_le_bytes());
        payload.extend_from_slice(&0_u64.to_le_bytes());
        encode_entry(&mut empty, "floats", 9, &payload);
        let entries = parse_entries(&empty, 1).unwrap();
        let floats = entries[0].value().as_array().unwrap();
        assert!(floats.is_empty());
        assert_eq!(floats.len(), 0);
        assert_eq!(floats.values().len(), 0);
    }

    #[test]
    fn parses_nested_arrays() {
        // array<array<u8>> [[1, 2], [3]]
        let mut payload = Vec::new();
        payload.extend_from_slice(&9_u32.to_le_bytes()); // element type = array
        payload.extend_from_slice(&2_u64.to_le_bytes()); // two inner arrays
        for inner in [&[1_u8, 2][..], &[3_u8][..]] {
            payload.extend_from_slice(&0_u32.to_le_bytes()); // inner element type = u8
            payload.extend_from_slice(&(inner.len() as u64).to_le_bytes());
            payload.extend_from_slice(inner);
        }
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "nested", 9, &payload);

        let entries = parse_entries(&bytes, 1).unwrap();
        let outer = entries[0].value().as_array().unwrap();
        assert_eq!(outer.element_type(), ValueType::Array);
        assert_eq!(outer.len(), 2);
        let first = outer.values()[0].as_array().unwrap();
        assert_eq!(first.element_type(), ValueType::U8);
        let first_values: Vec<u8> = first.iter().map(|v| v.as_u8().unwrap()).collect();
        assert_eq!(first_values, [1, 2]);
        let second = outer.values()[1].as_array().unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second.values()[0].as_u8(), Some(3));
    }

    #[test]
    fn deeply_nested_arrays_do_not_overflow_the_stack() {
        // 20 nested one-element arrays (below the 32-level cap): the explicit
        // stack decoder must handle nested arrays without recursion, and the
        // depth cap rejects pathological nesting before the owned value tree
        // can become deep enough to overflow the recursive drop glue.
        const DEPTH: usize = 20;
        let mut payload = Vec::with_capacity(DEPTH * 12 + 1);
        for _ in 0..DEPTH {
            payload.extend_from_slice(&9_u32.to_le_bytes());
            payload.extend_from_slice(&1_u64.to_le_bytes());
        }
        payload.extend_from_slice(&0_u32.to_le_bytes()); // innermost: array<u8>
        payload.extend_from_slice(&1_u64.to_le_bytes());
        payload.push(255);

        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "deep", 9, &payload);
        let entries = parse_entries(&bytes, 1).unwrap();
        let mut value = entries[0].value();
        let mut depth = 0;
        loop {
            let array = value.as_array().expect("every level is an array");
            depth += 1;
            assert_eq!(array.len(), 1);
            value = &array.values()[0];
            if array.element_type() != ValueType::Array {
                // The innermost array holds a single u8.
                assert_eq!(array.element_type(), ValueType::U8);
                break;
            }
        }
        assert_eq!(depth, DEPTH + 1);
        assert_eq!(value.as_u8(), Some(255));
    }

    #[test]
    fn array_nesting_beyond_the_cap_is_an_error() {
        let depth = MAX_ARRAY_DEPTH as usize + 1;
        let mut payload = Vec::new();
        for _ in 0..depth {
            payload.extend_from_slice(&9_u32.to_le_bytes());
            payload.extend_from_slice(&1_u64.to_le_bytes());
        }
        payload.extend_from_slice(&0_u32.to_le_bytes());
        payload.extend_from_slice(&1_u64.to_le_bytes());
        payload.push(0);

        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "deep", 9, &payload);
        match parse_entries(&bytes, 1).unwrap_err() {
            Error::ArrayNestingTooDeep { limit } => assert_eq!(limit, MAX_ARRAY_DEPTH),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn invalid_bool_payload_is_an_error() {
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "flag", 7, &[2]);
        match parse_entries(&bytes, 1).unwrap_err() {
            Error::InvalidBool { value, offset } => {
                assert_eq!(value, 2);
                assert!(offset > 0);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn invalid_utf8_key_or_value_is_an_error() {
        // Invalid UTF-8 in the key.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_u64.to_le_bytes());
        bytes.extend_from_slice(&[0xff, 0xfe]);
        bytes.extend_from_slice(&4_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        assert_eq!(parse_entries(&bytes, 1).unwrap_err().code(), "invalid-utf8");

        // Invalid UTF-8 in a string value.
        let mut bytes = Vec::new();
        encode_string(&mut bytes, "key");
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u64.to_le_bytes());
        bytes.push(0xff);
        assert_eq!(parse_entries(&bytes, 1).unwrap_err().code(), "invalid-utf8");
    }

    #[test]
    fn unknown_value_type_is_an_error() {
        for type_id in [13_u32, 99, u32::MAX] {
            let mut bytes = Vec::new();
            encode_entry(&mut bytes, "key", type_id, &[]);
            match parse_entries(&bytes, 1).unwrap_err() {
                Error::UnknownValueType(found) => assert_eq!(found, type_id),
                other => panic!("unexpected error: {other}"),
            }
        }
    }

    #[test]
    fn huge_entry_or_array_counts_are_rejected() {
        // Huge metadata count: cap error, not a giant allocation.
        let mut cursor = Cursor::new(&[]);
        let mut warnings = Vec::new();
        match parse_metadata(&mut cursor, u64::MAX, &mut warnings).unwrap_err() {
            Error::TooManyMetadataEntries { count, limit } => {
                assert_eq!(count, u64::MAX);
                assert_eq!(limit, MAX_METADATA_ENTRIES);
            }
            other => panic!("unexpected error: {other}"),
        }

        // Huge array count.
        let mut payload = Vec::new();
        payload.extend_from_slice(&0_u32.to_le_bytes());
        payload.extend_from_slice(&u64::MAX.to_le_bytes());
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "arr", 9, &payload);
        match parse_entries(&bytes, 1).unwrap_err() {
            Error::TooManyArrayElements { count, limit } => {
                assert_eq!(count, u64::MAX);
                assert_eq!(limit, MAX_ARRAY_ELEMENTS);
            }
            other => panic!("unexpected error: {other}"),
        }

        // Array count one above the cap.
        let mut payload = Vec::new();
        payload.extend_from_slice(&0_u32.to_le_bytes());
        payload.extend_from_slice(&(MAX_ARRAY_ELEMENTS + 1).to_le_bytes());
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "arr", 9, &payload);
        assert!(matches!(
            parse_entries(&bytes, 1).unwrap_err(),
            Error::TooManyArrayElements { .. }
        ));
    }

    #[test]
    fn array_count_that_cannot_fit_is_truncation() {
        // Claim 1000 u64 elements but provide none.
        let mut payload = Vec::new();
        payload.extend_from_slice(&10_u32.to_le_bytes());
        payload.extend_from_slice(&1000_u64.to_le_bytes());
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "arr", 9, &payload);
        assert_eq!(parse_entries(&bytes, 1).unwrap_err().code(), "truncated");
    }

    #[test]
    fn nested_array_element_type_unknown_is_an_error() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&9_u32.to_le_bytes());
        payload.extend_from_slice(&1_u64.to_le_bytes());
        payload.extend_from_slice(&99_u32.to_le_bytes()); // unknown nested element type
        payload.extend_from_slice(&0_u64.to_le_bytes());
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "arr", 9, &payload);
        match parse_entries(&bytes, 1).unwrap_err() {
            Error::UnknownValueType(99) => {}
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn key_warnings_cover_ascii_length_and_duplicates() {
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "café", 4, &7_u32.to_le_bytes());
        encode_entry(
            &mut bytes,
            &"x".repeat(MAX_METADATA_KEY_LEN + 1),
            4,
            &1_u32.to_le_bytes(),
        );
        encode_entry(&mut bytes, "dup", 4, &1_u32.to_le_bytes());
        encode_entry(&mut bytes, "dup", 4, &2_u32.to_le_bytes());

        let mut cursor = Cursor::new(&bytes);
        let mut warnings = Vec::new();
        let entries = parse_metadata(&mut cursor, 4, &mut warnings).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[3].key(), "dup");
        assert_eq!(entries[3].value(), &MetadataValue::U32(2));

        let codes: Vec<&str> = warnings.iter().map(Warning::code).collect();
        assert_eq!(
            codes,
            [
                Warning::METADATA_KEY_NOT_ASCII,
                Warning::METADATA_KEY_TOO_LONG,
                Warning::DUPLICATE_METADATA_KEY,
            ]
        );
        assert!(warnings[0].message().contains("café"));
        assert!(warnings[1].message().contains("65536"));
        assert!(warnings[2].message().contains("dup"));
    }

    #[test]
    fn trailing_bytes_are_left_for_the_caller() {
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "key", 4, &7_u32.to_le_bytes());
        let entry_len = bytes.len();
        bytes.extend_from_slice(&[9_u8, 9, 9]);
        let mut cursor = Cursor::new(&bytes);
        let mut warnings = Vec::new();
        parse_metadata(&mut cursor, 1, &mut warnings).unwrap();
        assert_eq!(cursor.position(), entry_len as u64);
        assert_eq!(cursor.remaining(), 3);
    }

    #[test]
    fn truncation_anywhere_is_a_clean_error() {
        let mut bytes = Vec::new();
        encode_entry(&mut bytes, "key", 4, &7_u32.to_le_bytes());
        let mut payload = Vec::new();
        payload.extend_from_slice(&9_u32.to_le_bytes());
        payload.extend_from_slice(&1_u64.to_le_bytes());
        payload.extend_from_slice(&0_u32.to_le_bytes());
        payload.extend_from_slice(&2_u64.to_le_bytes());
        payload.extend_from_slice(&[1, 2]);
        encode_entry(&mut bytes, "nested", 9, &payload);
        for cut in 0..bytes.len() {
            let mut cursor = Cursor::new(&bytes[..cut]);
            let mut warnings = Vec::new();
            let result = parse_metadata(&mut cursor, 2, &mut warnings);
            assert!(result.is_err(), "cut={cut} unexpectedly parsed");
        }
    }
}
