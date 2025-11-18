//! Basic types for font parsing.

use core::{fmt, ops};

use crate::{alloc::Vec, write::VecExt, ParseError, ParseErrorKind};

/// 4-byte tag of an OpenType font table.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableTag(pub(crate) [u8; 4]);

impl_tag!(TableTag);

impl TableTag {
    pub(crate) const CMAP: Self = Self(*b"cmap");
    pub(crate) const HEAD: Self = Self(*b"head");
    pub(crate) const HHEA: Self = Self(*b"hhea");
    pub(crate) const HMTX: Self = Self(*b"hmtx");
    pub(crate) const MAXP: Self = Self(*b"maxp");
    pub(crate) const NAME: Self = Self(*b"name");
    pub(crate) const OS2: Self = Self(*b"OS/2");
    pub(crate) const POST: Self = Self(*b"post");
    pub(crate) const LOCA: Self = Self(*b"loca");
    pub(crate) const GLYF: Self = Self(*b"glyf");
    pub(crate) const CVT: Self = Self(*b"cvt ");
    pub(crate) const FPGM: Self = Self(*b"fpgm");
    pub(crate) const PREP: Self = Self(*b"prep");
    pub(crate) const FVAR: Self = Self(*b"fvar");
    pub(crate) const AVAR: Self = Self(*b"avar");
    pub(crate) const GVAR: Self = Self(*b"gvar");
}

/// Fixed-point 32-bit value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fixed(pub(super) i32);

impl fmt::Debug for Fixed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&f32::from(*self), formatter)
    }
}

impl fmt::Display for Fixed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&f32::from(*self), formatter)
    }
}

impl From<Fixed> for f32 {
    #[allow(clippy::cast_precision_loss)]
    fn from(value: Fixed) -> Self {
        value.0 as f32 * 2.0_f32.powi(-16)
    }
}

impl From<i16> for Fixed {
    fn from(value: i16) -> Self {
        Self(i32::from(value) << 16)
    }
}

