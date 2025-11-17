//! Logic for serializing `FontSubset`s in OpenType format.

use core::iter;

use crate::{
    alloc::{vec, Vec},
    font::Cursor,
    Font, TableTag,
};

#[cfg(feature = "woff2")]
mod brotli;
#[cfg(test)]
mod tests;
#[cfg(feature = "woff2")]
mod woff2;

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

impl Font<'_> {
    /// Serializes this subset to the OpenType format.
    pub fn to_opentype(&self) -> Vec<u8> {
        self.to_writer().into_opentype()
    }

    /// Serializes this subset to the WOFF2 format.
    #[cfg(feature = "woff2")]
    #[cfg_attr(docsrs, doc(cfg(feature = "woff2")))]
    pub fn to_woff2(&self) -> Vec<u8> {
        self.to_writer().into_woff2()
    }

    fn to_writer(&self) -> FontWriter {
        let mut writer = FontWriter::default();
        writer.write(&self.cmap);
        if let Some(cvt) = self.cvt {
            writer.write(&(TableTag::CVT, cvt));
        }
        if let Some(fpgm) = self.fpgm {
            writer.write(&(TableTag::FPGM, fpgm));
        }
        writer.write(&self.hmtx);
        writer.write(&self.hhea);
        writer.write(&self.maxp);
        writer.write(&self.name);
        writer.write(&self.os2);
        writer.write(&self.post);
        if let Some(prep) = self.prep {
            writer.write(&(TableTag::PREP, prep));
        }
        writer.write(&self.glyf);
        writer.write(&self.loca);
        writer.write(&self.head);
        writer
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
}

#[derive(Debug, Clone, Default)]
struct FontWriter {
    tables: Vec<TableRecord>,
    /// Contains *aligned* table data
    table_data: Vec<u8>,
}

impl FontWriter {
    const SFNT_HEADER_LEN: usize = 12;

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
}
