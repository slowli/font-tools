//! `cmap` table processing.

use core::ops;
use std::mem;

use super::Cursor;
use crate::{
    alloc::Vec,
    errors::ParseErrorKind,
    utils::Either,
    write::{VecExt, WriteTable},
    ParseError, TableTag,
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
        cursor.read_u16_checked(|format| check_exact!(format, 4))?;

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
            glyph_id_array: cursor.bytes(),
        })
    }

    fn map_char(&self, ch: char) -> Result<u16, ParseError> {
        let Ok(ch) = u16::try_from(ch as u32) else {
            return Ok(0); // missing glyph
        };

        let segment_idx = self
            .segments
            .binary_search_by_key(&ch, |segment| segment.end_code)
            .unwrap_or_else(|pos| pos);
        let segment = &self.segments[segment_idx];
        if segment.start_code > ch {
            return Ok(0); // missing glyph
        }

        if segment.id_range_offset == 0 {
            Ok(segment.id_delta.wrapping_add(ch))
        } else {
            // Offset is counted from the start of `idRangeOffsets`
            let mut byte_offset = 2 * segment_idx;
            byte_offset += usize::from(segment.id_range_offset);
            byte_offset += 2 * usize::from(ch - segment.start_code);

            if byte_offset < 2 * self.segments.len() {
                return Err(ParseError {
                    kind: ParseErrorKind::OffsetOutOfBounds(byte_offset),
                    offset: 0,
                    table: Some(TableTag::CMAP),
                });
            }
            // Shift the offset to count from the start of `glyphIdArray`
            byte_offset -= 2 * self.segments.len();
            let glyph_id_bytes = self
                .glyph_id_array
                .get(byte_offset..(byte_offset + 2))
                .ok_or(ParseError {
                    kind: ParseErrorKind::OffsetOutOfBounds(byte_offset),
                    offset: 0,
                    table: Some(TableTag::CMAP),
                })?;
            let glyph_id = u16::from_be_bytes(glyph_id_bytes.try_into().unwrap());
            Ok(segment.id_delta.wrapping_add(glyph_id))
        }
    }

    fn subtable_len(&self) -> usize {
        16 + 8 * self.segments.len()
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.write_u16(4); // subtable format
        buffer.write_u16(
            self.subtable_len()
                .try_into()
                .expect("subtable_len overflow"),
        );
        buffer.write_u16(0); // language

        let segment_count = u16::try_from(self.segments.len()).expect("segments.len() overflow");
        buffer.write_u16(2 * segment_count);
        let entry_selector = u16::try_from(segment_count.ilog2()).unwrap();
        let search_range = 1 << (entry_selector + 1);
        buffer.write_u16(search_range);
        buffer.write_u16(entry_selector);
        let range_shift = 2 * segment_count - search_range;
        buffer.write_u16(range_shift);

        for segment in &self.segments {
            buffer.write_u16(segment.end_code);
        }
        buffer.write_u16(0); // reserved padding
        for segment in &self.segments {
            buffer.write_u16(segment.start_code);
        }
        for segment in &self.segments {
            buffer.write_u16(segment.id_delta);
        }
        for segment in &self.segments {
            buffer.write_u16(segment.id_range_offset);
        }
        buffer.extend_from_slice(self.glyph_id_array);
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
        cursor.read_u16_checked(|format| check_exact!(format, 12))?;

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

    fn subtable_len(&self) -> usize {
        16 + 12 * self.groups.len()
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.write_u16(12); // subtable format
        buffer.write_u16(0); // reserved

        buffer.write_u32(
            self.subtable_len()
                .try_into()
                .expect("subtable_len overflow"),
        );
        buffer.write_u32(0); // language
        buffer.write_u32(self.groups.len().try_into().expect("groups.len() overflow"));
        for group in &self.groups {
            buffer.write_u32(group.start_char_code);
            buffer.write_u32(group.end_char_code);
            buffer.write_u32(group.start_glyph_id);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CmapTable<'a> {
    Deltas(SegmentDeltas<'a>),
    Coverage(SegmentedCoverage),
}

impl<'a> CmapTable<'a> {
    pub(crate) const UNICODE_PLATFORM: u16 = 0;
    pub(crate) const WINDOWS_PLATFORM: u16 = 3;

    pub(super) fn parse(mut cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let table_cursor = cursor;
        cursor.read_u16_checked(|version| check_exact!(version, 0))?;

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

            // Delta encoding has lower priority than segmented coverage because it doesn't cover
            // chars > u16::MAX.
            match expected_table_format {
                CmapTableFormat::SegmentDeltas if this.is_none() => {
                    let mut subtable = table_cursor;
                    subtable.skip(offset as usize)?;
                    this = Some(Self::Deltas(SegmentDeltas::parse(subtable)?));
                }
                CmapTableFormat::SegmentedCoverage if !matches!(&this, Some(Self::Coverage(_))) => {
                    let mut subtable = table_cursor;
                    subtable.skip(offset as usize)?;
                    this = Some(Self::Coverage(SegmentedCoverage::parse(subtable)?));
                }
                _ => { /* We've already got a necessary table; do nothing */ }
            }
        }

        this.ok_or_else(|| cursor.err(ParseErrorKind::NoSupportedCmap))
    }

    pub(super) fn map_char(&self, ch: char) -> Result<u16, ParseError> {
        match self {
            Self::Deltas(deltas) => deltas.map_char(ch),
            Self::Coverage(coverage) => Ok(coverage.map_char(ch)),
        }
    }

    pub(super) fn char_ranges(&self) -> impl Iterator<Item = ops::RangeInclusive<u32>> + '_ {
        match self {
            Self::Deltas(deltas) => {
                Either::Left(deltas.segments.iter().filter_map(|segment| {
                    if segment.start_code == u16::MAX {
                        // Filters out the last dummy segment
                        None
                    } else {
                        Some(u32::from(segment.start_code)..=u32::from(segment.end_code))
                    }
                }))
            }
            Self::Coverage(coverage) => Either::Right(
                coverage
                    .groups
                    .iter()
                    .map(|group| group.start_char_code..=group.end_char_code),
            ),
        }
    }

    #[cfg(test)]
    pub(super) fn char_range(&self) -> ops::RangeInclusive<char> {
        match self {
            Self::Deltas(deltas) => {
                let first_segment = deltas.segments.first().expect("empty deltas");
                let first = char::try_from(u32::from(first_segment.start_code)).unwrap();
                // The last segment always has single u16::MAX char as per spec.
                let last_real_segment = &deltas.segments[deltas.segments.len() - 2];
                let last = char::try_from(u32::from(last_real_segment.end_code)).unwrap();
                first..=last
            }
            Self::Coverage(coverage) => {
                let first_group = coverage.groups.first().expect("empty coverage");
                let first = char::try_from(first_group.start_char_code).expect("invalid char");
                let last_group = coverage.groups.last().expect("empty coverage");
                let last = char::try_from(last_group.end_char_code).expect("invalid char");
                first..=last
            }
        }
    }
}

impl CmapTable<'static> {
    pub(crate) fn from_map(map: &[(char, u16)]) -> Self {
        let coverage = Self::create_coverage(map);
        let can_be_encoded_as_deltas = map
            .last()
            .is_none_or(|&(ch, _)| u32::from(ch) < u32::from(u16::MAX));
        if can_be_encoded_as_deltas {
            #[allow(clippy::cast_possible_truncation)]
            // `_ as u16` is safe due to the `can_be_encoded_as_deltas` check
            let delta_segments = coverage.groups.iter().map(|group| {
                let start_code = group.start_char_code as u16;
                SegmentWithDelta {
                    start_code,
                    end_code: group.end_char_code as u16,
                    id_delta: (group.start_glyph_id as u16).wrapping_sub(start_code),
                    id_range_offset: 0,
                }
            });
            // Add en empty segment with `start_code == end_code == 0xffff` as per spec.
            let delta_segments = delta_segments.chain([SegmentWithDelta {
                start_code: u16::MAX,
                end_code: u16::MAX,
                id_delta: 1, // will map `start_code` to glyph #0 (the missing glyph) as recommended
                id_range_offset: 0,
            }]);
            Self::Deltas(SegmentDeltas {
                segments: delta_segments.collect(),
                glyph_id_array: &[],
            })
        } else {
            Self::Coverage(coverage)
        }
    }

    fn create_coverage(map: &[(char, u16)]) -> SegmentedCoverage {
        let mut groups = vec![];
        let [(first_char, first_idx), rest @ ..] = map else {
            return SegmentedCoverage::default();
        };
        let mut current_group = SequentialMapGroup {
            start_char_code: (*first_char).into(),
            end_char_code: (*first_char).into(),
            start_glyph_id: (*first_idx).into(),
        };

        for &(ch, glyph_idx) in rest {
            if u32::from(ch) == current_group.end_char_code + 1
                && u32::from(glyph_idx) == current_group.map_unchecked(ch)
            {
                current_group.end_char_code += 1;
            } else {
                let prev_group = mem::replace(
                    &mut current_group,
                    SequentialMapGroup {
                        start_char_code: ch.into(),
                        end_char_code: ch.into(),
                        start_glyph_id: glyph_idx.into(),
                    },
                );
                groups.push(prev_group);
            }
        }

        groups.push(current_group);
        SegmentedCoverage { groups }
    }
}

impl WriteTable for CmapTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::CMAP
    }

    /// Writes 2 subtables for Unicode and Windows platforms. Both subtables point at the same data.
    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        const SUBTABLE_OFFSET: u32 = 4 + 2 * 8;

        let prev_len = buffer.len();
        buffer.write_u16(0); // table version
        buffer.write_u16(2); // num_tables

        buffer.write_u16(CmapTable::UNICODE_PLATFORM);
        let encoding_id = match self {
            Self::Deltas(_) => 3,
            Self::Coverage(_) => 4,
        };
        buffer.write_u16(encoding_id);
        buffer.write_u32(SUBTABLE_OFFSET);

        buffer.write_u16(CmapTable::WINDOWS_PLATFORM);
        let encoding_id = match self {
            Self::Deltas(_) => 1,
            Self::Coverage(_) => 10,
        };
        buffer.write_u16(encoding_id);
        buffer.write_u32(SUBTABLE_OFFSET);

        debug_assert_eq!(buffer.len() - prev_len, SUBTABLE_OFFSET as usize);

        match self {
            Self::Deltas(deltas) => deltas.write_to_vec(buffer),
            Self::Coverage(coverage) => coverage.write_to_vec(buffer),
        }
    }
}
