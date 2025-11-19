//! Basic types for font parsing.

use core::{fmt, ops};

use crate::{alloc::Vec, write::VecExt, ParseError, ParseErrorKind};

/// 4-byte tag of an OpenType font table.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableTag(pub(crate) [u8; 4]);

impl_tag!(TableTag);

macro_rules! tag_const {
    ($($val:tt, $int:tt $(as $name:ident)?;)+) => {
        $(
        tag_const!(@define $val $(as $name)?);
        )+

        #[cfg(feature = "woff2")]
        pub(crate) fn as_u8(self) -> Option<u8> {
            Some(match &self.0 {
                $($val => $int,)+
                _ => return None,
            })
        }

        #[cfg(feature = "woff2")]
        pub(crate) fn from_u8(val: u8) -> Option<Self> {
            Some(match val & 63 {
                $($int => Self(*$val),)+
                _ => return None,
            })
        }
    };

    (@define $val:tt as $name:ident) => {
        #[doc = concat!("Tag for the `", stringify!($val), "` table.")]
        pub const $name: Self = Self(*$val);
    };
    (@define $val:tt) => {
        // produce nothing
    };
}

impl TableTag {
    // public tables are ones used in the `Font`
    tag_const!(
        b"cmap",  0 as CMAP;
        b"head",  1 as HEAD;
        b"hhea",  2 as HHEA;
        b"hmtx",  3 as HMTX;
        b"maxp",  4 as MAXP;
        b"name",  5 as NAME;
        b"OS/2",  6 as OS2;
        b"post",  7 as POST;
        b"cvt ",  8 as CVT;
        b"fpgm",  9 as FPGM;
        b"glyf", 10 as GLYF;
        b"loca", 11 as LOCA;
        b"prep", 12 as PREP;
        b"CFF ", 13;
        b"VORG", 14;
        b"EBDT", 15;
        b"EBLC", 16;
        b"gasp", 17;
        b"hdmx", 18;
        b"kern", 19;
        b"LTSH", 20;
        b"PCLT", 21;
        b"VDMX", 22;
        b"vhea", 23;
        b"vmtx", 24;
        b"BASE", 25;
        b"GDEF", 26;
        b"GPOS", 27;
        b"GSUB", 28;
        b"EBSC", 29;
        b"JSTF", 30;
        b"MATH", 31;
        b"CBDT", 32;
        b"CBLC", 33;
        b"COLR", 34;
        b"CPAL", 35;
        b"SVG ", 36;
        b"sbix", 37;
        b"acnt", 38;
        b"avar", 39 as AVAR;
        b"bdat", 40;
        b"bloc", 41;
        b"bsln", 42;
        b"cvar", 43;
        b"fdsc", 44;
        b"feat", 45;
        b"fmtx", 46;
        b"fvar", 47 as FVAR;
        b"gvar", 48 as GVAR;
        b"hsty", 49;
        b"just", 50;
        b"lcar", 51;
        b"mort", 52;
        b"morx", 53;
        b"opbd", 54;
        b"prop", 55;
        b"trak", 56;
        b"Zapf", 57;
        b"Silf", 58;
        b"Glat", 59;
        b"Gloc", 60;
        b"Feat", 61;
        b"Sill", 62;
    );
}

/// Fixed-point signed 32-bit value. Used in [`VariableAxis`](crate::VariableAxis) params.
///
/// This type has the 16.16 shape; i.e., it's mapped from `i32` by dividing by `65_536 == 1 << 16`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fixed(pub(super) i32);

impl fmt::Debug for Fixed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&f32::from(*self), formatter)
    }
}

impl fmt::Display for Fixed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, formatter)
    }
}

impl From<Fixed> for f32 {
    #[allow(clippy::cast_precision_loss)] // acceptable
    fn from(value: Fixed) -> Self {
        value.0 as f32 / 65_536.0_f32
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
