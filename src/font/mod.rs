//! OpenType parsing logic.

use core::{fmt, ops};

pub(crate) use self::{
    cmap::{CmapTable, SegmentDeltas, SegmentWithDelta, SegmentedCoverage, SequentialMapGroup},
    glyph::{Glyph, GlyphComponent, GlyphComponentArgs, GlyphWithMetrics, TransformData},
};
use crate::errors::{MapError, ParseError, ParseErrorKind};

mod cmap;
mod glyph;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TableTag(pub(crate) [u8; 4]);

impl fmt::Debug for TableTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(s) = core::str::from_utf8(&self.0) {
            fmt::Debug::fmt(&s, formatter)
        } else {
            write!(formatter, "{:x}", u32::from_be_bytes(self.0))
        }
    }
}

impl From<u32> for TableTag {
    fn from(val: u32) -> Self {
        Self(val.to_be_bytes())
    }
}

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
}

/// Font reading cursor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    table: Option<TableTag>,
}

impl AsRef<[u8]> for Cursor<'_> {
    fn as_ref(&self) -> &[u8] {
        self.bytes
    }
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            table: None,
        }
    }

    fn err(&self, kind: ParseErrorKind) -> ParseError {
        ParseError {
            kind,
            offset: self.offset,
            table: self.table,
        }
    }

    fn skip(&mut self, n: usize) -> Result<(), ParseError> {
        if self.bytes.len() < n {
            Err(self.err(ParseErrorKind::UnexpectedEof))
        } else {
            self.bytes = &self.bytes[n..];
            self.offset += n;
            Ok(())
        }
    }

    fn read_u16(&mut self) -> Result<u16, ParseError> {
        let [a, b, rest @ ..] = self.bytes else {
            return Err(self.err(ParseErrorKind::UnexpectedEof));
        };
        self.bytes = rest;
        self.offset += 2;
        Ok(u16::from_be_bytes([*a, *b]))
    }

    fn read_u16_checked<T>(
        &mut self,
        check: impl FnOnce(u16) -> Result<T, ParseErrorKind>,
    ) -> Result<T, ParseError> {
        check(self.read_u16()?).map_err(|kind| ParseError {
            kind,
            table: self.table,
            offset: self.offset - 2, // use the starting offset for the value
        })
    }

    fn read_u32(&mut self) -> Result<u32, ParseError> {
        let [a, b, c, d, rest @ ..] = self.bytes else {
            return Err(self.err(ParseErrorKind::UnexpectedEof));
        };
        self.bytes = rest;
        self.offset += 4;
        Ok(u32::from_be_bytes([*a, *b, *c, *d]))
    }

    fn read_u32_checked<T>(
        &mut self,
        check: impl FnOnce(u32) -> Result<T, ParseErrorKind>,
    ) -> Result<T, ParseError> {
        check(self.read_u32()?).map_err(|kind| ParseError {
            kind,
            table: self.table,
            offset: self.offset - 4, // use the starting offset for the value
        })
    }

    fn read_byte_array<const N: usize>(&mut self) -> Result<[u8; N], ParseError> {
        if self.bytes.len() < N {
            Err(self.err(ParseErrorKind::UnexpectedEof))
        } else {
            let (head, tail) = self.bytes.split_at(N);
            self.bytes = tail;
            self.offset += N;
            Ok(head.try_into().unwrap())
        }
    }

    fn range(&self, range: ops::Range<usize>) -> Result<Self, ParseError> {
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

    fn split_at(&mut self, pos: usize) -> Result<Self, ParseError> {
        let prefix = self.range(0..pos)?;
        self.skip(pos)?;
        Ok(prefix)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HheaTable<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) number_of_h_metrics: u16,
}

impl<'a> HheaTable<'a> {
    pub(crate) const EXPECTED_LEN: usize = 36; // 18 words as per spec

