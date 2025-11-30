//! `head` table parsing.

use super::types::{BoundingBox, Cursor, LongDateTime, OffsetFormat};
use crate::{
    alloc::Vec,
    font::GlyphWithMetrics,
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct HeadTable {
    pub(crate) font_revision: u32,
    pub(crate) checksum_adjustment: u32,
    pub(crate) flags: u16,
    pub(crate) units_per_em: u16,
    pub(crate) created: LongDateTime,
    pub(crate) modified: LongDateTime,
    pub(crate) bounding_box: BoundingBox,
    pub(crate) mac_style: u16,
    pub(crate) lowest_recommended_ppem: u16,
    pub(crate) loca_format: OffsetFormat,
}

impl HeadTable {
    const VERSION: u32 = 0x_0001_0000;
    const MAGIC: u32 = 0x_5f0f_3cf5;

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip_all, fields(range = ?cursor.range()))
    )]
    pub(super) fn parse(mut cursor: Cursor<'_>) -> Result<Self, ParseError> {
        cursor.read_u32_checked(|version| check_exact!(version, Self::VERSION))?;
        let font_revision = cursor.read_u32()?;
        let checksum_adjustment = cursor.read_u32()?;

        cursor.read_u32_checked(|magic| check_exact!(magic, Self::MAGIC))?;
        let flags = cursor.read_u16()?;
        let units_per_em = cursor.read_u16()?;
        let created = LongDateTime(cursor.read_i64()?);
        let modified = LongDateTime(cursor.read_i64()?);
        let bounding_box = BoundingBox::parse(&mut cursor)?;
        let mac_style = cursor.read_u16()?;
        let lowest_recommended_ppem = cursor.read_u16()?;
        cursor.read_u16_checked(|font_direction_hint| check_exact!(font_direction_hint, 2))?;
        let loca_format = cursor.read_u16_checked(|format| match format {
            0 => Ok(OffsetFormat::Short),
            1 => Ok(OffsetFormat::Long),
            _ => Err(ParseErrorKind::UnexpectedValue {
                name: "loca_format",
                expected: "0 or 1".into(),
                actual: format.into(),
            }),
        })?;
        cursor.read_u16_checked(|glyph_data_format| check_exact!(glyph_data_format, 0))?;

        #[cfg(feature = "tracing")]
        tracing::debug!(
            font_revision,
            flags,
            units_per_em,
            lowest_recommended_ppem,
            ?bounding_box,
            ?loca_format,
            "parsed `head`"
        );

        Ok(Self {
            font_revision,
            checksum_adjustment,
            flags,
            units_per_em,
            created,
            modified,
            bounding_box,
            mac_style,
            lowest_recommended_ppem,
            loca_format,
        })
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(level = "debug", skip_all))]
    pub(crate) fn subset(&mut self, loca_format: OffsetFormat, glyphs: &[GlyphWithMetrics<'_>]) {
        const LOSSLESS_DATA_FLAG: u16 = 1 << 11;

        let bounding_box = glyphs
            .iter()
            .filter_map(|glyph| glyph.inner.bounding_box())
            .reduce(BoundingBox::union)
            .unwrap_or(BoundingBox::ZERO);
        #[cfg(feature = "tracing")]
        tracing::debug!(?loca_format, ?bounding_box, "updating table data");

        self.checksum_adjustment = 0; // will be adjusted later
        self.flags |= LOSSLESS_DATA_FLAG;
        self.loca_format = loca_format;
        self.bounding_box = bounding_box;
    }
}

impl WriteTable for HeadTable {
    fn tag(&self) -> TableTag {
        TableTag::HEAD
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.write_u32(Self::VERSION);
        buffer.write_u32(self.font_revision);

        // checksum_adjustment. Set to 0 to correctly compute the table checksum; adjusted at the end.
        buffer.write_u32(0);

        buffer.write_u32(Self::MAGIC);
        buffer.write_u16(self.flags);
        buffer.write_u16(self.units_per_em);
        buffer.write_i64(self.created.0);
        buffer.write_i64(self.modified.0);
        self.bounding_box.write_to_vec(buffer);
        buffer.write_u16(self.mac_style);
        buffer.write_u16(self.lowest_recommended_ppem);
        buffer.write_u16(2); // fontDirectionHint
        buffer.write_u16(self.loca_format as u16);
        buffer.write_u16(0); // glyphDataFormat
    }
}
