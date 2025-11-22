//! `htmx` table support.

use core::iter;

use super::{Cursor, GlyphWithMetrics};
use crate::{
    alloc::{format, Vec},
    utils::Either,
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag,
};

#[derive(Debug, Clone)]
pub(crate) enum HmtxTable<'a> {
    Parsed {
        raw: Cursor<'a>,
        glyph_count: u16,
        number_of_h_metrics: u16,
    },
    Subset {
        advances: Vec<u16>,
        left_side_bearings: Vec<i16>,
    },
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

        Ok(Self::Parsed {
            raw,
            glyph_count,
            number_of_h_metrics,
        })
    }

    /// Iterates over `(advance, lsb)` pairs for all glyphs.
    pub(super) fn iter(&self) -> impl Iterator<Item = (u16, i16)> + '_ {
        match self {
            &Self::Parsed {
                raw,
                glyph_count,
                number_of_h_metrics,
            } => {
                let mut cursor = raw;
                let mut advance = 0;
                Either::Left((0..glyph_count).map(move |idx| {
                    if idx < number_of_h_metrics {
                        advance = cursor.read_u16().unwrap();
                        let lsb = cursor.read_i16().unwrap();
                        (advance, lsb)
                    } else {
                        let lsb = cursor.read_i16().unwrap();
                        (advance, lsb)
                    }
                }))
            }
            Self::Subset {
                advances,
                left_side_bearings,
            } => {
                let last_advance = *advances.last().unwrap();
                let advances = advances.iter().copied().chain(iter::repeat(last_advance));
                Either::Right(advances.zip(left_side_bearings.iter().copied()))
            }
        }
    }

    pub(super) fn advance_and_lsb(&self, glyph_idx: u16) -> Result<(u16, i16), ParseError> {
        let (advance, lsb);
        match self {
            &Self::Parsed {
                raw,
                number_of_h_metrics,
                ..
            } => {
                if glyph_idx < number_of_h_metrics {
                    let offset = usize::from(glyph_idx) * 4;
                    let mut cursor = raw;
                    cursor.skip(offset)?;
                    advance = cursor.read_u16()?;
                    lsb = cursor.read_i16()?;
                } else {
                    let advance_offset = usize::from(number_of_h_metrics - 1) * 4;
                    let mut read_cursor = raw;
                    read_cursor.skip(advance_offset)?;
                    advance = read_cursor.read_u16()?;

                    let lsb_offset = usize::from(number_of_h_metrics) * 4
                        + usize::from(glyph_idx - number_of_h_metrics) * 2;
                    let mut read_cursor = raw;
                    read_cursor.skip(lsb_offset)?;
                    lsb = read_cursor.read_i16()?;
                }
            }
            Self::Subset {
                advances,
                left_side_bearings,
            } => {
                let glyph_idx = usize::from(glyph_idx);
                advance = *advances
                    .get(glyph_idx)
                    .unwrap_or_else(|| advances.last().unwrap());
                lsb = left_side_bearings[glyph_idx];
            }
        }

        Ok((advance, lsb))
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", skip_all, fields(glyphs.len = glyphs.len()))
    )]
    pub(crate) fn subset(glyphs: &[GlyphWithMetrics<'_>]) -> (Self, u16) {
        let mut number_of_h_metrics = glyphs.len();
        while let Some([prev, current]) = glyphs[..number_of_h_metrics].last_chunk::<2>() {
            if prev.advance != current.advance {
                break;
            }
            number_of_h_metrics -= 1;
        }
        #[cfg(feature = "tracing")]
        tracing::debug!(number_of_h_metrics, "reduced number of metrics");

        let mut advances = Vec::with_capacity(number_of_h_metrics);
        let mut left_side_bearings = Vec::with_capacity(glyphs.len());
        for (i, glyph) in glyphs.iter().enumerate() {
            if i < number_of_h_metrics {
                advances.push(glyph.advance);
            }
            left_side_bearings.push(glyph.lsb);
        }
        let this = Self::Subset {
            advances,
            left_side_bearings,
        };
        // `unwrap()` should be safe: `number_of_h_metrics` <= number of glyphs, which doesn't exceed u16::MAX
        (this, number_of_h_metrics.try_into().unwrap())
    }
}

impl WriteTable for HmtxTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::HMTX
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        match self {
            Self::Parsed { raw, .. } => {
                buffer.extend_from_slice(raw.bytes());
            }
            Self::Subset {
                advances,
                left_side_bearings,
            } => {
                for (&advance, &lsb) in advances.iter().zip(left_side_bearings) {
                    buffer.write_u16(advance);
                    buffer.write_i16(lsb);
                }
                for &lsb in &left_side_bearings[advances.len()..] {
                    buffer.write_i16(lsb);
                }
            }
        }
    }
}