    fn parse(cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let bytes = cursor.bytes;
        if bytes.len() != Self::EXPECTED_LEN {
            return Err(cursor.err(ParseErrorKind::UnexpectedTableLen {
                expected: Self::EXPECTED_LEN,
                actual: bytes.len(),
            }));
        }
        let number_of_h_metrics =
            u16::from_be_bytes([bytes[Self::EXPECTED_LEN - 2], bytes[Self::EXPECTED_LEN - 1]]);
        Ok(Self {
            raw: bytes,
            number_of_h_metrics,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HmtxTable<'a> {
    raw: Cursor<'a>,
    number_of_h_metrics: u16,
}

impl HmtxTable<'_> {
    fn advance_and_lsb(&self, glyph_idx: u16) -> Result<(u16, u16), ParseError> {
        let (advance, lsb);
        if glyph_idx < self.number_of_h_metrics {
            let offset = usize::from(glyph_idx) * 4;
            let mut cursor = self.raw;
            cursor.skip(offset)?;
            advance = cursor.read_u16()?;
            lsb = cursor.read_u16()?;
        } else {
            let advance_offset = usize::from(self.number_of_h_metrics - 1) * 4;
            let mut read_cursor = self.raw;
            read_cursor.skip(advance_offset)?;
            advance = read_cursor.read_u16()?;

            let lsb_offset = usize::from(self.number_of_h_metrics) * 4
                + usize::from(glyph_idx - self.number_of_h_metrics) * 2;
            let mut read_cursor = self.raw;
            read_cursor.skip(lsb_offset)?;
            lsb = read_cursor.read_u16()?;
        }
        Ok((advance, lsb))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum LocaFormat {
    Short,
    Long,
}

impl LocaFormat {
    const fn bytes_per_offset(self) -> usize {
        match self {
            Self::Short => 2,
            Self::Long => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LocaTable<'a> {
    format: LocaFormat,
    cursor: Cursor<'a>,
}

impl<'a> LocaTable<'a> {
    fn new(format: LocaFormat, glyph_count: u16, cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let expected_len = format.bytes_per_offset() * (glyph_count as usize + 1);
        if cursor.bytes.len() != expected_len {
            Err(cursor.err(ParseErrorKind::UnexpectedTableLen {
                expected: expected_len,
                actual: cursor.bytes.len(),
            }))
        } else {
            Ok(Self { format, cursor })
        }
    }

    fn glyph_range(&self, glyph_idx: u16) -> Result<ops::Range<usize>, ParseError> {
        let glyph_idx = usize::from(glyph_idx);
        Ok(match self.format {
            LocaFormat::Short => {
                let mut cursor = self.cursor;
                cursor.skip(glyph_idx * 2)?;
                let start_offset = usize::from(cursor.read_u16()?) * 2;
                let end_offset = usize::from(cursor.read_u16()?) * 2;
                start_offset..end_offset
            }
            LocaFormat::Long => {
                let mut cursor = self.cursor;
                cursor.skip(glyph_idx * 4)?;
                let start_offset = cursor.read_u32()? as usize;
                let end_offset = cursor.read_u32()? as usize;
                start_offset..end_offset
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct Font<'a> {
    pub(crate) cmap: CmapTable<'a>,
    pub(crate) head: Cursor<'a>,
    pub(crate) hhea: HheaTable<'a>,
    pub(crate) hmtx: HmtxTable<'a>,
    pub(crate) maxp: Cursor<'a>,
    pub(crate) name: Cursor<'a>,
    pub(crate) os2: Cursor<'a>,
    pub(crate) post: Cursor<'a>,
    pub(crate) loca: LocaTable<'a>,
    pub(crate) glyf: Cursor<'a>,
    pub(crate) cvt: Option<Cursor<'a>>,
    pub(crate) fpgm: Option<Cursor<'a>>,
    pub(crate) prep: Option<Cursor<'a>>,
}

impl<'a> Font<'a> {
    pub(crate) const SFNT_VERSION: u32 = 0x_0001_0000;
    pub(crate) const SFNT_CHECKSUM: u32 = 0x_b1b0_afba;

    /// Offset of the checksum in the `head` table.
    pub(crate) const HEAD_CHECKSUM_OFFSET: usize = 8;

    pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let mut cursor = Cursor::new(bytes);
        let font_bytes = bytes;
        let sfnt_version = cursor.read_u32()?;
        if sfnt_version != Self::SFNT_VERSION {
            return Err(cursor.err(ParseErrorKind::UnexpectedFontVersion));
        }
        let table_count = cursor.read_u16()?;
        cursor.skip(6)?; // searchRange, entrySelector, rangeShift

        let table_records =
            (0..table_count).map(|_| Self::parse_table_record(&mut cursor, font_bytes));

        let (mut cmap, mut head, mut hhea, mut maxp, mut hmtx) = (None, None, None, None, None);
        let (mut name, mut os2, mut post, mut loca, mut glyf) = (None, None, None, None, None);
        let (mut cvt, mut fpgm, mut prep) = (None, None, None);
        for record in table_records {
            let (tag, table_cursor) = record?;
            match tag {
                TableTag::CMAP => {
                    cmap = Some(CmapTable::parse(table_cursor)?);
                }
                TableTag::HEAD => head = Some(table_cursor),
                TableTag::HHEA => hhea = Some(HheaTable::parse(table_cursor)?),
                TableTag::HMTX => hmtx = Some(table_cursor),
                TableTag::MAXP => maxp = Some(table_cursor),
                TableTag::NAME => name = Some(table_cursor),
                TableTag::OS2 => os2 = Some(table_cursor),
                TableTag::POST => post = Some(table_cursor),
                TableTag::LOCA => loca = Some(table_cursor),
                TableTag::GLYF => glyf = Some(table_cursor),
                TableTag::CVT => cvt = Some(table_cursor),
                TableTag::FPGM => fpgm = Some(table_cursor),
                TableTag::PREP => prep = Some(table_cursor),
                _ => { /* skip table */ }
            }
        }

        let head = head.ok_or_else(|| ParseError::missing_table(TableTag::HEAD))?;
        let loca_format = Self::parse_loca_format(head)?;
        let maxp = maxp.ok_or_else(|| ParseError::missing_table(TableTag::MAXP))?;
        let glyph_count = Self::parse_glyph_count(maxp)?;
        let loca = loca.ok_or_else(|| ParseError::missing_table(TableTag::LOCA))?;
        let loca = LocaTable::new(loca_format, glyph_count, loca)?;
        let hhea = hhea.ok_or_else(|| ParseError::missing_table(TableTag::HHEA))?;
        let hmtx = HmtxTable {
            raw: hmtx.ok_or_else(|| ParseError::missing_table(TableTag::HMTX))?,
            number_of_h_metrics: hhea.number_of_h_metrics,
        };

        Ok(Self {
            cmap: cmap.ok_or_else(|| ParseError::missing_table(TableTag::CMAP))?,
            head,
            hhea,
            hmtx,
            maxp,
            name: name.ok_or_else(|| ParseError::missing_table(TableTag::NAME))?,
            os2: os2.ok_or_else(|| ParseError::missing_table(TableTag::OS2))?,
            post: post.ok_or_else(|| ParseError::missing_table(TableTag::POST))?,
            loca,
            glyf: glyf.ok_or_else(|| ParseError::missing_table(TableTag::GLYF))?,
            cvt,
            fpgm,
            prep,
        })
    }

    fn aligned_checksum(cursor: &Cursor<'_>) -> Result<u32, ParseError> {
        if cursor.offset % 4 != 0 {
            return Err(cursor.err(ParseErrorKind::UnalignedTable));
        }
        Ok(Self::checksum(cursor.bytes))
    }

    pub(crate) fn checksum(bytes: &[u8]) -> u32 {
        bytes.chunks(4).fold(0_u32, |acc, chunk| {
            debug_assert!(chunk.len() <= 4);
            let mut u32_bytes = [0_u8; 4];
            u32_bytes[..chunk.len()].copy_from_slice(chunk);
            acc.wrapping_add(u32::from_be_bytes(u32_bytes))
        })
    }

    fn parse_table_record(
        header_cursor: &mut Cursor<'_>,
        font_bytes: &'a [u8],
    ) -> Result<(TableTag, Cursor<'a>), ParseError> {
        let tag = TableTag::from(header_cursor.read_u32()?);
        let checksum = header_cursor.read_u32()?;
        let offset = header_cursor.read_u32()? as usize;
        let len = header_cursor.read_u32()? as usize;
        let table_bytes = font_bytes.get(offset..(offset + len)).ok_or_else(|| {
            header_cursor.err(ParseErrorKind::RangeOutOfBounds {
                range: offset..(offset + len),
                len: font_bytes.len(),
            })
        })?;
        let cursor = Cursor {
            bytes: table_bytes,
            offset,
            table: Some(tag),
        };
        let mut actual_checksum = Self::aligned_checksum(&cursor)?;
        if tag == TableTag::HEAD {
            // Zero out the checksum adjustment field.
            let adjustment =
                &table_bytes[Self::HEAD_CHECKSUM_OFFSET..Self::HEAD_CHECKSUM_OFFSET + 4];
            let adjustment = u32::from_be_bytes(adjustment.try_into().unwrap());
            actual_checksum = actual_checksum.wrapping_sub(adjustment);
        }

        if checksum != actual_checksum {
            return Err(cursor.err(ParseErrorKind::Checksum {
                expected: checksum,
                actual: actual_checksum,
            }));
        }

        Ok((tag, cursor))
    }

    fn parse_loca_format(mut head_cursor: Cursor<'_>) -> Result<LocaFormat, ParseError> {
        head_cursor.read_u32_checked(|version| {
            if version != 0x_0001_0000 {
                return Err(ParseErrorKind::UnexpectedTableVersion { version });
            }
            Ok(())
        })?;

        head_cursor.skip(46)?;
        // ^ fontRevision, checksumAdjustment, magicNumber, flags, unitsPerEm, created, modified,
        // bounding box, macStyle, lowestRecPPEM, fontDirectionHint

        head_cursor.read_u16_checked(|format| match format {
            0 => Ok(LocaFormat::Short),
            1 => Ok(LocaFormat::Long),
            _ => Err(ParseErrorKind::UnexpectedTableFormat { format }),
        })
    }

    fn parse_glyph_count(mut maxp_cursor: Cursor<'_>) -> Result<u16, ParseError> {
        maxp_cursor.read_u32_checked(|version| {
            if version != 0x_0000_5000 && version != 0x_0001_0000 {
                return Err(ParseErrorKind::UnexpectedTableVersion { version });
            }
            Ok(())
        })?;
        maxp_cursor.read_u16()
    }

    pub(crate) fn map_char(&self, ch: char) -> Result<u16, MapError> {
        self.cmap.map_char(ch)
    }

    pub(crate) fn glyph(&self, glyph_idx: u16) -> Result<GlyphWithMetrics<'a>, ParseError> {
        let range = self.loca.glyph_range(glyph_idx)?;
        let raw = self.glyf.range(range.clone())?;
        let inner = Glyph::new(raw)?;
        let (advance, lsb) = self.hmtx.advance_and_lsb(glyph_idx)?;
        Ok(GlyphWithMetrics {
            inner,
            advance,
            lsb,
        })
    }
}
