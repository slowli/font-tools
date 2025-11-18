//! `gvar` table.

use crate::{
    font::{Cursor, OffsetFormat},
    ParseError, ParseErrorKind,
};

#[derive(Debug)]
pub(super) struct GlyphVariationData<'a> {
    all_bytes: &'a [u8],
    tuple_refs: Vec<(usize, u16)>,
}

impl<'a> GlyphVariationData<'a> {
    fn parse(
        mut cursor: Cursor<'a>,
        axis_count: u16,
        shared_tuple_count: u16,
    ) -> Result<Self, ParseError> {
        const EMBEDDED_PEAK_TUPLE_MASK: u16 = 0x8000;
        const INTERMEDIATE_REGION_MASK: u16 = 0x4000;

        let axis_count = usize::from(axis_count);
        let all_bytes = cursor.bytes();
        let start_offset = cursor.offset;
        let tuple_variation_count = cursor.read_u16()? & 0x0fff;
        cursor.skip(2)?; // data_offset

        let mut tuple_refs = Vec::new();
        for _ in 0..tuple_variation_count {
            cursor.skip(2)?; // variation_data_size
            let tuple_index = cursor.read_u16()?;
            let has_embedded_tuple = tuple_index & EMBEDDED_PEAK_TUPLE_MASK != 0;
            let has_intermediate_region = tuple_index & INTERMEDIATE_REGION_MASK != 0;

            if has_embedded_tuple {
                cursor.skip(2 * axis_count)?; // peak_tuple
            } else {
                let tuple_index = tuple_index & 0x0fff;
                if tuple_index >= shared_tuple_count {
                    return Err(cursor.err(ParseErrorKind::UnexpectedValue {
                        name: "tuple_index",
                        expected: format!("< numer of shared tuples ({shared_tuple_count})"),
                        actual: tuple_index.into(),
                    }));
                }
                let current_offset = cursor.offset - start_offset;
                tuple_refs.push((current_offset, tuple_index));
            }
            if has_intermediate_region {
                cursor.skip(4 * axis_count)?; // intermediate_start_tuple, intermediate_end_tuple
            }
        }
        Ok(Self {
            all_bytes,
            tuple_refs,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GvarTable<'a> {
    glyph_count: u16,
    axis_count: u16,
    shared_tuple_count: u16,
    shared_tuples: Cursor<'a>,
    offset_format: OffsetFormat,
    glyph_variation_data_offsets: Cursor<'a>,
    glyph_variation_data: Cursor<'a>,
}

impl<'a> GvarTable<'a> {
    const VERSION: u32 = 0x10000;

    pub(super) fn parse(mut cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let full_cursor = cursor;

        cursor.read_u32_checked(|version| check_exact!(version, Self::VERSION))?;
        let axis_count = cursor.read_u16()?;
        let shared_tuple_count = cursor.read_u16()?;
        let shared_tuples_offset = usize::try_from(cursor.read_u32()?).unwrap();
        let shared_tuples_len =
            2 /* size_of(F2DO14) */ * usize::from(axis_count) * usize::from(shared_tuple_count);
        let shared_tuples =
            full_cursor.range(shared_tuples_offset..shared_tuples_offset + shared_tuples_len)?;

        let glyph_count = cursor.read_u16()?;
        let flags = cursor.read_u16()?;
        let offset_format = if flags & 1 == 0 {
            OffsetFormat::Short
        } else {
            OffsetFormat::Long
        };
        let glyph_variation_data_array_offset = cursor.read_u32()?;
        let mut glyph_variation_data = full_cursor;
        glyph_variation_data.skip(glyph_variation_data_array_offset.try_into().unwrap())?;
        let len = offset_format.bytes_per_offset() * (usize::from(glyph_count) + 1);
        let glyph_variation_data_offsets = cursor.range(0..len)?;

        Ok(Self {
            glyph_count,
            axis_count,
            shared_tuple_count,
            shared_tuples,
            offset_format,
            glyph_variation_data_offsets,
            glyph_variation_data,
        })
    }

    fn resolve_offset(&self, glyph_idx: u16) -> Result<usize, ParseError> {
        let offset_in_data_offsets = usize::from(glyph_idx) * self.offset_format.bytes_per_offset();
        let mut cursor = self.glyph_variation_data_offsets;
        cursor.skip(offset_in_data_offsets)?;
        Ok(match self.offset_format {
            OffsetFormat::Short => usize::from(cursor.read_u16()?) * 2,
            OffsetFormat::Long => usize::try_from(cursor.read_u32()?).unwrap(),
        })
    }

    /// Returns `Ok(None)` for empty variation data.
    pub(super) fn variation_data(
        &self,
        glyph_idx: u16,
    ) -> Result<Option<GlyphVariationData<'a>>, ParseError> {
        let start = self.resolve_offset(glyph_idx)?;
        let end = self.resolve_offset(glyph_idx + 1)?;
        let range = start..end;
        if range.is_empty() {
            Ok(None)
        } else {
            let raw = self.glyph_variation_data.range(range)?;
            GlyphVariationData::parse(raw, self.axis_count, self.shared_tuple_count).map(Some)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::{testonly::TestFont, OpenTypeReader, TableTag};

    #[test]
    fn parsing_gvar_table() {
        let reader = OpenTypeReader::new(TestFont::ROBOTO.bytes).unwrap();
        let table_cursor = reader
            .iter()
            .find_map(|(tag, cursor)| (tag == TableTag(*b"gvar")).then_some(cursor))
            .unwrap();
        let table = GvarTable::parse(table_cursor).unwrap();
        let mut referenced_shared_tuples = HashSet::new();
        for glyph_id in 0..table.glyph_count {
            if let Some(data) = table.variation_data(glyph_id).unwrap() {
                assert!(!data.all_bytes.is_empty());
                referenced_shared_tuples.extend(data.tuple_refs.iter().map(|(_, idx)| *idx));
            }
        }
        assert_eq!(
            referenced_shared_tuples,
            HashSet::from_iter(0..table.shared_tuple_count)
        );
    }
}
