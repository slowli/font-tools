//! Logic for serializing `FontSubset`s in OpenType format.

use core::{iter, mem};

use crate::{
    font::{
        CmapTable, Glyph, GlyphComponent, GlyphComponentArgs, GlyphWithMetrics, HheaTable,
        HmtxTable, LocaFormat, LocaTable, SegmentDeltas, SegmentWithDelta, SegmentedCoverage,
        SequentialMapGroup, TransformData,
    },
    Font, FontSubset,
};

fn write_u16(writer: &mut Vec<u8>, value: u16) {
    writer.extend_from_slice(&value.to_be_bytes())
}

fn write_u32(writer: &mut Vec<u8>, value: u32) {
    writer.extend_from_slice(&value.to_be_bytes())
}

impl CmapTable<'static> {
    fn from_map(map: &[(char, u16)]) -> Self {
        let coverage = Self::create_coverage(map);
        let can_be_encoded_as_deltas = map
            .last()
            .is_none_or(|&(ch, _)| u32::from(ch) < u32::from(u16::MAX));
        if can_be_encoded_as_deltas {
            let delta_segments = coverage.groups.iter().map(|group| {
                // `_ as u16` is safe due to the `can_be_encoded_as_deltas` check
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
        write_u16(writer, self.subtable_len() as u16);
        write_u16(writer, 0); // language

        let segment_count = self.segments.len() as u16;
        write_u16(writer, 2 * segment_count);
        let entry_selector = segment_count.ilog2() as u16;
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

        write_u32(writer, self.subtable_len() as u32);
        write_u32(writer, 0); // language
        write_u32(writer, self.groups.len() as u32);
        for group in &self.groups {
            write_u32(writer, group.start_char_code);
            write_u32(writer, group.end_char_code);
            write_u32(writer, group.start_glyph_id);
        }
    }
}

/// Offset of the checksum in the `head` table.
const HEAD_CHECKSUM_OFFSET: usize = 8;

impl FontSubset<'_> {
    pub fn write(&self) -> FontBuilder {
        let cmap = CmapTable::from_map(&self.char_map);

        let mut builder = FontBuilder::default();
        builder.write_table(Font::CMAP_TAG, |writer| cmap.write(writer));
        if let Some(cvt) = self.font.cvt {
            builder.write_raw_table(Font::CVT_TAG, cvt.as_ref());
        }
        if let Some(fpgm) = self.font.fpgm {
            builder.write_raw_table(Font::FPGM_TAG, fpgm.as_ref());
        }

        let number_of_h_metrics = builder.write_table(Font::HMTX_TAG, |writer| {
            HmtxTable::write_for_glyphs(&self.glyphs, writer)
        });
        let mut hhea = self.font.hhea;
        hhea.number_of_h_metrics = number_of_h_metrics;
        builder.write_table(Font::HHEA_TAG, |writer| {
            hhea.write(writer);
        });

        let maxp = self.font.maxp.as_ref();
        builder.write_table(Font::MAXP_TAG, |writer| {
            // Patch the number of glyphs (u16 at bytes 4..6), and leave other bytes intact.
            writer.extend_from_slice(&maxp[..4]);
            write_u16(writer, self.glyphs.len() as u16);
            writer.extend_from_slice(&maxp[6..]);
        });

        // TODO: reduce `name` table?
        builder.write_raw_table(Font::NAME_TAG, self.font.name.as_ref());
        builder.write_raw_table(Font::OS2_TAG, self.font.os2.as_ref());

        let post = self.font.post.as_ref();
        builder.write_table(Font::POST_TAG, |writer| {
            // Truncate the `post` table to not contain glyph names
            write_u32(writer, 0x_00030000); // version
            writer.extend_from_slice(&post[4..32]);
        });

        if let Some(prep) = self.font.prep {
            builder.write_raw_table(Font::PREP_TAG, prep.as_ref());
        }

        let locations = builder.write_table(Font::GLYF_TAG, |writer| {
            let mut locations = vec![0];
            let initial_offset = writer.len();
            for glyph in &self.glyphs {
                let glyph = &glyph.inner;
                glyph.write(writer);
                locations.push(writer.len() - initial_offset);
            }
            locations
        });

        let loca_format = builder.write_table(Font::LOCA_TAG, |writer| {
            LocaTable::write(&locations, writer)
        });
        builder.write_table(Font::HEAD_TAG, |writer| {
            Self::write_head_table(self.font.head.as_ref(), loca_format, writer);
        });

        builder
    }

    fn write_head_table(original: &[u8], loca_format: LocaFormat, writer: &mut Vec<u8>) {
        const LOCA_FORMAT_OFFSET: usize = 50;

        writer.extend_from_slice(&original[..HEAD_CHECKSUM_OFFSET]);
        write_u32(writer, 0); // Zero the checksum as per spec. It will be adjusted later
        writer.extend_from_slice(&original[HEAD_CHECKSUM_OFFSET + 4..LOCA_FORMAT_OFFSET]);
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

        number_of_h_metrics as u16
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
                write_u16(writer, (loc / 2) as u16);
            }
            LocaFormat::Short
        } else {
            for &loc in locations {
                write_u32(writer, loc as u32);
            }
            LocaFormat::Long
        }
    }
}

