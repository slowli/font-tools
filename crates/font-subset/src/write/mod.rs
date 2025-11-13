//! Logic for serializing `FontSubset`s in OpenType format.

use core::{iter, mem};

use crate::{
    alloc::{vec, Vec},
    font::{
        CmapTable, Glyph, GlyphComponent, GlyphComponentArgs, GlyphWithMetrics, HheaTable,
        HmtxTable, LocaFormat, LocaTable, SegmentDeltas, SegmentWithDelta, SegmentedCoverage,
        SequentialMapGroup, TransformData,
    },
    Font, FontSubset, TableTag,
};

mod brotli;

fn write_u16(writer: &mut Vec<u8>, value: u16) {
    writer.extend_from_slice(&value.to_be_bytes());
}

fn write_u32(writer: &mut Vec<u8>, value: u32) {
    writer.extend_from_slice(&value.to_be_bytes());
}

fn uint_base128_len(val: u32) -> usize {
    if val == 0 {
        1
    } else {
        val.ilog2() as usize / 7 + 1
    }
}

#[allow(clippy::cast_possible_truncation)] // intentional
fn write_uint_base128(buffer: &mut Vec<u8>, val: u32) {
    //let mut prev_len = buffer.len();
    if val >= 1 << 28 {
        buffer.push(0x80 | (val >> 28) as u8);
    }
    if val >= 1 << 21 {
        buffer.push(0x80 | (val >> 21) as u8);
    }
    if val >= 1 << 14 {
        buffer.push(0x80 | (val >> 14) as u8);
    }
    if val >= 1 << 7 {
        buffer.push(0x80 | (val >> 7) as u8);
    }
    buffer.push((val & 127) as u8);
}

impl CmapTable<'static> {
    fn from_map(map: &[(char, u16)]) -> Self {
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

impl CmapTable<'_> {
    fn write(&self, writer: &mut Vec<u8>) {
        write_u16(writer, 0); // table version
        write_u16(writer, 1); // num_tables

        write_u16(writer, CmapTable::UNICODE_PLATFORM);
        let encoding_id = match self {
            Self::Deltas(_) => 3,
            Self::Coverage(_) => 4,
        };
        write_u16(writer, encoding_id);
        write_u32(writer, 12); // subtable_offset

        match self {
            Self::Deltas(deltas) => deltas.write(writer),
            Self::Coverage(coverage) => coverage.write(writer),
        }
    }
}

impl SegmentDeltas<'_> {
    fn subtable_len(&self) -> usize {
        16 + 8 * self.segments.len()
    }

    fn write(&self, writer: &mut Vec<u8>) {
        write_u16(writer, 4); // subtable format
        write_u16(
            writer,
            self.subtable_len()
                .try_into()
                .expect("subtable_len overflow"),
        );
        write_u16(writer, 0); // language

        let segment_count = u16::try_from(self.segments.len()).expect("segments.len() overflow");
        write_u16(writer, 2 * segment_count);
        let entry_selector = u16::try_from(segment_count.ilog2()).unwrap();
        let search_range = 1 << (entry_selector + 1);
        write_u16(writer, search_range);
        write_u16(writer, entry_selector);
        let range_shift = 2 * segment_count - search_range;
        write_u16(writer, range_shift);

        for segment in &self.segments {
            write_u16(writer, segment.end_code);
        }
        write_u16(writer, 0); // reserved padding
        for segment in &self.segments {
            write_u16(writer, segment.start_code);
        }
        for segment in &self.segments {
            write_u16(writer, segment.id_delta);
        }
        for segment in &self.segments {
            write_u16(writer, segment.id_range_offset);
        }
        writer.extend_from_slice(self.glyph_id_array);
    }
}

impl SegmentedCoverage {
    fn subtable_len(&self) -> usize {
        16 + 12 * self.groups.len()
    }

    fn write(&self, writer: &mut Vec<u8>) {
        write_u16(writer, 12); // subtable format
        write_u16(writer, 0); // reserved

        write_u32(
            writer,
            self.subtable_len()
                .try_into()
                .expect("subtable_len overflow"),
        );
        write_u32(writer, 0); // language
        write_u32(
            writer,
            self.groups.len().try_into().expect("groups.len() overflow"),
        );
        for group in &self.groups {
            write_u32(writer, group.start_char_code);
            write_u32(writer, group.end_char_code);
            write_u32(writer, group.start_glyph_id);
        }
    }
}

