//! `gvar` table.

use crate::{
    alloc::{format, BTreeMap, BTreeSet, Vec},
    font::{Cursor, OffsetFormat},
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag,
};

#[derive(Debug, Clone)]
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
                // Subtract 2 to get to the start of the `tuple_index`
                tuple_refs.push((current_offset - 2, tuple_index));
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

    fn write(&self, buffer: &mut Vec<u8>) {
        let prev_len = buffer.len();
        buffer.extend_from_slice(self.all_bytes);

        // Patch shared tuple indices
        let data_bytes = &mut buffer[prev_len..];
        for &(offset, idx) in &self.tuple_refs {
            let [hi, lo] = idx.to_be_bytes();
            // Copy the lower 4 bits of `hi` and leave the higher 4 bits intact.
            data_bytes[offset] &= 0b_1111_0000 | hi;
            data_bytes[offset + 1] = lo;
        }
    }
}

#[derive(Debug, Clone)]
enum SharedTuples<'a> {
    Parsed { count: u16, cursor: Cursor<'a> },
    Subset(Vec<&'a [u8]>),
}

impl<'a> SharedTuples<'a> {
    fn len(&self) -> u16 {
        match self {
            Self::Parsed { count, .. } => *count,
            Self::Subset(tuples) => tuples.len().try_into().unwrap(),
        }
    }

    fn get(&self, idx: u16, axis_count: u16) -> Result<&'a [u8], ParseError> {
        let idx = usize::from(idx);
        match self {
            Self::Parsed { cursor, .. } => {
                let tuple_len = usize::from(axis_count) * 2;
                let start = idx * tuple_len;
                let end = start + tuple_len;
                Ok(cursor.read_range(start..end)?.bytes())
            }
            Self::Subset(slices) => Ok(slices[idx]),
        }
    }
}

#[derive(Debug, Clone)]
enum GlyphVariationDataVec<'a> {
    Parsed {
        glyph_count: u16,
        offset_format: OffsetFormat,
        offsets: Cursor<'a>,
        data: Cursor<'a>,
    },
    Subset(Vec<Option<GlyphVariationData<'a>>>),
}

impl<'a> GlyphVariationDataVec<'a> {
    fn glyph_count(&self) -> u16 {
        match self {
            Self::Parsed { glyph_count, .. } => *glyph_count,
            Self::Subset(data) => data.len().try_into().unwrap(),
        }
    }

    fn resolve_offset(
        glyph_idx: u16,
        offset_format: OffsetFormat,
        offsets: Cursor<'a>,
    ) -> Result<usize, ParseError> {
        let offset_in_data_offsets = usize::from(glyph_idx) * offset_format.bytes_per_offset();
        let mut cursor = offsets;
        cursor.skip(offset_in_data_offsets)?;
        Ok(match offset_format {
            OffsetFormat::Short => usize::from(cursor.read_u16()?) * 2,
            OffsetFormat::Long => usize::try_from(cursor.read_u32()?).unwrap(),
        })
    }

