//! `hhea` table support.

use super::{Cursor, GlyphWithMetrics};
use crate::{
    write::{VecExt, WriteTable},
    ParseError, TableTag,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct HheaTable {
    /// ascender, descender, lineGap
    unparsed_after_version: [u8; 6],
    pub(crate) advance_width_max: u16,
    pub(crate) min_left_side_bearing: i16,
    pub(crate) min_right_side_bearing: i16,
    pub(crate) x_max_extent: i16,
    /// caretSlopeRise ..= metricDataFormat
    unparsed_after_extent: [u8; 16],
    pub(crate) number_of_h_metrics: u16,
}

impl HheaTable {
    const VERSION: u32 = 0x0001_0000;

    pub(super) fn parse(mut cursor: Cursor<'_>) -> Result<Self, ParseError> {
        cursor.read_u32_checked(|version| check_exact!(version, Self::VERSION))?;
        let unparsed_after_version = cursor.read_byte_array::<6>()?;
        let advance_width_max = cursor.read_u16()?;
        let min_left_side_bearing = cursor.read_i16()?;
        let min_right_side_bearing = cursor.read_i16()?;
        let x_max_extent = cursor.read_i16()?;
        let unparsed_after_extent = cursor.read_byte_array::<16>()?;
        let number_of_h_metrics = cursor.read_u16()?;

        Ok(Self {
            unparsed_after_version,
            advance_width_max,
            min_left_side_bearing,
            min_right_side_bearing,
            x_max_extent,
            unparsed_after_extent,
            number_of_h_metrics,
        })
    }

    pub(crate) fn subset(&mut self, glyphs: &[GlyphWithMetrics<'_>], number_of_h_metrics: u16) {
        let mut max_advance = 0;
        let mut max_extent = i16::MIN;
        let mut min_left_bearing = i16::MAX;
        let mut min_right_bearing = i16::MAX;
        for glyph in glyphs {
            let Some(bbox) = glyph.inner.bounding_box() else {
                continue;
            };
            max_advance = max_advance.max(glyph.advance);
            min_left_bearing = min_left_bearing.min(glyph.lsb);
            let extent = bbox.x_max - bbox.x_min + glyph.lsb;
            max_extent = max_extent.max(extent);

            let rsb = i32::from(glyph.advance) - i32::from(extent);
            if rsb < i32::from(i16::MIN) {
                min_right_bearing = i16::MIN;
            } else if let Ok(rsb) = i16::try_from(rsb) {
                min_right_bearing = min_right_bearing.min(rsb);
            }
        }

        self.advance_width_max = max_advance;
        self.x_max_extent = max_extent;
        self.min_left_side_bearing = min_left_bearing;
        self.min_right_side_bearing = min_right_bearing;
        self.number_of_h_metrics = number_of_h_metrics;
    }
}

impl WriteTable for HheaTable {
    fn tag(&self) -> TableTag {
        TableTag::HHEA
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.write_u32(Self::VERSION);
        buffer.extend_from_slice(&self.unparsed_after_version);
        buffer.write_u16(self.advance_width_max);
        buffer.write_i16(self.min_left_side_bearing);
        buffer.write_i16(self.min_right_side_bearing);
        buffer.write_i16(self.x_max_extent);
        buffer.extend_from_slice(&self.unparsed_after_extent);
        buffer.write_u16(self.number_of_h_metrics);
    }
}