impl FontSubset<'_> {
    /// Serializes this subset to the OpenType format.
    pub fn to_truetype(&self) -> Vec<u8> {
        self.to_writer().into_opentype()
    }

    /// Serializes this subset to the WOFF2 format.
    pub fn to_woff2(&self) -> Vec<u8> {
        self.to_writer().into_woff2()
    }

    fn to_writer(&self) -> FontWriter {
        let cmap = CmapTable::from_map(&self.char_map);

        let mut writer = FontWriter::default();
        writer.write_table(TableTag::CMAP, |buffer| cmap.write(buffer));
        if let Some(cvt) = self.font.cvt {
            writer.write_raw_table(TableTag::CVT, cvt.as_ref());
        }
        if let Some(fpgm) = self.font.fpgm {
            writer.write_raw_table(TableTag::FPGM, fpgm.as_ref());
        }

        let number_of_h_metrics = writer.write_table(TableTag::HMTX, |buffer| {
            HmtxTable::write_for_glyphs(&self.glyphs, buffer)
        });
        let mut hhea = self.font.hhea;
        hhea.number_of_h_metrics = number_of_h_metrics;
        writer.write_table(TableTag::HHEA, |buffer| {
            hhea.write(buffer);
        });

        let maxp = self.font.maxp.as_ref();
        writer.write_table(TableTag::MAXP, |buffer| {
            // Patch the number of glyphs (u16 at bytes 4..6), and leave other bytes intact.
            buffer.extend_from_slice(&maxp[..4]);
            // `unwrap()` should be safe: the subset shouldn't contain >65536 glyphs because the original font doesn't.
            write_u16(buffer, self.glyphs.len().try_into().unwrap());
            buffer.extend_from_slice(&maxp[6..]);
        });

        // TODO: reduce `name` table?
        writer.write_raw_table(TableTag::NAME, self.font.name.as_ref());
        writer.write_raw_table(TableTag::OS2, self.font.os2.as_ref());

        let post = self.font.post.as_ref();
        writer.write_table(TableTag::POST, |buffer| {
            // Truncate the `post` table to not contain glyph names
            write_u32(buffer, 0x_00030000); // version
            buffer.extend_from_slice(&post[4..32]);
        });

        if let Some(prep) = self.font.prep {
            writer.write_raw_table(TableTag::PREP, prep.as_ref());
        }

        let locations = writer.write_table(TableTag::GLYF, |buffer| {
            let mut locations = vec![0];
            let initial_offset = buffer.len();
            for glyph in &self.glyphs {
                let glyph = &glyph.inner;
                glyph.write(buffer);
                locations.push(buffer.len() - initial_offset);
            }
            locations
        });

        let loca_format = writer.write_table(TableTag::LOCA, |buffer| {
            LocaTable::write(&locations, buffer)
        });
        writer.write_table(TableTag::HEAD, |buffer| {
            Self::write_head_table(self.font.head.as_ref(), loca_format, buffer);
        });

        writer
    }

    fn write_head_table(original: &[u8], loca_format: LocaFormat, writer: &mut Vec<u8>) {
        const LOCA_FORMAT_OFFSET: usize = 50;

        writer.extend_from_slice(&original[..Font::HEAD_CHECKSUM_OFFSET]);
        write_u32(writer, 0); // Zero the checksum as per spec. It will be adjusted later
        writer.extend_from_slice(&original[Font::HEAD_CHECKSUM_OFFSET + 4..LOCA_FORMAT_OFFSET]);
        write_u16(
            writer,
            match loca_format {
                LocaFormat::Short => 0,
                LocaFormat::Long => 1,
            },
        );
        writer.extend_from_slice(&original[LOCA_FORMAT_OFFSET + 2..]);
    }
}

