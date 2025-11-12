//! `cmap` table processing.

use super::{offset_bytes, read_prefix, read_u16, read_u32, skip};
use crate::errors::{MapError, ParseError};

#[derive(Debug)]
enum CmapTableFormat {
    /// Segment mapping to delta values (format 4).
    SegmentDeltas,
    /// Segmented coverage (format 12).
    SegmentedCoverage,
}

#[derive(Debug)]
pub(crate) struct SegmentWithDelta {
    pub(crate) start_code: u16,
    pub(crate) end_code: u16,
    pub(crate) id_delta: u16,
    pub(crate) id_range_offset: u16,
}

/// Segment mapping to delta values (format 4) subtable of the `cmap` table.
#[derive(Debug)]
pub(crate) struct SegmentDeltas<'a> {
    pub(crate) segments: Vec<SegmentWithDelta>,
    pub(crate) glyph_id_array: &'a [u8],
}

impl<'a> SegmentDeltas<'a> {
    fn parse(mut bytes: &'a [u8]) -> Result<Self, ParseError> {
        let format = read_u16(&mut bytes)?;
        if format != 4 {
            return Err(ParseError::UnexpectedCmapTableFormat {
                expected: 4,
                actual: format,
            });
        }
        let subtable_len = read_u16(&mut bytes)?;
        let remaining_len = subtable_len
            .checked_sub(4)
            .ok_or(ParseError::UnexpectedEof)? as usize;
        if remaining_len > bytes.len() {
            return Err(ParseError::UnexpectedEof);
        }
        bytes = &bytes[..remaining_len];

        skip(&mut bytes, 2)?; // language
        let segment_count = read_u16(&mut bytes)? / 2;
        skip(&mut bytes, 6)?; // searchRange, entrySelector, rangeShift

        let vec_len = 2 * usize::from(segment_count);
        let mut end_codes = read_prefix(&mut bytes, vec_len)?;
        skip(&mut bytes, 2)?; // reserved padding
        let mut start_codes = read_prefix(&mut bytes, vec_len)?;
        let mut id_deltas = read_prefix(&mut bytes, vec_len)?;
        let mut id_range_offsets = read_prefix(&mut bytes, vec_len)?;

        let segments = (0..segment_count).map(|_| {
            Ok(SegmentWithDelta {
                start_code: read_u16(&mut start_codes)?,
                end_code: read_u16(&mut end_codes)?,
                id_delta: read_u16(&mut id_deltas)?,
                id_range_offset: read_u16(&mut id_range_offsets)?,
            })
        });

        Ok(Self {
            segments: segments.collect::<Result<_, ParseError>>()?,
            glyph_id_array: bytes,
        })
    }

    fn map_char(&self, c: char) -> Result<u16, MapError> {
        let c = u16::try_from(c as u32).map_err(|_| MapError::CharTooLarge)?;

        let segment_idx = self
            .segments
            .binary_search_by_key(&c, |segment| segment.end_code)
            .unwrap_or_else(|pos| pos);
        let segment = &self.segments[segment_idx];
        if segment.start_code > c {
            return Ok(0); // missing glyph
        }

        if segment.id_range_offset == 0 {
            Ok(segment.id_delta.wrapping_add(c))
        } else {
            // Offset is counted from the start of `idRangeOffsets`
            let mut byte_offset = 2 * segment_idx;
            byte_offset += usize::from(segment.id_range_offset);
            byte_offset += 2 * usize::from(c - segment.start_code);

            if byte_offset < 2 * self.segments.len() {
                return Err(MapError::InvalidOffset);
            }
            // Shift the offset to count from the start of `glyphIdArray`
            byte_offset -= 2 * self.segments.len();
            let glyph_id_bytes = self
                .glyph_id_array
                .get(byte_offset..(byte_offset + 2))
                .ok_or(MapError::InvalidOffset)?;
            let glyph_id = u16::from_be_bytes(glyph_id_bytes.try_into().unwrap());
            Ok(segment.id_delta.wrapping_add(glyph_id))
        }
    }
}