/// Font reading cursor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cursor<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) offset: usize,
    pub(super) table: Option<TableTag>,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            table: None,
        }
    }

    pub(super) fn for_table(bytes: &'a [u8], offset: usize, table: TableTag) -> Self {
        Self {
            bytes,
            offset,
            table: Some(table),
        }
    }

    pub(crate) fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub(super) fn offset(&self) -> usize {
        self.offset
    }

    pub(super) fn err(&self, kind: ParseErrorKind) -> ParseError {
        ParseError {
            kind,
            offset: self.offset,
            table: self.table,
        }
    }

    pub(super) fn skip(&mut self, n: usize) -> Result<(), ParseError> {
        if self.bytes.len() < n {
            Err(self.err(ParseErrorKind::UnexpectedEof))
        } else {
            self.bytes = &self.bytes[n..];
            self.offset += n;
            Ok(())
        }
    }

    pub(super) fn read_u16(&mut self) -> Result<u16, ParseError> {
        let [a, b, rest @ ..] = self.bytes else {
            return Err(self.err(ParseErrorKind::UnexpectedEof));
        };
        self.bytes = rest;
        self.offset += 2;
        Ok(u16::from_be_bytes([*a, *b]))
    }

    #[allow(clippy::cast_possible_wrap)] // intentional
    pub(super) fn read_i16(&mut self) -> Result<i16, ParseError> {
        Ok(self.read_u16()? as i16)
    }

    pub(super) fn read_u16_checked<T>(
        &mut self,
        check: impl FnOnce(u16) -> Result<T, ParseErrorKind>,
    ) -> Result<T, ParseError> {
        check(self.read_u16()?).map_err(|kind| ParseError {
            kind,
            table: self.table,
            offset: self.offset - 2, // use the starting offset for the value
        })
    }

    pub(super) fn read_u32(&mut self) -> Result<u32, ParseError> {
        let [a, b, c, d, rest @ ..] = self.bytes else {
            return Err(self.err(ParseErrorKind::UnexpectedEof));
        };
        self.bytes = rest;
        self.offset += 4;
        Ok(u32::from_be_bytes([*a, *b, *c, *d]))
    }

    #[allow(clippy::cast_possible_wrap)] // intentional
    pub(super) fn read_i32(&mut self) -> Result<i32, ParseError> {
        Ok(self.read_u32()? as i32)
    }

    pub(super) fn read_u32_checked<T>(
        &mut self,
        check: impl FnOnce(u32) -> Result<T, ParseErrorKind>,
    ) -> Result<T, ParseError> {
        check(self.read_u32()?).map_err(|kind| ParseError {
            kind,
            table: self.table,
            offset: self.offset - 4, // use the starting offset for the value
        })
    }

    pub(super) fn read_u64(&mut self) -> Result<u64, ParseError> {
        let u64_bytes = self
            .bytes
            .first_chunk::<8>()
            .ok_or_else(|| self.err(ParseErrorKind::UnexpectedEof))?;

        self.bytes = &self.bytes[8..];
        self.offset += 8;
        Ok(u64::from_be_bytes(*u64_bytes))
    }

    #[allow(clippy::cast_possible_wrap)] // intentional
    pub(super) fn read_i64(&mut self) -> Result<i64, ParseError> {
        Ok(self.read_u64()? as i64)
    }

    pub(super) fn read_u128(&mut self) -> Result<u128, ParseError> {
        let u128_bytes = self
            .bytes
            .first_chunk::<16>()
            .ok_or_else(|| self.err(ParseErrorKind::UnexpectedEof))?;

        self.bytes = &self.bytes[16..];
        self.offset += 16;
        Ok(u128::from_be_bytes(*u128_bytes))
    }

    pub(super) fn read_byte_array<const N: usize>(&mut self) -> Result<[u8; N], ParseError> {
        if self.bytes.len() < N {
            Err(self.err(ParseErrorKind::UnexpectedEof))
        } else {
            let (head, tail) = self.bytes.split_at(N);
            self.bytes = tail;
            self.offset += N;
            Ok(head.try_into().unwrap())
        }
    }

    pub(super) fn range(&self, range: ops::Range<usize>) -> Result<Self, ParseError> {
        let bytes = self.bytes.get(range.clone()).ok_or_else(|| {
            self.err(ParseErrorKind::RangeOutOfBounds {
                range: range.clone(),
                len: self.bytes.len(),
            })
        })?;
        Ok(Self {
            bytes,
            offset: self.offset + range.start,
            table: self.table,
        })
    }

    pub(super) fn split_at(&mut self, pos: usize) -> Result<Self, ParseError> {
        let prefix = self.range(0..pos)?;
        self.skip(pos)?;
        Ok(prefix)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LongDateTime(pub(crate) i64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoundingBox {
    pub(crate) x_min: i16,
    pub(crate) y_min: i16,
    pub(crate) x_max: i16,
    pub(crate) y_max: i16,
}

impl BoundingBox {
    pub(super) const BYTE_LEN: usize = 8;

    pub(crate) const ZERO: Self = Self {
        x_min: 0,
        y_min: 0,
        x_max: 0,
        y_max: 0,
    };

    pub(super) fn parse(cursor: &mut Cursor<'_>) -> Result<Self, ParseError> {
        let x_min = cursor.read_i16()?;
        let y_min = cursor.read_i16()?;
        let x_max = cursor.read_i16()?;
        let y_max = cursor.read_i16()?;
        Ok(Self {
            x_min,
            y_min,
            x_max,
            y_max,
        })
    }

    pub(crate) fn write_to_vec(self, buffer: &mut Vec<u8>) {
        buffer.write_i16(self.x_min);
        buffer.write_i16(self.y_min);
        buffer.write_i16(self.x_max);
        buffer.write_i16(self.y_max);
    }

    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            x_min: self.x_min.min(other.x_min),
            y_min: self.y_min.min(other.y_min),
            x_max: self.x_max.max(other.x_max),
            y_max: self.y_max.max(other.y_max),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum OffsetFormat {
    Short,
    Long,
}

impl OffsetFormat {
    pub(super) const fn bytes_per_offset(self) -> usize {
        match self {
            Self::Short => 2,
            Self::Long => 4,
        }
    }
}
