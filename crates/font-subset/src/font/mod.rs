//! OpenType parsing logic.

use core::{fmt, ops};

#[cfg(feature = "woff2")]
pub use self::woff2::Woff2Reader;
pub(crate) use self::{
    cmap::CmapTable,
    fvar::FvarTable,
    glyph::{GlyfTable, Glyph, GlyphWithMetrics},
    head::HeadTable,
    hhea::HheaTable,
    hmtx::HmtxTable,
    loca::LocaTable,
    maxp::MaxpTable,
    name::NameTable,
    os2::Os2Table,
    post::PostTable,
    types::{Cursor, OffsetFormat},
};
pub use self::{
    fvar::{VariationAxis, VariationAxisTag},
    name::FontNaming,
    os2::{EmbeddingPermissions, UsagePermissions},
    types::{Fixed, TableTag},
};
use self::{hhea::HorizontalGlyphStats, types::BoundingBox};
use crate::{
    alloc::{format, BTreeSet, Cow, Vec},
    errors::{ParseError, ParseErrorKind, Warnings},
    font::gvar::GvarTable,
    subset::FontSubset,
    utils::{Either, RangeConcat},
};

mod cmap;
mod fvar;
mod glyph;
mod gvar;
mod head;
mod hhea;
mod hmtx;
mod loca;
mod maxp;
mod name;
mod os2;
mod post;
mod types;
#[cfg(feature = "woff2")]
mod woff2;

/// Reader for OpenType files (`.otf` / `.ttf`). Borrows data from an external source.
#[derive(Debug, Clone)]
pub struct OpenTypeReader<'a> {
    tables: Vec<(TableTag, Cursor<'a>)>,
}

impl<'a> OpenTypeReader<'a> {
    /// Creates a reader from the specified raw bytes.
    ///
    /// This will parse the OpenType header and table records.
    ///
    /// # Errors
    ///
    /// Returns parsing errors if any are encountered.
    #[allow(clippy::missing_panics_doc)] // false positive
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(
            level = "debug",
            name = "OpenTypeReader::new",
            err,
            skip_all,
            fields(bytes.len = bytes.len()),
        )
    )]
    pub fn new(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let mut cursor = Cursor::new(bytes);
        let font_bytes = bytes;
        cursor.read_u32_checked(|sfnt_version| check_exact!(sfnt_version, Font::SFNT_VERSION))?;

        let table_count = cursor.read_u16()?;
        #[cfg(feature = "tracing")]
        tracing::debug!(table_count, "read table count");

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

        let tables = (0..table_count)
            .map(|_| Self::parse_table_record(&mut cursor, font_bytes))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { tables })
    }

    fn aligned_checksum(cursor: &Cursor<'_>) -> Result<u32, ParseError> {
        if cursor.offset() % 4 != 0 {
            return Err(cursor.err(ParseErrorKind::UnalignedTable));
        }
        Ok(Font::checksum(cursor.bytes()))
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
                &table_bytes[Font::HEAD_CHECKSUM_OFFSET..Font::HEAD_CHECKSUM_OFFSET + 4];
            let adjustment = u32::from_be_bytes(adjustment.try_into().unwrap());
            actual_checksum = actual_checksum.wrapping_sub(adjustment);
        }

        if checksum != actual_checksum {
            return Err(cursor.err(ParseErrorKind::Checksum {
                expected: checksum,
                actual: actual_checksum,
            }));
        }

        #[cfg(feature = "tracing")]
        tracing::debug!(?tag, checksum, offset, len, "read table record");

        Ok((tag, cursor))
    }

    // visible for testing
    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = (TableTag, Cursor<'a>)> + '_ {
        self.tables.iter().copied()
    }

    /// Iterates over all tables in the file (including ones that are not processed by [`Font`]).
    pub fn raw_tables(&self) -> impl ExactSizeIterator<Item = (TableTag, &'a [u8])> + '_ {
        self.tables
            .iter()
            .map(|(tag, cursor)| (*tag, cursor.bytes()))
    }

    /// Reads a [`Font`] from this reader. The font will borrow data from the underlying source.
    ///
    /// # Errors
    ///
    /// Returns parsing errors (e.g., on missing required tables).
    pub fn read(&self) -> Result<Font<'a>, ParseError> {
        Font::from_tables(self.iter())
    }
}

/// Supported font formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileFormat {
    /// OpenType / TrueType font (`.ttf` / `.otf` extension).
    OpenType,
    /// WOFF2 font.
    #[cfg(feature = "woff2")]
    Woff2,
}

impl fmt::Display for FileFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OpenType => "OpenType",
            #[cfg(feature = "woff2")]
            Self::Woff2 => "WOFF2",
        })
    }
}

