//! High-level WOFF2 serialization logic.

use core::iter;

use super::{FontWriter, TableRecord, VecExt as _};
use crate::{
    alloc::{vec, Vec},
    Font, TableTag,
};

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

impl TableRecord {
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

impl FontWriter {
    const WOFF2_HEADER_LEN: usize = 48;

    pub(super) fn into_woff2(mut self) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base128_encoding() {
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
}
