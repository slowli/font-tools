//! `htmx` table support.

use super::{Cursor, GlyphWithMetrics};
use crate::{
    alloc::{format, Vec},
    write::VecExt,
    ParseError, ParseErrorKind,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct HmtxTable<'a> {
    raw: Cursor<'a>,
    glyph_count: u16,
    number_of_h_metrics: u16,
}

impl<'a> HmtxTable<'a> {
    pub(super) fn parse(
        raw: Cursor<'a>,
        glyph_count: u16,
        number_of_h_metrics: u16,
    ) -> Result<Self, ParseError> {
        // These checks allow to ensure that `self.iter()` returns metrics for all glyphs.
        if number_of_h_metrics > glyph_count {
            return Err(raw.err(ParseErrorKind::UnexpectedValue {
                name: "number_of_h_metrics",
                expected: format!("<= glyph count ({glyph_count})"),
                actual: number_of_h_metrics.into(),
            }));
        } else if number_of_h_metrics == 0 {
            return Err(raw.err(ParseErrorKind::UnexpectedValue {
                name: "number_of_h_metrics",
                expected: "positive value".into(),
                actual: number_of_h_metrics.into(),
            }));
        }

        let expected_len = usize::from(number_of_h_metrics) * 4
            + usize::from(glyph_count - number_of_h_metrics) * 2;
        if raw.bytes().len() != expected_len {
            return Err(raw.err(ParseErrorKind::UnexpectedTableLen {
                expected: expected_len,
                actual: raw.bytes().len(),
            }));
        }

        Ok(Self {
            raw,
            glyph_count,
            number_of_h_metrics,
        })
    }

    /// Iterates over `(advance, lsb)` pairs for all glyphs.
    pub(super) fn iter(&self) -> impl Iterator<Item = (u16, i16)> + '_ {
        let mut cursor = self.raw;
        let mut advance = 0;
        (0..self.glyph_count).map(move |idx| {
            if idx < self.number_of_h_metrics {
                advance = cursor.read_u16().unwrap();
                let lsb = cursor.read_i16().unwrap();
                (advance, lsb)
            } else {
                let lsb = cursor.read_i16().unwrap();
                (advance, lsb)
            }
        })
    }

    pub(super) fn advance_and_lsb(&self, glyph_idx: u16) -> Result<(u16, i16), ParseError> {
        let (advance, lsb);
        if glyph_idx < self.number_of_h_metrics {
            let offset = usize::from(glyph_idx) * 4;
            let mut cursor = self.raw;
            cursor.skip(offset)?;
            advance = cursor.read_u16()?;
            lsb = cursor.read_i16()?;
        } else {
            let advance_offset = usize::from(self.number_of_h_metrics - 1) * 4;
            let mut read_cursor = self.raw;
            read_cursor.skip(advance_offset)?;
            advance = read_cursor.read_u16()?;

            let lsb_offset = usize::from(self.number_of_h_metrics) * 4
                + usize::from(glyph_idx - self.number_of_h_metrics) * 2;
            let mut read_cursor = self.raw;
            read_cursor.skip(lsb_offset)?;
            lsb = read_cursor.read_i16()?;
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
                buffer.write_i16(glyph.lsb);
            } else {
                buffer.write_i16(glyph.lsb);
            }
        }

        // `unwrap()` should be safe: `number_of_h_metrics` <= number of glyphs, which doesn't exceed u16::MAX
        number_of_h_metrics.try_into().unwrap()
    }
}