    fn get(
        &self,
        glyph_idx: u16,
        axis_count: u16,
        shared_tuple_count: u16,
    ) -> Result<Option<GlyphVariationData<'a>>, ParseError> {
        match self {
            &Self::Parsed {
                offset_format,
                offsets,
                data,
                ..
            } => {
                let start = Self::resolve_offset(glyph_idx, offset_format, offsets)?;
                let end = Self::resolve_offset(glyph_idx + 1, offset_format, offsets)?;
                let range = start..end;
                if range.is_empty() {
                    Ok(None)
                } else {
                    let raw = data.read_range(range)?;
                    GlyphVariationData::parse(raw, axis_count, shared_tuple_count).map(Some)
                }
            }
            Self::Subset(data) => Ok(data[usize::from(glyph_idx)].clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GvarTable<'a> {
    axis_count: u16,
    shared_tuples: SharedTuples<'a>,
    variation_data: GlyphVariationDataVec<'a>,
}

impl<'a> GvarTable<'a> {
    const VERSION: u32 = 0x0001_0000;

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip_all, fields(range = ?cursor.range()))
    )]
    pub(super) fn parse(mut cursor: Cursor<'a>, glyph_count: u16) -> Result<Self, ParseError> {
        let full_cursor = cursor;

        cursor.read_u32_checked(|version| check_exact!(version, Self::VERSION))?;
        let axis_count = cursor.read_u16()?;
        let shared_tuple_count = cursor.read_u16()?;
        let shared_tuples_offset = usize::try_from(cursor.read_u32()?).unwrap();
        let shared_tuples_len =
            2 /* size_of(F2DO14) */ * usize::from(axis_count) * usize::from(shared_tuple_count);
        let shared_tuples = full_cursor
            .read_range(shared_tuples_offset..shared_tuples_offset + shared_tuples_len)?;

        cursor.read_u16_checked(|count| check_exact!(count, glyph_count))?;
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
        let glyph_variation_data_offsets = cursor.read_range(0..len)?;

        #[cfg(feature = "tracing")]
        tracing::debug!(
            axis_count,
            shared_tuple_count,
            shared_tuples = ?shared_tuples.range(),
            ?offset_format,
            glyph_variation_data_offsets = ?glyph_variation_data_offsets.range(),
            glyph_variation_data = ?glyph_variation_data.range(),
            "parsed basic info"
        );

        Ok(Self {
            axis_count,
            shared_tuples: SharedTuples::Parsed {
                count: shared_tuple_count,
                cursor: shared_tuples,
            },
            variation_data: GlyphVariationDataVec::Parsed {
                glyph_count,
                offset_format,
                offsets: glyph_variation_data_offsets,
                data: glyph_variation_data,
            },
        })
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip_all)
    )]
    pub(crate) fn subset(&self, glyph_ids: impl Iterator<Item = u16>) -> Result<Self, ParseError> {
        let mut referenced_shared_tuples = BTreeSet::new();
        let shared_tuples_count = self.shared_tuples.len();

        let mut variation_data = glyph_ids
            .map(|glyph_idx| {
                let data =
                    self.variation_data
                        .get(glyph_idx, self.axis_count, shared_tuples_count)?;
                if let Some(data) = &data {
                    referenced_shared_tuples.extend(data.tuple_refs.iter().map(|(_, idx)| *idx));
                }
                Ok(data)
            })
            .collect::<Result<Vec<_>, _>>()?;
        #[cfg(feature = "tracing")]
        tracing::debug!(
            variation_data.len = variation_data.len(),
            "extracted variation data subset"
        );

        let shared_tuples = referenced_shared_tuples
            .iter()
            .map(|&idx| self.shared_tuples.get(idx, self.axis_count))
            .collect::<Result<_, _>>()?;
        let shared_tuple_mapping: BTreeMap<_, _> = referenced_shared_tuples
            .iter()
            .enumerate()
            .map(|(new_idx, &old_idx)| (old_idx, u16::try_from(new_idx).unwrap()))
            .collect();
        #[cfg(feature = "tracing")]
        tracing::debug!(?shared_tuple_mapping, "mapped shared tuple indices");

        // Update shared tuple indices in the glyph data.
        for glyph_data in variation_data.iter_mut().flatten() {
            for (_, idx) in &mut glyph_data.tuple_refs {
                *idx = shared_tuple_mapping[idx];
            }
        }

        Ok(Self {
            axis_count: self.axis_count,
            shared_tuples: SharedTuples::Subset(shared_tuples),
            variation_data: GlyphVariationDataVec::Subset(variation_data),
        })
    }
}

