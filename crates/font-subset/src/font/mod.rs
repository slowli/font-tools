//! OpenType parsing logic.

use core::ops;

pub(crate) use self::{
    cmap::CmapTable,
    glyph::{Glyph, GlyphWithMetrics},
    head::HeadTable,
    hhea::HheaTable,
    hmtx::HmtxTable,
    loca::LocaTable,
    maxp::MaxpTable,
    name::NameTable,
    os2::Os2Table,
    types::{Cursor, LocaFormat},
};
use self::{hhea::HorizontalGlyphStats, types::BoundingBox};
pub use self::{
    name::FontNaming,
    os2::{EmbeddingPermissions, UsagePermissions},
    types::TableTag,
};
use crate::{
    alloc::BTreeSet,
    errors::{ParseError, ParseErrorKind, Warnings},
    utils::RangeConcat,
    FontSubset,
};

mod cmap;
mod glyph;
mod head;
mod hhea;
mod hmtx;
mod loca;
mod maxp;
mod name;
mod os2;
mod types;

/// Shallowly parsed OpenType font.
#[derive(Debug, Clone)]
pub struct Font<'a> {
    pub(crate) cmap: CmapTable<'a>,
    pub(crate) head: HeadTable,
    pub(crate) hhea: HheaTable,
    pub(crate) hmtx: HmtxTable<'a>,
    pub(crate) maxp: MaxpTable<'a>,
    pub(crate) name: NameTable<'a>,
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
                TableTag::MAXP => maxp = Some(MaxpTable::parse(table_cursor)?),
                TableTag::NAME => name = Some(NameTable::parse(table_cursor)?),
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
        let loca = loca.ok_or_else(|| ParseError::missing_table(TableTag::LOCA))?;
        let loca = LocaTable::new(head.loca_format, maxp.glyph_count, loca)?;
        let hhea = hhea.ok_or_else(|| ParseError::missing_table(TableTag::HHEA))?;
        let hmtx = hmtx.ok_or_else(|| ParseError::missing_table(TableTag::HMTX))?;
        let hmtx = HmtxTable::parse(hmtx, maxp.glyph_count, hhea.number_of_h_metrics)?;

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

    /// Returns naming information for this font.
    pub fn naming(&self) -> &FontNaming {
        &self.name.parsed
    }

    /// Gets usage permissions for this font.
    pub fn permissions(&self) -> UsagePermissions {
        self.os2.usage_permissions
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

    /// Returns the total glyph count in this font.
    pub fn glyph_count(&self) -> usize {
        self.maxp.glyph_count.into()
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

    fn all_glyphs(&self) -> impl Iterator<Item = Result<GlyphWithMetrics<'a>, ParseError>> + '_ {
        self.loca
            .all_ranges()
            .zip(self.hmtx.iter())
            .map(|(range, (advance, lsb))| {
                let raw = self.glyf.range(range)?;
                Ok(GlyphWithMetrics {
                    inner: Glyph::new(raw)?,
                    advance,
                    lsb,
                })
            })
    }

    /// Performs some in-depth checks regarding font consistency.
    /// This involves parsing more font data, hence returning a `Result`.
    ///
    /// # Errors
    ///
    /// Returns parsing errors if any are encountered during additional parsing.
    pub fn validate(&self) -> Result<Option<Warnings>, ParseError> {
        let mut bounding_box = BoundingBox {
            x_min: i16::MAX,
            y_min: i16::MAX,
            x_max: i16::MIN,
            y_max: i16::MIN,
        };
        let mut horizontal_stats = HorizontalGlyphStats::default();

        for glyph in self.all_glyphs() {
            let glyph = glyph?;
            if let Some(bbox) = glyph.inner.bounding_box() {
                bounding_box = bounding_box.union(bbox);
            }
            horizontal_stats.update(&glyph);
        }

        let mut warnings = Warnings::empty();
        // `head` table checks
        {
            let mut warnings = warnings.for_table(TableTag::HEAD);
            warnings.check_match("x_min", bounding_box.x_min, self.head.bounding_box.x_min);
            warnings.check_match("y_min", bounding_box.y_min, self.head.bounding_box.y_min);
            warnings.check_match("x_max", bounding_box.x_max, self.head.bounding_box.x_max);
            warnings.check_match("y_max", bounding_box.y_max, self.head.bounding_box.y_max);
        }

        // `OS/2` table checks
        {
            let mut warnings = warnings.for_table(TableTag::OS2);
            let actual_range = self.cmap.char_range();
            let computed_first_char = u16::try_from(*actual_range.start()).unwrap_or(u16::MAX);
            warnings.check_match(
                "first_char_index",
                computed_first_char,
                self.os2.first_char_index,
            );
            let computed_last_char = u16::try_from(*actual_range.end()).unwrap_or(u16::MAX);
            warnings.check_match(
                "last_char_index",
                computed_last_char,
                self.os2.last_char_index,
            );
        }

        // `hhea` table checks
        {
            let mut warnings = warnings.for_table(TableTag::HHEA);
            warnings.check_match(
                "advance_width_max",
                horizontal_stats.advance_width_max,
                self.hhea.advance_width_max,
            );
            warnings.check_match(
                "x_max_extent",
                horizontal_stats.x_max_extent,
                self.hhea.x_max_extent,
            );
            warnings.check_match(
                "min_left_side_bearing",
                horizontal_stats.min_left_side_bearing,
                self.hhea.min_left_side_bearing,
            );
            warnings.check_match(
                "min_right_side_bearing",
                horizontal_stats.min_right_side_bearing,
                self.hhea.min_right_side_bearing,
            );
        }

        Ok(warnings.into_option())
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
    use std::collections::HashSet;

    use test_casing::test_casing;

    use super::*;
    use crate::{
        tests::{TestFont, FONTS},
        WarningKind,
    };

    #[test_casing(2, FONTS)]
    fn parsing_permissions(font: TestFont) {
        let font = Font::new(font.bytes).unwrap();
        let permissions = font.permissions();
        assert!(permissions.embedding.is_lenient());
        assert!(!permissions.embed_only_bitmaps);
        assert!(permissions.allow_subsetting);
    }

    #[test]
    fn parsing_name_table() {
        let font = Font::new(TestFont::FIRA_MONO.bytes).unwrap();
        let naming = font.naming();
        assert_eq!(naming.family.as_deref(), Some("Fira Mono"));
        assert_eq!(naming.subfamily.as_deref(), Some("Regular"));
        assert_eq!(
            naming.manufacturer.as_deref(),
            Some("Carrois Corporate GbR & Edenspiekermann AG")
        );
        assert_eq!(
            naming.license.as_deref(),
            Some("Licensed under the Open Font License, version 1.1 or later")
        );
        assert_eq!(
            naming.license_url.as_deref(),
            Some("http://scripts.sil.org/OFL")
        );
    }

    #[test_casing(2, FONTS)]
    fn validating_font(font: TestFont) {
        let font = Font::new(font.bytes).unwrap();
        let warnings = font.validate().unwrap();
        assert!(warnings.is_none(), "{warnings:#?}");
    }

    #[test]
    fn validating_font_with_mutations() {
        let font = Font::new(TestFont::FIRA_MONO.bytes).unwrap();

        let mut bogus_font = font.clone();
        bogus_font.head.bounding_box.x_min -= 1;
        bogus_font.head.bounding_box.y_max += 1;

        let warnings = bogus_font.validate().unwrap().expect("no warnings");
        assert_eq!(warnings.len(), 2);
        let field_names = warnings.iter().map(|warn| {
            assert_eq!(warn.table(), Some(TableTag::HEAD));
            match warn.kind() {
                WarningKind::ValueMismatch { name, .. } => *name,
            }
        });
        let field_names: HashSet<_> = field_names.collect();
        assert_eq!(field_names, HashSet::from(["x_min", "y_max"]));

        let mut bogus_font = font.clone();
        bogus_font.os2.first_char_index = 0x7f;
        bogus_font.os2.last_char_index = 0x7fff;
        bogus_font.hhea.min_right_side_bearing += 1;

        let warnings = bogus_font.validate().unwrap().expect("no warnings");
        assert_eq!(warnings.len(), 3);
        let field_names = warnings.iter().map(|warn| match warn.kind() {
            WarningKind::ValueMismatch { name, .. } => (warn.table().unwrap(), *name),
        });
        let fields: HashSet<_> = field_names.collect();
        assert_eq!(
            fields,
            HashSet::from([
                (TableTag::OS2, "first_char_index"),
                (TableTag::OS2, "last_char_index"),
                (TableTag::HHEA, "min_right_side_bearing"),
            ])
        );

        warnings.into_result().unwrap_err();
    }
}
