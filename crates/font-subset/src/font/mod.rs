//! OpenType parsing logic.

use core::ops;

use self::types::Cursor;
pub(crate) use self::{
    cmap::{CmapTable, SegmentDeltas, SegmentWithDelta, SegmentedCoverage, SequentialMapGroup},
    glyph::{Glyph, GlyphComponent, GlyphComponentArgs, GlyphWithMetrics, TransformData},
    head::HeadTable,
    os2::Os2Table,
    types::{BoundingBox, LocaFormat},
};
pub use self::{
    os2::{EmbeddingPermissions, UsagePermissions},
    types::TableTag,
};
use crate::{
    alloc::BTreeSet,
    errors::{ParseError, ParseErrorKind},
    utils::RangeConcat,
    FontSubset,
};

mod cmap;
mod glyph;
mod head;
mod os2;
mod types;

#[derive(Debug, Clone, Copy)]
pub(crate) struct HheaTable<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) number_of_h_metrics: u16,
}

impl<'a> HheaTable<'a> {
    pub(crate) const EXPECTED_LEN: usize = 36; // 18 words as per spec

    fn parse(cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let bytes = cursor.bytes();
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
pub(crate) struct LocaTable<'a> {
    format: LocaFormat,
    cursor: Cursor<'a>,
}

impl<'a> LocaTable<'a> {
    fn new(format: LocaFormat, glyph_count: u16, cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let expected_len = format.bytes_per_offset() * (glyph_count as usize + 1);
        if cursor.bytes().len() == expected_len {
            Ok(Self { format, cursor })
        } else {
            Err(cursor.err(ParseErrorKind::UnexpectedTableLen {
                expected: expected_len,
                actual: cursor.bytes().len(),
            }))
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

    #[cfg(test)]
    fn all_ranges(&self) -> impl Iterator<Item = ops::Range<usize>> + '_ {
        let parse_chunk = |chunk: &[u8]| -> usize {
            match self.format {
                LocaFormat::Short => usize::from(u16::from_be_bytes(chunk.try_into().unwrap())) * 2,
                LocaFormat::Long => u32::from_be_bytes(chunk.try_into().unwrap())
                    .try_into()
                    .unwrap(),
            }
        };

        let bytes = self.cursor.bytes();
        let (prev, bytes) = bytes.split_at(self.format.bytes_per_offset());
        let mut prev: usize = parse_chunk(prev);
        bytes
            .chunks(self.format.bytes_per_offset())
            .map(move |chunk| {
                let pos = parse_chunk(chunk);
                let range = prev..pos;
                prev = pos;
                range
            })
    }
}

/// Shallowly parsed OpenType font.
#[derive(Debug, Clone)]
pub struct Font<'a> {
    pub(crate) cmap: CmapTable<'a>,
    pub(crate) head: HeadTable,
    pub(crate) hhea: HheaTable<'a>,
    pub(crate) hmtx: HmtxTable<'a>,
    pub(crate) maxp: Cursor<'a>,
    pub(crate) name: Cursor<'a>,
    pub(crate) os2: Os2Table<'a>,
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

