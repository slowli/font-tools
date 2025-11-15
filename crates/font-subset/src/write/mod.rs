//! Logic for serializing `FontSubset`s in OpenType format.

use core::{iter, ops};

use crate::{
    alloc::{vec, Vec},
    font::{CmapTable, Cursor, HmtxTable, LocaTable, MaxpTable},
    Font, FontSubset, TableTag,
};

mod brotli;
#[cfg(test)]
mod tests;

/// Writes a single font table to a byte buffer.
pub(crate) trait WriteTable {
    fn tag(&self) -> TableTag;

    fn write_to_vec(&self, buffer: &mut Vec<u8>);
}

impl WriteTable for (TableTag, Cursor<'_>) {
    fn tag(&self) -> TableTag {
        self.0
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(self.1.bytes());
    }
}

/// Extension trait for `Vec<u8>` allowing to write various data to it.
pub(crate) trait VecExt {
    fn write_u16(&mut self, value: u16);

    fn write_i16(&mut self, value: i16);

    fn write_u32(&mut self, value: u32);

    fn write_u64(&mut self, value: u64);

    fn write_i64(&mut self, value: i64);
}

impl VecExt for Vec<u8> {
    fn write_u16(&mut self, value: u16) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn write_i16(&mut self, value: i16) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.extend_from_slice(&value.to_be_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.extend_from_slice(&value.to_be_bytes());
    }
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

impl FontSubset<'_> {
    /// Serializes this subset to the OpenType format.
    pub fn to_opentype(&self) -> Vec<u8> {
        self.to_writer().into_opentype()
    }

    /// Serializes this subset to the WOFF2 format.
    pub fn to_woff2(&self) -> Vec<u8> {
        self.to_writer().into_woff2()
    }

    fn char_range(&self) -> ops::RangeInclusive<char> {
        let &(first, _) = self.char_map.first().expect("empty subset");
        let &(last, _) = self.char_map.last().expect("empty subset");
        first..=last
    }

    fn to_writer(&self) -> FontWriter {
        let mut writer = FontWriter::default();

        let cmap = CmapTable::from_map(&self.char_map);
        writer.write(&cmap);
        if let Some(cvt) = self.font.cvt {
            writer.write(&(TableTag::CVT, cvt));
        }
        if let Some(fpgm) = self.font.fpgm {
            writer.write(&(TableTag::FPGM, fpgm));
        }

        let number_of_h_metrics = writer.write_custom(TableTag::HMTX, |buffer| {
            HmtxTable::write_to_vec(&self.glyphs, buffer)
        });
        let mut hhea = self.font.hhea;
        hhea.number_of_h_metrics = number_of_h_metrics;
        writer.write(&hhea);

        let mut maxp = self.font.maxp;
        // `unwrap()` should be safe: the subset shouldn't contain >65536 glyphs because the original font doesn't.
        let glyph_count = u16::try_from(self.glyphs.len()).unwrap();
        maxp.subset(glyph_count);
        writer.write(&maxp);

        // TODO: reduce `name` table?
        writer.write(&self.font.name);

        let mut os2 = self.font.os2;
        os2.subset(self.char_range());
        writer.write(&os2);

        let post = self.font.post.bytes();
        writer.write_custom(TableTag::POST, |buffer| {
            // Truncate the `post` table to not contain glyph names
            buffer.write_u32(0x_0003_0000); // version
            buffer.extend_from_slice(&post[4..32]);
        });

        if let Some(prep) = self.font.prep {
            writer.write(&(TableTag::PREP, prep));
        }

        let locations = writer.write_custom(TableTag::GLYF, |buffer| {
            let mut locations = vec![0];
            let initial_offset = buffer.len();
            for glyph in &self.glyphs {
                let glyph = &glyph.inner;
                glyph.write_to_vec(buffer);
                locations.push(buffer.len() - initial_offset);
            }
            locations
        });

        let loca_format = writer.write_custom(TableTag::LOCA, |buffer| {
            LocaTable::write_to_vec(&locations, buffer)
        });

        let mut head = self.font.head;
        head.subset(loca_format, &self.glyphs);
        writer.write(&head);

        writer
    }
}

impl MaxpTable<'_> {
    fn subset(&mut self, glyph_count: u16) {
        self.glyph_count = glyph_count;
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

    fn write_opentype(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&self.tag.0);
        buffer.write_u32(self.checksum);
        buffer.write_u32(self.offset);
        buffer.write_u32(self.length);
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

    fn write_custom<T>(&mut self, tag: TableTag, with: impl FnOnce(&mut Vec<u8>) -> T) -> T {
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

    fn write(&mut self, table: &impl WriteTable) {
        self.write_custom(table.tag(), |buffer| table.write_to_vec(buffer));
    }

    fn write_sfnt_header(&self) -> Vec<u8> {
        let mut buffer = vec![];
        buffer.write_u32(Font::SFNT_VERSION);

        // `unwrap()`s are safe: we don't have many tables written.
        let table_count = u16::try_from(self.tables.len()).unwrap();
        buffer.write_u16(table_count);
        let entry_selector = u16::try_from(table_count.ilog2()).unwrap();
        let search_range = 1 << (4 + entry_selector);
        buffer.write_u16(search_range);
        buffer.write_u16(entry_selector);
        let range_shift = 16 * table_count - search_range;
        buffer.write_u16(range_shift);

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
        buffer.write_u32(WOFF2_SIGNATURE);
        buffer.write_u32(Font::SFNT_VERSION);
        buffer.write_u32(file_len.try_into().expect("file length overflow"));
        // `unwrap()` is safe: we don't write many tables
        buffer.write_u16(self.tables.len().try_into().unwrap());
        buffer.write_u16(0); // reserved

        let decompressed_len = self.data_offset() + self.table_data.len();
        // `unwrap`s are safe, since `file_len` fits into u32.
        buffer.write_u32(decompressed_len.try_into().unwrap());
        buffer.write_u32(compressed_data.len().try_into().unwrap());
        buffer.write_u32(0); // WOFF version
        buffer.write_u32(0); // metadata offset
        buffer.write_u32(0); // metadata length
        buffer.write_u32(0); // original metadata length
        buffer.write_u32(0); // private block offset
        buffer.write_u32(0); // private block length
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
        debug_assert_eq!(file_len, buffer.len());
        buffer
    }
}
