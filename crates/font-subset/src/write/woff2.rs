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
        let is_custom_tag = self.tag.as_u8().is_none();
        1 /* flags */ + usize::from(is_custom_tag) * 4 + uint_base128_len(self.length)
    }

    fn write_woff2(&self, buffer: &mut Vec<u8>) {
        let mut flags = self.tag.as_u8().unwrap_or(63);
        debug_assert!(flags <= 63);
        let is_custom = flags == 63;
        if matches!(self.tag, TableTag::GLYF | TableTag::LOCA) {
            flags |= TableTag::NULL_TRANSFORM_MASK;
        }
        buffer.push(flags);
        if is_custom {
            buffer.extend_from_slice(&self.tag.0);
        }
        write_uint_base128(buffer, self.length);
    }
}

impl FontWriter {
    const WOFF2_HEADER_LEN: usize = 48;

    #[cfg_attr(feature = "tracing", tracing::instrument(level = "debug", skip_all))]
    pub(super) fn into_woff2(mut self) -> Vec<u8> {
        self.adjust_data(Font::checksum(&self.write_sfnt_header()));

        let compressed_data = self.compress_data();
        #[cfg(feature = "tracing")]
        tracing::debug!(
            compressed_data.len = compressed_data.len(),
            "compressed table data"
        );

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
        buffer.write_u32(Font::WOFF2_SIGNATURE);
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
    use assert_matches::assert_matches;
    use test_casing::{test_casing, Product};

    use super::*;
    use crate::{font::Cursor, testonly::TestFont, ParseErrorKind, Woff2Reader};

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

    #[test]
    fn base128_roundtrip() {
        let near_powers_of_2 = (15..32).flat_map(|exp| {
            let pow = 1_u32 << exp;
            (pow - 100)..=(pow + 100)
        });
        for val in (0..=16_385)
            .chain(near_powers_of_2)
            .chain(u32::MAX - 100..=u32::MAX)
        {
            let mut buffer = vec![];
            write_uint_base128(&mut buffer, val);
            let read = Cursor::new(&buffer).read_uint_base128().unwrap();
            assert_eq!(read, val);
        }

        let err = Cursor::new(&[0x80; 5]).read_uint_base128().unwrap_err();
        assert_matches!(err.kind(), ParseErrorKind::UintBase128);
    }

    #[test_casing(6, Product((TestFont::ALL, [false, true])))]
    fn roundtrip_via_reader_and_writer(font: TestFont, subset: bool) {
        let mut font = Font::opentype(font.bytes).unwrap();
        if subset {
            font = font.subset(&('a'..='z').collect()).unwrap();
        }

        let writer = font.to_writer();
        let woff2 = writer.clone().into_woff2();
        let reader = Woff2Reader::new(&woff2).unwrap();
        let reader_tables: Vec<_> = reader.iter().collect();

        assert_eq!(writer.tables.len(), reader_tables.len());
        for (writer_rec, &(reader_tag, cursor)) in writer.tables.iter().zip(&reader_tables) {
            assert_eq!(writer_rec.tag, reader_tag);
            println!("Checking table {reader_tag}");

            let start = writer_rec.offset as usize;
            let end = start + writer_rec.length as usize;
            let writer_bytes = &writer.table_data[start..end];

            if reader_tag == TableTag::HEAD {
                let mut writer_bytes = writer_bytes.to_vec();
                // Correct `checksum_adjustment`
                writer_bytes[8..12].copy_from_slice(&cursor.bytes()[8..12]);
                assert_eq!(writer_bytes, cursor.bytes());
            } else {
                assert_eq!(writer_bytes, cursor.bytes());
            }
        }
    }
}
