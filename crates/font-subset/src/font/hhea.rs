//! `hhea` table support.

use super::{Cursor, GlyphWithMetrics};
use crate::{
    alloc::Vec,
    write::{VecExt, WriteTable},
    ParseError, TableTag,
};

#[derive(Debug)]
pub(super) struct HorizontalGlyphStats {
    pub(super) advance_width_max: u16,
    pub(super) min_left_side_bearing: i16,
    pub(super) min_right_side_bearing: i16,
    pub(super) x_max_extent: i16,
}

impl Default for HorizontalGlyphStats {
    fn default() -> Self {
        Self {
            advance_width_max: 0,
            min_left_side_bearing: i16::MAX,
            min_right_side_bearing: i16::MAX,
            x_max_extent: i16::MIN,
        }
    }
}

impl HorizontalGlyphStats {
    pub(super) fn update(&mut self, glyph: &GlyphWithMetrics<'_>) {
        let Some(bbox) = glyph.inner.bounding_box() else {
            return;
        };
        self.advance_width_max = self.advance_width_max.max(glyph.advance);
        self.min_left_side_bearing = self.min_left_side_bearing.min(glyph.lsb);
        let extent = bbox.x_max - bbox.x_min + glyph.lsb;
        self.x_max_extent = self.x_max_extent.max(extent);

        let rsb = i32::from(glyph.advance) - i32::from(extent);
        if rsb < i32::from(i16::MIN) {
            self.min_right_side_bearing = i16::MIN;
        } else if let Ok(rsb) = i16::try_from(rsb) {
            self.min_right_side_bearing = self.min_right_side_bearing.min(rsb);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HheaTable {
    pub(crate) ascender: i16,
    pub(crate) descender: i16,
    pub(crate) line_gap: i16,
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

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip_all, fields(range = ?cursor.range()))
    )]
    pub(super) fn parse(mut cursor: Cursor<'_>) -> Result<Self, ParseError> {
        cursor.read_u32_checked(|version| check_exact!(version, Self::VERSION))?;
        let ascender = cursor.read_i16()?;
        let descender = cursor.read_i16()?;
        let line_gap = cursor.read_i16()?;
        let advance_width_max = cursor.read_u16()?;
        let min_left_side_bearing = cursor.read_i16()?;
        let min_right_side_bearing = cursor.read_i16()?;
        let x_max_extent = cursor.read_i16()?;
        let unparsed_after_extent = cursor.read_byte_array::<16>()?;
        let number_of_h_metrics = cursor.read_u16()?;

        #[cfg(feature = "tracing")]
        tracing::debug!(
            advance_width_max,
            min_left_side_bearing,
            min_right_side_bearing,
            x_max_extent,
            number_of_h_metrics,
            "parsed basic info"
        );

        Ok(Self {
            ascender,
            descender,
            line_gap,
            advance_width_max,
            min_left_side_bearing,
            min_right_side_bearing,
            x_max_extent,
            unparsed_after_extent,
            number_of_h_metrics,
        })
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(level = "debug", skip_all))]
    pub(crate) fn subset(&mut self, glyphs: &[GlyphWithMetrics<'_>], number_of_h_metrics: u16) {
        let mut glyph_stats = HorizontalGlyphStats::default();
        for glyph in glyphs {
            glyph_stats.update(glyph);
        }
        #[cfg(feature = "tracing")]
        tracing::debug!(?glyph_stats, "computed glyph stats");

        self.advance_width_max = glyph_stats.advance_width_max;
        self.x_max_extent = glyph_stats.x_max_extent;
        self.min_left_side_bearing = glyph_stats.min_left_side_bearing;
        self.min_right_side_bearing = glyph_stats.min_right_side_bearing;
        self.number_of_h_metrics = number_of_h_metrics;
    }
}

impl WriteTable for HheaTable {
    fn tag(&self) -> TableTag {
        TableTag::HHEA
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.write_u32(Self::VERSION);
        buffer.write_i16(self.ascender);
        buffer.write_i16(self.descender);
        buffer.write_i16(self.line_gap);
        buffer.write_u16(self.advance_width_max);
        buffer.write_i16(self.min_left_side_bearing);
        buffer.write_i16(self.min_right_side_bearing);
        buffer.write_i16(self.x_max_extent);
        buffer.extend_from_slice(&self.unparsed_after_extent);
        buffer.write_u16(self.number_of_h_metrics);
    }
}