#[derive(Debug)]
pub(crate) struct SequentialMapGroup {
    pub(crate) start_char_code: u32,
    pub(crate) end_char_code: u32,
    pub(crate) start_glyph_id: u32,
}

impl SequentialMapGroup {
    pub(crate) fn map_unchecked(&self, ch: char) -> u32 {
        u32::from(ch) - self.start_char_code + self.start_glyph_id
    }
}

/// Segmented coverage (format 12) subtable of the `cmap` table.
#[derive(Debug, Default)]
pub(crate) struct SegmentedCoverage {
    pub(crate) groups: Vec<SequentialMapGroup>,
}

impl SegmentedCoverage {
    fn parse(mut bytes: &[u8]) -> Result<Self, ParseError> {
        let format = read_u16(&mut bytes)?;
        if format != 12 {
            return Err(ParseError::UnexpectedCmapTableFormat {
                expected: 12,
                actual: format,
            });
        }
        skip(&mut bytes, 2)?; // reserved

        let subtable_len = read_u32(&mut bytes)?;
        let remaining_len = subtable_len
            .checked_sub(8)
            .ok_or(ParseError::UnexpectedEof)? as usize;
        if remaining_len > bytes.len() {
            return Err(ParseError::UnexpectedEof);
        }
        bytes = &bytes[..remaining_len];

        skip(&mut bytes, 4)?; // language
        let num_groups = read_u32(&mut bytes)?;
        let groups = (0..num_groups).map(|_| {
            Ok(SequentialMapGroup {
                start_char_code: read_u32(&mut bytes)?,
                end_char_code: read_u32(&mut bytes)?,
                start_glyph_id: read_u32(&mut bytes)?,
            })
        });

        Ok(Self {
            groups: groups.collect::<Result<_, ParseError>>()?,
        })
    }
}

#[derive(Debug)]
pub(crate) struct CmapTable<'a> {
    pub(crate) segment_deltas: Option<SegmentDeltas<'a>>,
    pub(crate) segmented_coverage: Option<SegmentedCoverage>,
}

impl<'a> CmapTable<'a> {
    pub(crate) const UNICODE_PLATFORM: u16 = 0;
    const WINDOWS_PLATFORM: u16 = 3;

    pub(super) fn parse(mut bytes: &'a [u8]) -> Result<Self, ParseError> {
        let table_bytes = bytes;
        let version = read_u16(&mut bytes)?;
        if version != 0 {
            return Err(ParseError::UnexpectedTableVersion {
                table: "cmap",
                version: version.into(),
            });
        }

        let num_tables = read_u16(&mut bytes)?;
        let (mut segment_deltas, mut segmented_coverage) = (None, None);
        for _ in 0..num_tables {
            let platform_id = read_u16(&mut bytes)?;
            let encoding_id = read_u16(&mut bytes)?;
            let offset = read_u32(&mut bytes)?;
            let expected_table_format = match (platform_id, encoding_id) {
                (Self::UNICODE_PLATFORM, 3) | (Self::WINDOWS_PLATFORM, 1) => {
                    CmapTableFormat::SegmentDeltas
                }
                (Self::UNICODE_PLATFORM, 4) | (Self::WINDOWS_PLATFORM, 10) => {
                    CmapTableFormat::SegmentedCoverage
                }
                _ => continue, // unsupported table format
            };

            match expected_table_format {
                CmapTableFormat::SegmentDeltas if segment_deltas.is_none() => {
                    let subtable_bytes = offset_bytes(table_bytes, offset)?;
                    segment_deltas = Some(SegmentDeltas::parse(subtable_bytes)?);
                }
                CmapTableFormat::SegmentedCoverage if segmented_coverage.is_none() => {
                    let subtable_bytes = offset_bytes(table_bytes, offset)?;
                    segmented_coverage = Some(SegmentedCoverage::parse(subtable_bytes)?);
                }
                _ => { /* We've already got a necessary table; do nothing */ }
            }
        }

        Ok(Self {
            segment_deltas,
            segmented_coverage,
        })
    }

    pub(super) fn map_char(&self, ch: char) -> Result<u16, MapError> {
        // FIXME: incorrect in the general case
        self.segment_deltas.as_ref().unwrap().map_char(ch)
    }
}