    // Visible for testing.
    pub(crate) fn parse_header(
        bytes: &'a [u8],
    ) -> Result<impl Iterator<Item = Result<(TableTag, Cursor<'a>), ParseError>> + 'a, ParseError>
    {
        let mut cursor = Cursor::new(bytes);
        let font_bytes = bytes;
        cursor.read_u32_checked(|sfnt_version| check_exact!(sfnt_version, Self::SFNT_VERSION))?;

        let table_count = cursor.read_u16()?;
        let expected_entry_selector = u16::try_from(table_count.ilog2()).unwrap();
        let expected_search_range = 1 << (4 + expected_entry_selector);
        cursor
            .read_u16_checked(|search_range| check_exact!(search_range, expected_search_range))?;
        cursor.read_u16_checked(|entry_selector| {
            check_exact!(entry_selector, expected_entry_selector)
        })?;
        cursor.read_u16_checked(|range_shift| {
            check_exact!(range_shift, 16 * table_count - expected_search_range)
        })?;

        Ok((0..table_count).map(move |_| Self::parse_table_record(&mut cursor, font_bytes)))
    }

    /// Parses `bytes` of an OpenType font.
    ///
    /// # Errors
    ///
    /// Returns parsing errors.
    pub fn new(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let table_records = Self::parse_header(bytes)?;

        let (mut cmap, mut head, mut hhea, mut maxp, mut hmtx) = (None, None, None, None, None);
        let (mut name, mut os2, mut post, mut loca, mut glyf) = (None, None, None, None, None);
        let (mut cvt, mut fpgm, mut prep) = (None, None, None);
        for record in table_records {
            let (tag, table_cursor) = record?;
            match tag {
                TableTag::CMAP => {
                    cmap = Some(CmapTable::parse(table_cursor)?);
                }
                TableTag::HEAD => head = Some(HeadTable::parse(table_cursor)?),
                TableTag::HHEA => hhea = Some(HheaTable::parse(table_cursor)?),
                TableTag::HMTX => hmtx = Some(table_cursor),
                TableTag::MAXP => maxp = Some(table_cursor),
                TableTag::NAME => name = Some(table_cursor),
                TableTag::OS2 => os2 = Some(Os2Table::parse(table_cursor)?),
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
        let maxp = maxp.ok_or_else(|| ParseError::missing_table(TableTag::MAXP))?;
        let glyph_count = Self::parse_glyph_count(maxp)?;
        let loca = loca.ok_or_else(|| ParseError::missing_table(TableTag::LOCA))?;
        let loca = LocaTable::new(head.loca_format, glyph_count, loca)?;
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
        if cursor.offset() % 4 != 0 {
            return Err(cursor.err(ParseErrorKind::UnalignedTable));
        }
        Ok(Self::checksum(cursor.bytes()))
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
        let cursor = Cursor::for_table(table_bytes, offset, tag);
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

    /// Gets usage permissions for this font.
    pub fn permissions(&self) -> UsagePermissions {
        self.os2.usage_permissions
    }

    fn parse_glyph_count(mut maxp_cursor: Cursor<'_>) -> Result<u16, ParseError> {
        maxp_cursor.read_u32_checked(|version| check_exact!(version, 0x_0001_0000))?;
        maxp_cursor.read_u16()
    }

    pub(crate) fn map_char(&self, ch: char) -> Result<u16, ParseError> {
        self.cmap.map_char(ch)
    }

    /// Checks whether the font contains a glyph for the specified char.
    pub fn contains_char(&self, ch: char) -> bool {
        self.cmap.map_char(ch).is_ok_and(|glyph_id| glyph_id != 0)
    }

    /// Iterates over char ranges covered by this font.
    pub fn char_ranges(&self) -> impl Iterator<Item = ops::RangeInclusive<char>> + '_ {
        RangeConcat::new(self.cmap.char_ranges()).filter_map(|range| {
            let start = char::try_from(*range.start()).ok()?;
            let end = char::try_from(*range.end()).ok()?;
            Some(start..=end)
        })
    }

    pub(crate) fn glyph(&self, glyph_idx: u16) -> Result<GlyphWithMetrics<'a>, ParseError> {
        let range = self.loca.glyph_range(glyph_idx)?;
        let raw = self.glyf.range(range)?;
        let inner = Glyph::new(raw)?;
        let (advance, lsb) = self.hmtx.advance_and_lsb(glyph_idx)?;
        Ok(GlyphWithMetrics {
            inner,
            advance,
            lsb,
        })
    }

    #[cfg(test)]
    fn all_glyphs(&self) -> impl Iterator<Item = Glyph<'a>> + '_ {
        self.loca.all_ranges().map(|range| {
            let raw = self.glyf.range(range).unwrap();
            Glyph::new(raw).unwrap()
        })
    }

    /// Subsets this font by retaining only specified `chars`.
    ///
    /// # Errors
    ///
    /// This operation will parse more font data, so it may return parsing errors.
    pub fn subset(self, chars: &BTreeSet<char>) -> Result<FontSubset<'a>, ParseError> {
        FontSubset::new(self, chars)
    }
}

#[cfg(test)]
mod tests {
    use test_casing::test_casing;

    use super::*;
    use crate::tests::{TestFont, FONTS};

    #[test_casing(2, FONTS)]
    fn head_bounding_box_is_consistent(font: TestFont) {
        let font = Font::new(font.bytes).unwrap();

        let union_bbox = font
            .all_glyphs()
            .filter_map(|glyph| glyph.bounding_box())
            .reduce(BoundingBox::union)
            .unwrap();
        assert_eq!(union_bbox, font.head.bounding_box);
    }

    #[test_casing(2, FONTS)]
    fn parsing_os2_table(font: TestFont) {
        let font = Font::new(font.bytes).unwrap();
        let permissions = font.permissions();
        assert!(permissions.embedding.is_lenient());
        assert!(!permissions.embed_only_bitmaps);
        assert!(permissions.allow_subsetting);

        let actual_range = font.cmap.char_range();
        assert_eq!(
            font.os2.first_char_index,
            u16::try_from(*actual_range.start()).unwrap()
        );
        let end_char = *actual_range.end();
        if let Ok(char) = u16::try_from(end_char) {
            assert_eq!(font.os2.last_char_index, char);
        } else {
            assert_eq!(font.os2.last_char_index, u16::MAX);
        }
    }
}