#[derive(Debug)]
struct TableRecord {
    tag: [u8; 4],
    checksum: u32,
    offset: u32,
    length: u32,
}

impl TableRecord {
    const BYTE_LEN: usize = 16;

    fn write(&self, writer: &mut Vec<u8>) {
        writer.extend_from_slice(&self.tag);
        write_u32(writer, self.checksum);
        write_u32(writer, self.offset);
        write_u32(writer, self.length);
    }
}

#[derive(Debug, Default)]
pub struct FontBuilder {
    tables: Vec<TableRecord>,
    table_heap: Vec<u8>,
}

impl FontBuilder {
    fn write_table<T>(&mut self, tag: [u8; 4], with: impl FnOnce(&mut Vec<u8>) -> T) -> T {
        let offset = self.table_heap.len();
        debug_assert_eq!(offset % 4, 0, "unaligned offset: {offset}");

        let output = with(&mut self.table_heap);
        let length = self.table_heap.len() - offset;
        // Pad the table heap to a 4-byte boundary.
        if length % 4 > 0 {
            let zero_padding = 4 - length % 4;
            self.table_heap.extend(iter::repeat_n(0_u8, zero_padding));
        }

        let checksum = Font::checksum(&self.table_heap[offset..]);
        self.tables.push(TableRecord {
            tag,
            checksum,
            offset: offset as u32,
            length: length as u32,
        });
        output
    }

    fn write_raw_table(&mut self, tag: [u8; 4], content: &[u8]) {
        self.write_table(tag, |buffer| buffer.extend_from_slice(content));
    }

    pub fn into_truetype(mut self) -> Vec<u8> {
        let mut buffer = vec![];
        write_u32(&mut buffer, Font::SNFT_VERSION);

        let table_count = self.tables.len() as u16;
        write_u16(&mut buffer, table_count);
        let entry_selector = table_count.ilog2() as u16;
        let search_range = 1 << (4 + entry_selector);
        write_u16(&mut buffer, search_range);
        write_u16(&mut buffer, entry_selector);
        let range_shift = 16 * table_count - search_range;
        write_u16(&mut buffer, range_shift);

        let heap_offset = (buffer.len() + self.tables.len() * TableRecord::BYTE_LEN) as u32;
        self.tables.sort_unstable_by_key(|record| record.tag);
        for record in &mut self.tables {
            record.offset += heap_offset;
            record.write(&mut buffer);
        }

        buffer.extend(self.table_heap);

        // Adjust the checksum in the `head` table.
        let checksum = 0x_b1b0_afba_u32.wrapping_sub(Font::checksum(&buffer));
        let head_table = self
            .tables
            .iter()
            .find(|record| record.tag == Font::HEAD_TAG)
            .unwrap();
        let checksum_offset = head_table.offset as usize + HEAD_CHECKSUM_OFFSET;
        buffer[checksum_offset..checksum_offset + 4].copy_from_slice(&checksum.to_be_bytes());

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
