//! `cmap` table processing.

use super::Cursor;
use crate::{
    errors::{MapError, ParseErrorKind},
    ParseError,
};

#[derive(Debug)]
enum CmapTableFormat {
    /// Segment mapping to delta values (format 4).
    SegmentDeltas,
    /// Segmented coverage (format 12).
    SegmentedCoverage,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SegmentWithDelta {
    pub(crate) start_code: u16,
    pub(crate) end_code: u16,
    pub(crate) id_delta: u16,
    pub(crate) id_range_offset: u16,
}

/// Segment mapping to delta values (format 4) subtable of the `cmap` table.
#[derive(Debug, Clone)]
pub(crate) struct SegmentDeltas<'a> {
    pub(crate) segments: Vec<SegmentWithDelta>,
    pub(crate) glyph_id_array: &'a [u8],
}

impl<'a> SegmentDeltas<'a> {
    fn parse(mut cursor: Cursor<'a>) -> Result<Self, ParseError> {
        cursor.read_u16_checked(|format| {
            if format != 4 {
                return Err(ParseErrorKind::UnexpectedTableFormat { format });
            }
            Ok(())
        })?;

        let remaining_len = cursor.read_u16_checked(|subtable_len| {
            Ok(subtable_len
                .checked_sub(4)
                .ok_or(ParseErrorKind::UnexpectedEof)? as usize)
        })?;
        cursor = cursor.range(0..remaining_len)?;

        cursor.skip(2)?; // language
        let segment_count = cursor.read_u16()? / 2;
        cursor.skip(6)?; // searchRange, entrySelector, rangeShift

        let vec_len = 2 * usize::from(segment_count);
        let mut end_codes = cursor.split_at(vec_len)?;
        cursor.skip(2)?; // reserved padding
        let mut start_codes = cursor.split_at(vec_len)?;
        let mut id_deltas = cursor.split_at(vec_len)?;
        let mut id_range_offsets = cursor.split_at(vec_len)?;

        let segments = (0..segment_count).map(|_| {
            Ok(SegmentWithDelta {
                start_code: start_codes.read_u16()?,
                end_code: end_codes.read_u16()?,
                id_delta: id_deltas.read_u16()?,
                id_range_offset: id_range_offsets.read_u16()?,
            })
        });

        Ok(Self {
            segments: segments.collect::<Result<_, ParseError>>()?,
            glyph_id_array: cursor.bytes,
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

#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Default, Clone)]
pub(crate) struct SegmentedCoverage {
    pub(crate) groups: Vec<SequentialMapGroup>,
}

impl SegmentedCoverage {
    fn parse(mut cursor: Cursor<'_>) -> Result<Self, ParseError> {
        cursor.read_u16_checked(|format| {
            if format != 12 {
                return Err(ParseErrorKind::UnexpectedTableFormat { format });
            }
            Ok(())
        })?;

        cursor.skip(2)?; // reserved

        let remaining_len = cursor.read_u32_checked(|subtable_len| {
            Ok(subtable_len
                .checked_sub(8)
                .ok_or(ParseErrorKind::UnexpectedEof)? as usize)
        })?;
        cursor = cursor.range(0..remaining_len)?;

        cursor.skip(4)?; // language
        let num_groups = cursor.read_u32()?;
        let groups = (0..num_groups).map(|_| {
            Ok(SequentialMapGroup {
                start_char_code: cursor.read_u32()?,
                end_char_code: cursor.read_u32()?,
                start_glyph_id: cursor.read_u32()?,
            })
        });

        Ok(Self {
            groups: groups.collect::<Result<_, ParseError>>()?,
        })
    }

    fn map_char(&self, ch: char) -> u16 {
        let ch = u32::from(ch);
        let group_idx = self
            .groups
            .binary_search_by_key(&ch, |group| group.end_char_code)
            .unwrap_or_else(|pos| pos);
        let Some(group) = self.groups.get(group_idx) else {
            return 0; // `ch` exceeds `end_char_code` for the last segment
        };
        if group.start_char_code > ch {
            return 0; // missing glyph
        }
        let glyph_id = ch - group.start_char_code + group.start_glyph_id;
        glyph_id.try_into().expect("glyph ID exceeds u16::MAX")
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CmapTable<'a> {
    Deltas(SegmentDeltas<'a>),
    Coverage(SegmentedCoverage),
}

impl<'a> CmapTable<'a> {
    pub(crate) const UNICODE_PLATFORM: u16 = 0;
    const WINDOWS_PLATFORM: u16 = 3;

    pub(super) fn parse(mut cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let table_cursor = cursor;
        cursor.read_u16_checked(|version| {
            if version != 0 {
                return Err(ParseErrorKind::UnexpectedTableVersion {
                    version: version.into(),
                });
            }
            Ok(())
        })?;

        let num_tables = cursor.read_u16()?;
        let mut this = None;
        for _ in 0..num_tables {
            let platform_id = cursor.read_u16()?;
            let encoding_id = cursor.read_u16()?;
            let offset = cursor.read_u32()?;
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
                CmapTableFormat::SegmentDeltas if this.is_none() => {
                    let mut subtable = table_cursor;
                    subtable.skip(offset as usize)?;
                    this = Some(Self::Deltas(SegmentDeltas::parse(subtable)?));
                }
                CmapTableFormat::SegmentedCoverage if this.is_none() => {
                    let mut subtable = table_cursor;
                    subtable.skip(offset as usize)?;
                    this = Some(Self::Coverage(SegmentedCoverage::parse(subtable)?));
                }
                _ => { /* We've already got a necessary table; do nothing */ }
            }
        }

        this.ok_or_else(|| cursor.err(ParseErrorKind::NoSupportedCmap))
    }

    pub(super) fn map_char(&self, ch: char) -> Result<u16, MapError> {
        match self {
            Self::Deltas(deltas) => deltas.map_char(ch),
            Self::Coverage(coverage) => Ok(coverage.map_char(ch)),
        }
    }
}
