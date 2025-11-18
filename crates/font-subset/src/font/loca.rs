//! `loca` table support.

use core::ops;

use super::{Cursor, OffsetFormat};
use crate::{
    alloc::Vec,
    utils::Either,
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag,
};

#[derive(Debug, Clone)]
pub(crate) enum LocaTable<'a> {
    Parsed {
        format: OffsetFormat,
        cursor: Cursor<'a>,
    },
    Subset {
        format: OffsetFormat,
        offsets: Vec<usize>,
    },
}

impl<'a> LocaTable<'a> {
    pub(super) fn new(
        format: OffsetFormat,
        glyph_count: u16,
        cursor: Cursor<'a>,
    ) -> Result<Self, ParseError> {
        let expected_len = format.bytes_per_offset() * (glyph_count as usize + 1);
        if cursor.bytes().len() == expected_len {
            Ok(Self::Parsed { format, cursor })
        } else {
            Err(cursor.err(ParseErrorKind::UnexpectedTableLen {
                expected: expected_len,
                actual: cursor.bytes().len(),
            }))
        }
    }

    pub(crate) fn format(&self) -> OffsetFormat {
        match self {
            Self::Parsed { format, .. } | Self::Subset { format, .. } => *format,
        }
    }

    pub(super) fn glyph_range(&self, glyph_idx: u16) -> Result<ops::Range<usize>, ParseError> {
        let glyph_idx = usize::from(glyph_idx);

        Ok(match self {
            &Self::Parsed {
                format: OffsetFormat::Short,
                cursor,
            } => {
                let mut cursor = cursor;
                cursor.skip(glyph_idx * 2)?;
                let start_offset = usize::from(cursor.read_u16()?) * 2;
                let end_offset = usize::from(cursor.read_u16()?) * 2;
                start_offset..end_offset
            }
            &Self::Parsed {
                format: OffsetFormat::Long,
                cursor,
            } => {
                let mut cursor = cursor;
                cursor.skip(glyph_idx * 4)?;
                let start_offset = cursor.read_u32()? as usize;
                let end_offset = cursor.read_u32()? as usize;
                start_offset..end_offset
            }
            Self::Subset { offsets, .. } => offsets[glyph_idx]..offsets[glyph_idx + 1],
        })
    }

    pub(super) fn all_ranges(&self) -> impl Iterator<Item = ops::Range<usize>> + '_ {
        match self {
            &Self::Parsed { format, cursor } => {
                let parse_chunk = move |chunk: &[u8]| -> usize {
                    // `chunk.try_into().unwrap()` are safe by construction; `chunk`s have appropriate length
                    match format {
                        OffsetFormat::Short => {
                            usize::from(u16::from_be_bytes(chunk.try_into().unwrap())) * 2
                        }
                        OffsetFormat::Long => u32::from_be_bytes(chunk.try_into().unwrap())
                            .try_into()
                            .expect("16-bit usize isn't supported"),
                    }
                };

                let bytes = cursor.bytes();
                let (prev, bytes) = bytes.split_at(format.bytes_per_offset());
                let mut prev: usize = parse_chunk(prev);
                Either::Left(bytes.chunks(format.bytes_per_offset()).map(move |chunk| {
                    let pos = parse_chunk(chunk);
                    let range = prev..pos;
                    prev = pos;
                    range
                }))
            }
            Self::Subset { offsets, .. } => Either::Right(offsets.windows(2).map(|window| {
                let &[from, to] = window else {
                    unreachable!();
                };
                from..to
            })),
        }
    }

    pub(crate) fn subset(offsets: Vec<usize>) -> Self {
        let all_even = offsets.iter().all(|&loc| loc % 2 == 0);
        let in_bounds = offsets
            .last()
            .is_none_or(|&loc| loc <= usize::from(u16::MAX) * 2);
        let format = if all_even && in_bounds {
            OffsetFormat::Short
        } else {
            OffsetFormat::Long
        };
        Self::Subset { format, offsets }
    }
}

impl WriteTable for LocaTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::LOCA
    }

    #[allow(clippy::cast_possible_truncation)] // checked during subsetting
    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        match self {
            Self::Parsed { cursor, .. } => {
                buffer.extend_from_slice(cursor.bytes());
            }
            Self::Subset { format, offsets } => {
                for &offset in offsets {
                    match format {
                        OffsetFormat::Short => buffer.write_u16((offset / 2) as u16),
                        OffsetFormat::Long => buffer.write_u32(offset as u32),
                    }
                }
            }
        }
    }
}
