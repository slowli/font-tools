//! `loca` table support.

use core::ops;

use super::{Cursor, LocaFormat};
use crate::{alloc::Vec, write::VecExt, ParseError, ParseErrorKind};

#[derive(Debug, Clone, Copy)]
pub(crate) struct LocaTable<'a> {
    format: LocaFormat,
    cursor: Cursor<'a>,
}

impl<'a> LocaTable<'a> {
    pub(super) fn new(
        format: LocaFormat,
        glyph_count: u16,
        cursor: Cursor<'a>,
    ) -> Result<Self, ParseError> {
        let expected_len = format.bytes_per_offset() * (glyph_count as usize + 1);
        if cursor.bytes().len() == expected_len {
            Ok(Self { format, cursor })
        } else {
            Err(cursor.err(ParseErrorKind::UnexpectedTableLen {
                expected: expected_len,
                actual: cursor.bytes().len(),
            }))
        }
    }

    pub(super) fn glyph_range(&self, glyph_idx: u16) -> Result<ops::Range<usize>, ParseError> {
        let glyph_idx = usize::from(glyph_idx);
        Ok(match self.format {
            LocaFormat::Short => {
                let mut cursor = self.cursor;
                cursor.skip(glyph_idx * 2)?;
                let start_offset = usize::from(cursor.read_u16()?) * 2;
                let end_offset = usize::from(cursor.read_u16()?) * 2;
                start_offset..end_offset
            }
            LocaFormat::Long => {
                let mut cursor = self.cursor;
                cursor.skip(glyph_idx * 4)?;
                let start_offset = cursor.read_u32()? as usize;
                let end_offset = cursor.read_u32()? as usize;
                start_offset..end_offset
            }
        })
    }

    pub(super) fn all_ranges(&self) -> impl Iterator<Item = ops::Range<usize>> + '_ {
        let parse_chunk = |chunk: &[u8]| -> usize {
            // `chunk.try_into().unwrap()` are safe by construction; `chunk`s have appropriate length
            match self.format {
                LocaFormat::Short => usize::from(u16::from_be_bytes(chunk.try_into().unwrap())) * 2,
                LocaFormat::Long => u32::from_be_bytes(chunk.try_into().unwrap())
                    .try_into()
                    .expect("16-bit usize isn't supported"),
            }
        };

        let bytes = self.cursor.bytes();
        let (prev, bytes) = bytes.split_at(self.format.bytes_per_offset());
        let mut prev: usize = parse_chunk(prev);
        bytes
            .chunks(self.format.bytes_per_offset())
            .map(move |chunk| {
                let pos = parse_chunk(chunk);
                let range = prev..pos;
                prev = pos;
                range
            })
    }

    pub(crate) fn write_to_vec(locations: &[usize], buffer: &mut Vec<u8>) -> LocaFormat {
        let all_even = locations.iter().all(|&loc| loc % 2 == 0);
        let in_bounds = locations
            .last()
            .is_none_or(|&loc| loc <= usize::from(u16::MAX) * 2);
        if all_even && in_bounds {
            for &loc in locations {
                #[allow(clippy::cast_possible_truncation)]
                // doesn't happen due to the preceding check
                buffer.write_u16((loc / 2) as u16);
            }
            LocaFormat::Short
        } else {
            for &loc in locations {
                buffer.write_u32(u32::try_from(loc).expect("glyph location overflow"));
            }
            LocaFormat::Long
        }
    }
}