impl HmtxTable<'_> {
    fn write_for_glyphs(glyphs: &[GlyphWithMetrics<'_>], writer: &mut Vec<u8>) -> u16 {
        let mut number_of_h_metrics = glyphs.len();
        while let Some([prev, current]) = glyphs[..number_of_h_metrics].last_chunk::<2>() {
            if prev.advance != current.advance {
                break;
            }
            number_of_h_metrics -= 1;
        }

        for (i, glyph) in glyphs.iter().enumerate() {
            if i < number_of_h_metrics {
                write_u16(writer, glyph.advance);
                write_u16(writer, glyph.lsb);
            } else {
                write_u16(writer, glyph.lsb);
            }
        }

        // `unwrap()` should be safe: `number_of_h_metrics` <= number of glyphs, which doesn't exceed u16::MAX
        number_of_h_metrics.try_into().unwrap()
    }
}

impl HheaTable<'_> {
    fn write(&self, writer: &mut Vec<u8>) {
        writer.extend_from_slice(&self.raw[..Self::EXPECTED_LEN - 2]);
        write_u16(writer, self.number_of_h_metrics);
    }
}

impl LocaTable<'_> {
    fn write(locations: &[usize], writer: &mut Vec<u8>) -> LocaFormat {
        let all_even = locations.iter().all(|&loc| loc % 2 == 0);
        let in_bounds = locations
            .last()
            .is_none_or(|&loc| loc <= usize::from(u16::MAX) * 2);
        if all_even && in_bounds {
            for &loc in locations {
                #[allow(clippy::cast_possible_truncation)]
                // doesn't happen due to the preceding check
                write_u16(writer, (loc / 2) as u16);
            }
            LocaFormat::Short
        } else {
            for &loc in locations {
                write_u32(writer, u32::try_from(loc).expect("glyph location overflow"));
            }
            LocaFormat::Long
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
struct TableRecord {
    tag: TableTag,
    checksum: u32,
    /// Offset is initially recorded relative to the table data start. It's always 4-byte aligned.
    offset: u32,
    length: u32,
}

impl TableRecord {
    const BYTE_LEN: usize = 16;

    fn write_opentype(&self, writer: &mut Vec<u8>) {
        writer.extend_from_slice(&self.tag.0);
        write_u32(writer, self.checksum);
        write_u32(writer, self.offset);
        write_u32(writer, self.length);
    }

    fn self_checksum(&self) -> u32 {
        u32::from_be_bytes(self.tag.0)
            .wrapping_add(self.checksum)
            .wrapping_add(self.offset)
            .wrapping_add(self.length)
    }

    fn woff2_len(&self) -> usize {
        1 /* flags */ + uint_base128_len(self.length)
    }

    fn write_woff2(&self, buffer: &mut Vec<u8>) {
        const NULL_TRANSFORM: u8 = 0b_1100_0000;

        let flags = match self.tag {
            TableTag::CMAP => 0,
            TableTag::HEAD => 1,
            TableTag::HHEA => 2,
            TableTag::HMTX => 3,
            TableTag::MAXP => 4,
            TableTag::NAME => 5,
            TableTag::OS2 => 6,
            TableTag::POST => 7,
            TableTag::CVT => 8,
            TableTag::FPGM => 9,
            TableTag::GLYF => 10 | NULL_TRANSFORM,
            TableTag::LOCA => 11 | NULL_TRANSFORM,
            TableTag::PREP => 12,
            _ => unreachable!("subsetting only produces well-known tables"),
        };
        buffer.push(flags);
        write_uint_base128(buffer, self.length);
    }
}

#[derive(Debug, Clone, Default)]
struct FontWriter {
    tables: Vec<TableRecord>,
    /// Contains *aligned* table data
    table_data: Vec<u8>,
}

impl FontWriter {
    const SFNT_HEADER_LEN: usize = 12;
    const WOFF2_HEADER_LEN: usize = 48;

    fn write_table<T>(&mut self, tag: TableTag, with: impl FnOnce(&mut Vec<u8>) -> T) -> T {
        let offset = self.table_data.len();
        debug_assert_eq!(offset % 4, 0, "unaligned offset: {offset}");

        let output = with(&mut self.table_data);
        let length = self.table_data.len() - offset;
        // Pad the table heap to a 4-byte boundary.
        if length % 4 > 0 {
            let zero_padding = 4 - length % 4;
            self.table_data.extend(iter::repeat_n(0_u8, zero_padding));
        }

        let checksum = Font::checksum(&self.table_data[offset..]);
        self.tables.push(TableRecord {
            tag,
            checksum,
            offset: u32::try_from(offset).expect("table offset overflow"),
            length: u32::try_from(length).expect("table length overflow"),
        });
        output
    }

    fn write_raw_table(&mut self, tag: TableTag, content: &[u8]) {
        self.write_table(tag, |buffer| buffer.extend_from_slice(content));
    }

    fn write_sfnt_header(&self) -> Vec<u8> {
        let mut buffer = vec![];
        write_u32(&mut buffer, Font::SFNT_VERSION);

        // `unwrap()`s are safe: we don't have many tables written.
        let table_count = u16::try_from(self.tables.len()).unwrap();
        write_u16(&mut buffer, table_count);
        let entry_selector = u16::try_from(table_count.ilog2()).unwrap();
        let search_range = 1 << (4 + entry_selector);
        write_u16(&mut buffer, search_range);
        write_u16(&mut buffer, entry_selector);
        let range_shift = 16 * table_count - search_range;
        write_u16(&mut buffer, range_shift);

        debug_assert_eq!(buffer.len(), Self::SFNT_HEADER_LEN);
        buffer
    }

    /// Returns the starting offset of table data.
    fn data_offset(&self) -> usize {
        Self::SFNT_HEADER_LEN + self.tables.len() * TableRecord::BYTE_LEN
    }

    fn into_opentype(mut self) -> Vec<u8> {
        let mut buffer = self.write_sfnt_header();
        self.adjust_data(Font::checksum(&buffer));

        self.tables.sort_unstable_by_key(|record| record.tag.0);
        for record in &self.tables {
            record.write_opentype(&mut buffer);
        }
        buffer.extend(self.table_data);
        buffer
    }

    fn adjust_data(&mut self, sfnt_header_checksum: u32) {
        let data_offset = self.data_offset();
        let data_offset_u32 = u32::try_from(data_offset).expect("data_offset overflow");

        let mut file_checksum = sfnt_header_checksum;
        for record in &mut self.tables {
            record.offset += data_offset_u32;
            file_checksum = file_checksum
                .wrapping_add(record.self_checksum())
                .wrapping_add(record.checksum);
        }
        self.patch_head_table(file_checksum, data_offset);
    }

    fn checksum_adjustment_offset(&self) -> usize {
        let head_table = self
            .tables
            .iter()
            .find(|record| record.tag == TableTag::HEAD)
            .expect("head table is always present");
        head_table.offset as usize + Font::HEAD_CHECKSUM_OFFSET
    }

    fn patch_head_table(&mut self, file_checksum: u32, data_offset: usize) {
        let checksum_adjustment = Font::SFNT_CHECKSUM.wrapping_sub(file_checksum);

        // At this point, the table offset already includes the heap offset, so we need to subtract it.
        let offset = self.checksum_adjustment_offset() - data_offset;
        self.table_data[offset..offset + 4].copy_from_slice(&checksum_adjustment.to_be_bytes());
    }

    fn into_woff2(mut self) -> Vec<u8> {
        const WOFF2_SIGNATURE: u32 = 0x_774f_4632;

        self.adjust_data(Font::checksum(&self.write_sfnt_header()));

        let compressed_data = self.compress_data();
        let tables_len = self
            .tables
            .iter()
            .map(TableRecord::woff2_len)
            .sum::<usize>();
        let mut file_len = Self::WOFF2_HEADER_LEN + tables_len + compressed_data.len();
        if file_len % 4 != 0 {
            file_len += 4 - file_len % 4;
        }

        let mut buffer = vec![];
        write_u32(&mut buffer, WOFF2_SIGNATURE);
        write_u32(&mut buffer, Font::SFNT_VERSION);
        write_u32(
            &mut buffer,
            file_len.try_into().expect("file length overflow"),
        );
        // `unwrap()` is safe: we don't write many tables
        write_u16(&mut buffer, self.tables.len().try_into().unwrap());
        write_u16(&mut buffer, 0); // reserved

        let decompressed_len = self.data_offset() + self.table_data.len();
        // `unwrap`s are safe, since `file_len` fits into u32.
        write_u32(&mut buffer, decompressed_len.try_into().unwrap());
        write_u32(&mut buffer, compressed_data.len().try_into().unwrap());
        write_u32(&mut buffer, 0); // WOFF version
        write_u32(&mut buffer, 0); // metadata offset
        write_u32(&mut buffer, 0); // metadata length
        write_u32(&mut buffer, 0); // original metadata length
        write_u32(&mut buffer, 0); // private block offset
        write_u32(&mut buffer, 0); // private block length
        debug_assert_eq!(buffer.len(), Self::WOFF2_HEADER_LEN);

        for record in &self.tables {
            record.write_woff2(&mut buffer);
        }
        debug_assert_eq!(buffer.len(), Self::WOFF2_HEADER_LEN + tables_len);
        buffer.extend(compressed_data);

        // Pad `buffer` to be 4-byte aligned. This is required even though we don't have metadata or private blocks.
        if buffer.len() % 4 != 0 {
            let padding = 4 - buffer.len() % 4;
            buffer.extend(iter::repeat_n(0, padding));
        }
        //debug_assert_eq!(file_len, buffer.len());
        buffer
    }
}

impl Glyph<'_> {
    fn write(&self, writer: &mut Vec<u8>) {
        match self {
            Self::Empty => { /* do nothing */ }
            Self::Simple(bytes) => {
                writer.extend_from_slice(bytes);
            }
            Self::Composite {
                header,
                components,
                instructions,
            } => {
                write_u16(writer, u16::MAX); // numberOfContours = -1
                writer.extend_from_slice(header);
                for component in components {
                    component.write(writer);
                }
                writer.extend_from_slice(instructions);
            }
        }
    }
}

impl GlyphComponent {
    fn write(&self, writer: &mut Vec<u8>) {
        write_u16(writer, self.flags);
        write_u16(writer, self.glyph_idx);
        match self.args {
            GlyphComponentArgs::U16(args) => write_u16(writer, args),
            GlyphComponentArgs::U32(args) => write_u32(writer, args),
        }
        match self.transform {
            TransformData::None => { /* do nothing */ }
            TransformData::Scale(val) => write_u16(writer, val),
            TransformData::TwoScales([x, y]) => {
                write_u16(writer, x);
                write_u16(writer, y);
            }
            TransformData::Affine([xx, xy, yx, yy]) => {
                write_u16(writer, xx);
                write_u16(writer, xy);
                write_u16(writer, yx);
                write_u16(writer, yy);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use allsorts::{binary::read::ReadScope, font_data::FontData, tables::FontTableProvider};
    use test_casing::{test_casing, Product};

    use super::*;
    use crate::tests::{TestCharSubset, TestFont, FONTS, SUBSET_CHARS};

    #[test]
    fn leb128_encoding() {
        let samples = &[
            (0_u32, &[0_u8] as &[u8]),
            (1, &[1]),
            (127, &[127]),
            (128, &[0x81, 0]),
            (129, &[0x81, 1]),
            (16_383, &[0xff, 0x7f]),
            (16_384, &[0x81, 0x80, 0]),
        ];
        for &(val, expected) in samples {
            assert_eq!(uint_base128_len(val), expected.len());
            let mut buffer = vec![];
            write_uint_base128(&mut buffer, val);
            assert_eq!(buffer, expected);
        }
    }

    #[test_casing(10, Product((FONTS, SUBSET_CHARS)))]
    #[test]
    fn woff2_tables_are_written_correctly(font: TestFont, chars: TestCharSubset) {
        let font = Font::new(font.bytes).unwrap();
        let writer = FontSubset::new(font, &chars.into_set())
            .unwrap()
            .to_writer();
        let FontWriter {
            tables, table_data, ..
        } = writer.clone();
        let woff2 = writer.into_woff2();

        let font_file = ReadScope::new(&woff2).read::<FontData>().unwrap();
        let font_provider = font_file.table_provider(0).unwrap();
        for record in &tables {
            println!("Testing table: {:?}", record.tag);
            let mut table_contents = font_provider
                .read_table_data(u32::from_be_bytes(record.tag.0))
                .unwrap();
            let start = record.offset as usize;
            let end = start + record.length as usize;

            if record.tag == TableTag::HEAD {
                let mut patched = table_contents.into_owned();
                patched[Font::HEAD_CHECKSUM_OFFSET..Font::HEAD_CHECKSUM_OFFSET + 4]
                    .copy_from_slice(&[0; 4]);
                table_contents = Cow::Owned(patched);
            }
            assert_eq!(table_contents.as_ref(), &table_data[start..end]);
        }

        allsorts::Font::new(font_provider).unwrap();
    }
}