/// Generic font reader that auto-detects the file format based on its first bytes.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FontReader<'a> {
    /// OpenType reader.
    OpenType(OpenTypeReader<'a>),
    /// WOFF2 reader.
    #[cfg(feature = "woff2")]
    Woff2(Woff2Reader),
}

impl<'a> FontReader<'a> {
    /// Creates a reader.
    ///
    /// # Errors
    ///
    /// Returns parsing errors if any are encountered. This includes the case when the file format cannot be detected.
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", name = "FontReader::new", skip_all,)
    )]
    pub fn new(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let format = Cursor::new(bytes).read_u32_checked(|signature| match signature {
            Font::SFNT_VERSION => Ok(FileFormat::OpenType),
            #[cfg(feature = "woff2")]
            Font::WOFF2_SIGNATURE => Ok(FileFormat::Woff2),
            _ => {
                #[cfg(not(feature = "woff2"))]
                let expected = format!("OpenType ({:x}) signature", Font::SFNT_VERSION);
                #[cfg(feature = "woff2")]
                let expected = format!(
                    "OpenType ({:x}) or WOFF2 ({:x}) signature",
                    Font::SFNT_VERSION,
                    Font::WOFF2_SIGNATURE
                );

                Err(ParseErrorKind::UnexpectedValue {
                    name: "signature",
                    expected,
                    actual: signature,
                })
            }
        })?;
        #[cfg(feature = "tracing")]
        tracing::debug!(?format, "detected font file format");

        match format {
            FileFormat::OpenType => OpenTypeReader::new(bytes).map(Self::OpenType),
            #[cfg(feature = "woff2")]
            FileFormat::Woff2 => Woff2Reader::new(bytes).map(Self::Woff2),
        }
    }

    /// Returns the font format.
    pub fn format(&self) -> FileFormat {
        match self {
            Self::OpenType(_) => FileFormat::OpenType,
            #[cfg(feature = "woff2")]
            Self::Woff2(_) => FileFormat::Woff2,
        }
    }

    /// Iterates over all tables in the file (including ones that are not processed by [`Font`]).
    pub fn raw_tables(&self) -> impl ExactSizeIterator<Item = (TableTag, &[u8])> + '_ {
        #[cfg(not(feature = "woff2"))]
        match self {
            Self::OpenType(reader) => reader.raw_tables(),
        }

        #[cfg(feature = "woff2")]
        match self {
            Self::OpenType(reader) => Either::Left(reader.raw_tables()),
            Self::Woff2(reader) => Either::Right(reader.raw_tables()),
        }
    }

    /// Reads a [`Font`] from this reader. The font may borrow data from this reader.
    ///
    /// # Errors
    ///
    /// Returns parsing errors (e.g., on missing required tables).
    pub fn read(&self) -> Result<Font<'_>, ParseError> {
        match self {
            Self::OpenType(reader) => reader.read(),
            #[cfg(feature = "woff2")]
            Self::Woff2(reader) => reader.read(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VariableFontTables<'a> {
    pub(crate) fvar: FvarTable<'a>,
    pub(crate) gvar: GvarTable<'a>,
    pub(crate) unparsed: Vec<(TableTag, Cursor<'a>)>,
}

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
    pub(crate) post: PostTable<'a>,
    pub(crate) loca: LocaTable<'a>,
    pub(crate) glyf: GlyfTable<'a>,
    pub(crate) variable: Option<VariableFontTables<'a>>,
    /// Unparsed tables in the order of their appearance in the source font.
    pub(crate) unparsed: Vec<(TableTag, Cursor<'a>)>,
}

impl<'a> Font<'a> {
    pub(crate) const SFNT_VERSION: u32 = 0x_0001_0000;
    pub(crate) const SFNT_CHECKSUM: u32 = 0x_b1b0_afba;
    pub(crate) const SFNT_HEADER_LEN: usize = 12;
    pub(crate) const TABLE_RECORD_LEN: usize = 16;

    /// Offset of the checksum in the `head` table.
    pub(crate) const HEAD_CHECKSUM_OFFSET: usize = 8;

    /// Parses `bytes` as an OpenType font. This is a shortcut for instantiating and reading
    /// from an [`OpenTypeReader`].
    ///
    /// # Errors
    ///
    /// Returns parsing errors.
    pub fn opentype(bytes: &'a [u8]) -> Result<Self, ParseError> {
        OpenTypeReader::new(bytes)?.read()
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip_all)
    )]
    fn from_tables(
        table_records: impl Iterator<Item = (TableTag, Cursor<'a>)>,
    ) -> Result<Self, ParseError> {
        let (mut cmap, mut head, mut hhea, mut maxp, mut hmtx) = (None, None, None, None, None);
        let (mut name, mut os2, mut post, mut loca, mut glyf) = (None, None, None, None, None);
        let (mut fvar, mut gvar) = (None, None);
        let (mut unparsed, mut unparsed_var) = (Vec::new(), Vec::new());
        for (tag, table_cursor) in table_records {
            match tag {
                TableTag::CMAP => {
                    cmap = Some(CmapTable::parse(table_cursor)?);
                }
                TableTag::HEAD => head = Some(HeadTable::parse(table_cursor)?),
                TableTag::HHEA => hhea = Some(HheaTable::parse(table_cursor)?),
                TableTag::HMTX => hmtx = Some(table_cursor),
                TableTag::MAXP => maxp = Some(MaxpTable::parse(table_cursor)?),
                TableTag::NAME => name = Some(table_cursor),
                TableTag::OS2 => os2 = Some(Os2Table::parse(table_cursor)?),
                TableTag::POST => post = Some(table_cursor),
                TableTag::LOCA => loca = Some(table_cursor),
                TableTag::GLYF => glyf = Some(table_cursor),
                TableTag::FVAR => fvar = Some(FvarTable::parse(table_cursor)?),
                TableTag::GVAR => gvar = Some(table_cursor),
                tag if tag.is_variable() => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!(?tag, "unparsed variation table");
                    unparsed_var.push((tag, table_cursor));
                }
                _ => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!(?tag, "unparsed table");
                    unparsed.push((tag, table_cursor));
                }
            }
        }

        let head = head.ok_or_else(|| ParseError::missing_table(TableTag::HEAD))?;
        let maxp = maxp.ok_or_else(|| ParseError::missing_table(TableTag::MAXP))?;
        let loca = loca.ok_or_else(|| ParseError::missing_table(TableTag::LOCA))?;
        let loca = LocaTable::new(head.loca_format, maxp.glyph_count, loca)?;
        let hhea = hhea.ok_or_else(|| ParseError::missing_table(TableTag::HHEA))?;
        let hmtx = hmtx.ok_or_else(|| ParseError::missing_table(TableTag::HMTX))?;
        let hmtx = HmtxTable::parse(hmtx, maxp.glyph_count, hhea.number_of_h_metrics)?;
        let glyf = glyf.ok_or_else(|| ParseError::missing_table(TableTag::GLYF))?;
        let post = post.ok_or_else(|| ParseError::missing_table(TableTag::POST))?;
        let post = PostTable::new(post);

        let name = name.ok_or_else(|| ParseError::missing_table(TableTag::NAME))?;
        let additional_ids = fvar
            .as_ref()
            .map_or_else(Vec::new, FvarTable::axis_name_ids);
        let name = NameTable::parse(name, &additional_ids)?;

        let variable = if let Some(mut fvar) = fvar {
            fvar.resolve_axe_names(&name);
            let gvar = gvar
                .map(|cursor| GvarTable::parse(cursor, maxp.glyph_count))
                .ok_or_else(|| ParseError::missing_table(TableTag::GVAR))??;
            Some(VariableFontTables {
                fvar,
                gvar,
                unparsed: unparsed_var,
            })
        } else {
            None
        };

        Ok(Self {
            cmap: cmap.ok_or_else(|| ParseError::missing_table(TableTag::CMAP))?,
            head,
            hhea,
            hmtx,
            maxp,
            name,
            os2: os2.ok_or_else(|| ParseError::missing_table(TableTag::OS2))?,
            post,
            loca,
            glyf: GlyfTable::Parsed(glyf),
            variable,
            unparsed,
        })
    }

    pub(crate) fn checksum(bytes: &[u8]) -> u32 {
        bytes.chunks(4).fold(0_u32, |acc, chunk| {
            debug_assert!(chunk.len() <= 4);
            let mut u32_bytes = [0_u8; 4];
            u32_bytes[..chunk.len()].copy_from_slice(chunk);
            acc.wrapping_add(u32::from_be_bytes(u32_bytes))
        })
    }

    /// Returns naming information for this font.
    pub fn naming(&self) -> &FontNaming {
        &self.name.parsed
    }

    /// Gets usage permissions for this font.
    pub fn permissions(&self) -> UsagePermissions {
        self.os2.usage_permissions
    }

    /// Checks whether this font is variable. This returns `true` iff [`Self::variation_axes()`]
    /// returns `Some(_)`.
    pub fn is_variable(&self) -> bool {
        self.variable.is_some()
    }

    /// Returns variation axes for this font.
    pub fn variation_axes(&self) -> Option<&[VariationAxis]> {
        Some(self.variable.as_ref()?.fvar.axes())
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
        RangeConcat::new(self.cmap.char_ranges())
    }

    /// Returns the total glyph count in this font.
    pub fn glyph_count(&self) -> usize {
        self.maxp.glyph_count.into()
    }

    pub(crate) fn glyph(&self, glyph_idx: u16) -> Result<GlyphWithMetrics<'a>, ParseError> {
        match &self.glyf {
            GlyfTable::Parsed(cursor) => {
                let range = self.loca.glyph_range(glyph_idx)?;
                let raw = cursor.read_range(range)?;
                let inner = Glyph::new(raw)?;
                let (advance, lsb) = self.hmtx.advance_and_lsb(glyph_idx)?;
                Ok(GlyphWithMetrics {
                    inner,
                    advance,
                    lsb,
                })
            }
            GlyfTable::Subset(glyphs) => Ok(glyphs[usize::from(glyph_idx)].clone()),
        }
    }

    fn all_glyphs(
        &self,
    ) -> impl Iterator<Item = Result<Cow<'_, GlyphWithMetrics<'a>>, ParseError>> + '_ {
        match &self.glyf {
            &GlyfTable::Parsed(cursor) => {
                Either::Left(self.loca.all_ranges().zip(self.hmtx.iter()).map(
                    move |(range, (advance, lsb))| {
                        let raw = cursor.read_range(range)?;
                        Ok(Cow::Owned(GlyphWithMetrics {
                            inner: Glyph::new(raw)?,
                            advance,
                            lsb,
                        }))
                    },
                ))
            }
            GlyfTable::Subset(glyphs) => {
                Either::Right(glyphs.iter().map(|glyph| Ok(Cow::Borrowed(glyph))))
            }
        }
    }

    /// Drops variable font tables if they are present.
    pub fn drop_variation(&mut self) {
        self.variable = None;
    }

    /// Performs some in-depth checks regarding font consistency.
    /// This involves parsing more font data, hence returning a `Result`.
    ///
    /// # Errors
    ///
    /// Returns parsing errors if any are encountered during additional parsing.
    pub fn validate(&self) -> Result<Warnings, ParseError> {
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

        Ok(warnings)
    }

    /// Subsets this font by retaining only specified `chars`.
    ///
    /// # Errors
    ///
    /// This operation will parse more font data, so it may return parsing errors.
    pub fn subset(&self, chars: &BTreeSet<char>) -> Result<Self, ParseError> {
        FontSubset::subset(self, chars)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use allsorts::{binary::read::ReadScope, font::MatchingPresentation, font_data::FontData};
    use test_casing::test_casing;

    use super::*;
    use crate::{testonly::TestFont, WarningKind};

    #[test_casing(3, TestFont::ALL)]
    fn reading_font(font: TestFont) {
        let parsed_font = Font::opentype(font.bytes).unwrap();

        let font_file = ReadScope::new(font.bytes).read::<FontData>().unwrap();
        let font_provider = font_file.table_provider(0).unwrap();
        let mut reference_font = allsorts::Font::new(font_provider).unwrap();

        let char_count = parsed_font
            .char_ranges()
            .map(Iterator::count)
            .sum::<usize>();
        assert!(char_count > 100, "{char_count}");

        for ch in parsed_font.char_ranges().flatten() {
            assert!(parsed_font.contains_char(ch));

            let glyph_id = parsed_font.map_char(ch).unwrap();
            let (expected_id, _) =
                reference_font.lookup_glyph_index(ch, MatchingPresentation::NotRequired, None);
            assert_eq!(glyph_id, expected_id);
        }

        for range in parsed_font.char_ranges() {
            if let Some(prev) = (char::MIN..*range.start()).next_back() {
                assert!(!parsed_font.contains_char(prev));
            }
            if let Some(ch) = (*range.end()..).nth(1) {
                assert!(!parsed_font.contains_char(ch));
            }
        }
    }

    #[test_casing(3, TestFont::ALL)]
    fn parsing_permissions(font: TestFont) {
        let font = Font::opentype(font.bytes).unwrap();
        let permissions = font.permissions();
        assert!(permissions.embedding.is_lenient());
        assert!(!permissions.embed_only_bitmaps);
        assert!(permissions.allow_subsetting);
    }

    #[test]
    fn parsing_name_table() {
        let font = Font::opentype(TestFont::FIRA_MONO.bytes).unwrap();
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

    #[test_casing(3, TestFont::ALL)]
    fn validating_font(font: TestFont) {
        let font = Font::opentype(font.bytes).unwrap();
        font.validate().unwrap().into_result().unwrap();
    }

    #[test]
    fn validating_font_with_mutations() {
        let font = Font::opentype(TestFont::FIRA_MONO.bytes).unwrap();

        let mut bogus_font = font.clone();
        bogus_font.head.bounding_box.x_min -= 1;
        bogus_font.head.bounding_box.y_max += 1;

        let warnings = bogus_font
            .validate()
            .unwrap()
            .into_result()
            .expect_err("no warnings");
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

        let warnings = bogus_font
            .validate()
            .unwrap()
            .into_result()
            .expect_err("no warnings");
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
