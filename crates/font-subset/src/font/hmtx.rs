//! `htmx` table support.

use super::{Cursor, GlyphWithMetrics};
use crate::{write::VecExt, ParseError};

#[derive(Debug, Clone, Copy)]
pub(crate) struct HmtxTable<'a> {
    pub(super) raw: Cursor<'a>,
    pub(super) number_of_h_metrics: u16,
}

impl HmtxTable<'_> {
    pub(super) fn advance_and_lsb(&self, glyph_idx: u16) -> Result<(u16, u16), ParseError> {
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

    pub(crate) fn write_to_vec(glyphs: &[GlyphWithMetrics<'_>], buffer: &mut Vec<u8>) -> u16 {
        let mut number_of_h_metrics = glyphs.len();
        while let Some([prev, current]) = glyphs[..number_of_h_metrics].last_chunk::<2>() {
            if prev.advance != current.advance {
                break;
            }
            number_of_h_metrics -= 1;
        }

        for (i, glyph) in glyphs.iter().enumerate() {
            if i < number_of_h_metrics {
                buffer.write_u16(glyph.advance);
                buffer.write_u16(glyph.lsb);
            } else {
                buffer.write_u16(glyph.lsb);
            }
        }

        // `unwrap()` should be safe: `number_of_h_metrics` <= number of glyphs, which doesn't exceed u16::MAX
        number_of_h_metrics.try_into().unwrap()
    }
}
