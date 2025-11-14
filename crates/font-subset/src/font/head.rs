//! `head` table parsing.

use super::types::{BoundingBox, Cursor, LocaFormat, LongDateTime};
use crate::{ParseError, ParseErrorKind};

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
    pub(crate) loca_format: LocaFormat,
}

impl HeadTable {
    pub(crate) const VERSION: u32 = 0x_0001_0000;
    pub(crate) const MAGIC: u32 = 0x_5f0f_3cf5;

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
            0 => Ok(LocaFormat::Short),
            1 => Ok(LocaFormat::Long),
            _ => Err(ParseErrorKind::UnexpectedValue {
                name: "loca_format",
                expected: "0 or 1".into(),
                actual: format.into(),
            }),
        })?;
        cursor.read_u16_checked(|glyph_data_format| check_exact!(glyph_data_format, 0))?;

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
}