impl WriteTable for GvarTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::GVAR
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        const FIXED_HEADER_LEN: usize = 20;

        let glyph_count = self.variation_data.glyph_count();
        let offset_format = match &self.variation_data {
            GlyphVariationDataVec::Parsed { offset_format, .. } => *offset_format,
            GlyphVariationDataVec::Subset(data) => {
                let mut total_byte_len = 0;
                let all_even = data.iter().all(|data| {
                    data.as_ref().is_none_or(|data| {
                        let byte_len = data.all_bytes.len();
                        total_byte_len += byte_len;
                        byte_len % 2 == 0
                    })
                });
                if all_even && total_byte_len / 2 < usize::from(u16::MAX) {
                    OffsetFormat::Short
                } else {
                    OffsetFormat::Long
                }
            }
        };
        let data_offsets_len = offset_format.bytes_per_offset() * (usize::from(glyph_count) + 1);
        let shared_tuples_offset = FIXED_HEADER_LEN + data_offsets_len;
        let shared_tuples_len = 2 /* size_of(F2DO14) */ * usize::from(self.axis_count) * usize::from(self.shared_tuples.len());
        let glyph_variation_data_array_offset = shared_tuples_offset + shared_tuples_len;

        let start_offset = buffer.len();
        buffer.write_u32(Self::VERSION);
        buffer.write_u16(self.axis_count);
        buffer.write_u16(self.shared_tuples.len());
        buffer.write_u32(shared_tuples_offset.try_into().expect("offset overflow"));
        buffer.write_u16(glyph_count);
        buffer.write_u16(match offset_format {
            OffsetFormat::Short => 0,
            OffsetFormat::Long => 1,
        });
        buffer.write_u32(
            glyph_variation_data_array_offset
                .try_into()
                .expect("offset overflow"),
        );
        debug_assert_eq!(buffer.len() - start_offset, FIXED_HEADER_LEN);

        // Write offsets
        match &self.variation_data {
            GlyphVariationDataVec::Parsed { offsets, .. } => {
                buffer.extend_from_slice(offsets.bytes());
            }
            GlyphVariationDataVec::Subset(data) => {
                #[allow(clippy::cast_possible_truncation)] // checked when choosing `offset_format`
                let mut write_offset = |offset: usize| match offset_format {
                    OffsetFormat::Short => buffer.write_u16((offset / 2) as u16),
                    OffsetFormat::Long => buffer.write_u32(offset as u32),
                };
                write_offset(0);

                let mut total_len = 0;
                for glyph_data in data {
                    let len = glyph_data.as_ref().map_or(0, |data| data.all_bytes.len());
                    total_len += len;
                    write_offset(total_len);
                }
            }
        }

        // Write shared tuples
        debug_assert_eq!(buffer.len() - start_offset, shared_tuples_offset);
        match &self.shared_tuples {
            SharedTuples::Parsed { cursor, .. } => {
                buffer.extend_from_slice(cursor.bytes());
            }
            SharedTuples::Subset(slices) => {
                for &slice in slices {
                    buffer.extend_from_slice(slice);
                }
            }
        }

        // Write glyph data
        debug_assert_eq!(
            buffer.len() - start_offset,
            glyph_variation_data_array_offset
        );
        match &self.variation_data {
            GlyphVariationDataVec::Parsed { data, .. } => {
                buffer.extend_from_slice(data.bytes());
            }
            GlyphVariationDataVec::Subset(data) => {
                for glyph_data in data.iter().flatten() {
                    glyph_data.write(buffer);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use test_casing::{test_casing, Product};

    use super::*;
    use crate::{font::MaxpTable, testonly::TestFont, OpenTypeReader, TableTag};

    impl PartialEq for GlyphVariationData<'_> {
        fn eq(&self, other: &Self) -> bool {
            if self.tuple_refs != other.tuple_refs || self.all_bytes.len() != other.all_bytes.len()
            {
                return false;
            }

            // Exclude byte ranges covered by `tuple_refs`.
            let mut current_offset = 0;
            for &(offset, _) in &self.tuple_refs {
                let checked_range = current_offset..offset;
                if self.all_bytes[checked_range.clone()] != other.all_bytes[checked_range] {
                    return false;
                }
                current_offset = offset + 2;
            }
            self.all_bytes[current_offset..] == other.all_bytes[current_offset..]
        }
    }

    fn test_gvar_table_cursor(test_font: TestFont) -> (Cursor<'static>, u16) {
        let reader = OpenTypeReader::new(test_font.bytes).unwrap();
        let maxp = reader
            .iter()
            .find_map(|(tag, cursor)| (tag == TableTag::MAXP).then_some(cursor))
            .unwrap();
        let glyph_count = MaxpTable::parse(maxp).unwrap().glyph_count;

        let gvar = reader
            .iter()
            .find_map(|(tag, cursor)| (tag == TableTag::GVAR).then_some(cursor))
            .unwrap();
        (gvar, glyph_count)
    }

    #[test_casing(3, TestFont::VAR)]
    fn parsing_gvar_table(font: TestFont) {
        let (table_cursor, glyph_count) = test_gvar_table_cursor(font);
        let table = GvarTable::parse(table_cursor, glyph_count).unwrap();
        let axis_count = table.axis_count;
        assert!(axis_count >= 1);
        let shared_tuples_count = table.shared_tuples.len();
        assert!(shared_tuples_count > 1);
        let glyph_count = table.variation_data.glyph_count();

        let mut referenced_shared_tuples = HashSet::new();
        for glyph_id in 0..glyph_count {
            if let Some(data) = table
                .variation_data
                .get(glyph_id, axis_count, shared_tuples_count)
                .unwrap()
            {
                assert!(!data.all_bytes.is_empty());
                for &(offset, idx) in &data.tuple_refs {
                    let read_idx_bytes = &data.all_bytes[offset..offset + 2];
                    let read_idx = u16::from_be_bytes(read_idx_bytes.try_into().unwrap());
                    assert_eq!(read_idx & 0x0fff, idx);
                }
                referenced_shared_tuples.extend(data.tuple_refs.iter().map(|(_, idx)| *idx));
            }
        }
        assert_eq!(
            referenced_shared_tuples,
            (0..shared_tuples_count).collect::<HashSet<_>>()
        );
    }

    #[test_casing(3, TestFont::VAR)]
    fn gvar_table_roundtrip(font: TestFont) {
        let (table_cursor, glyph_count) = test_gvar_table_cursor(font);
        let table = GvarTable::parse(table_cursor, glyph_count).unwrap();
        let mut buffer = vec![];
        table.write_to_vec(&mut buffer);
        assert_eq!(buffer, table_cursor.bytes());
    }

    #[test_casing(3, TestFont::VAR)]
    fn gvar_table_roundtrip_via_complete_subset(font: TestFont) {
        let (table_cursor, glyph_count) = test_gvar_table_cursor(font);
        let table = GvarTable::parse(table_cursor, glyph_count).unwrap();
        let glyph_count = table.variation_data.glyph_count();
        let table = table.subset(0..glyph_count).unwrap();

        let mut buffer = vec![];
        table.write_to_vec(&mut buffer);
        assert_eq!(buffer, table_cursor.bytes());
    }

    #[test]
    fn gvar_table_subsetting_with_zero_glyph() {
        let (table_cursor, glyph_count) = test_gvar_table_cursor(TestFont::ROBOTO);
        let table = GvarTable::parse(table_cursor, glyph_count).unwrap();
        let table = table.subset([0].into_iter()).unwrap();

        assert_eq!(table.axis_count, 2);
        let SharedTuples::Subset(shared_tuples) = &table.shared_tuples else {
            panic!("unexpected shared tuples: {table:?}")
        };
        assert_eq!(shared_tuples.len(), 3);

        let GlyphVariationDataVec::Subset(data) = &table.variation_data else {
            panic!("unexpected glyph data: {table:?}");
        };
        assert_eq!(data.len(), 1);
        let glyph_data = data[0].as_ref().unwrap();
        assert_eq!(glyph_data.tuple_refs, [(6, 2), (10, 0), (14, 1)]);

        let mut buffer = vec![];
        table.write_to_vec(&mut buffer);
        let parsed = GvarTable::parse(Cursor::new(&buffer), 1).unwrap();
        assert_eq!(parsed.axis_count, 2);
        assert_eq!(parsed.shared_tuples.len(), 3);
        assert_eq!(parsed.variation_data.glyph_count(), 1);
        let parsed_glyph_data = parsed
            .variation_data
            .get(0, 2, 3)
            .unwrap()
            .expect("no glyph data");
        assert_eq!(parsed_glyph_data, *glyph_data);
    }

    const GLYPH_IDS: [&[u16]; 4] = [&[3], &[0, 3], &[0, 1, 2, 3, 4, 5], &[0, 5, 10, 15]];

    #[test_casing(12, Product((TestFont::VAR, GLYPH_IDS)))]
    fn gvar_table_subsetting(font: TestFont, glyph_ids: &[u16]) {
        let (table_cursor, glyph_count) = test_gvar_table_cursor(font);
        let table = GvarTable::parse(table_cursor, glyph_count).unwrap();
        let table = table.subset(glyph_ids.iter().copied()).unwrap();
        let GlyphVariationDataVec::Subset(data) = &table.variation_data else {
            panic!("unexpected glyph data: {table:?}");
        };

        let mut buffer = vec![];
        table.write_to_vec(&mut buffer);
        let expected_glyph_count = u16::try_from(glyph_ids.len()).unwrap();
        let parsed = GvarTable::parse(Cursor::new(&buffer), expected_glyph_count).unwrap();

        assert_eq!(parsed.variation_data.glyph_count(), expected_glyph_count);
        let shared_tuple_count = parsed.shared_tuples.len();
        for glyph_id in 0..expected_glyph_count {
            println!("Testing glyph {glyph_id}");
            let glyph_data = data[usize::from(glyph_id)].as_ref();
            let parsed_glyph_data = parsed
                .variation_data
                .get(glyph_id, 2, shared_tuple_count)
                .unwrap();
            assert_eq!(parsed_glyph_data.as_ref(), glyph_data);
        }
    }
}
