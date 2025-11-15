use std::borrow::Cow;

use allsorts::{binary::read::ReadScope, font_data::FontData, tables::FontTableProvider};
use test_casing::{test_casing, Product};

use super::*;
use crate::tests::{TestCharSubset, TestFont, FONTS, SUBSET_CHARS};

impl Font<'_> {
    fn table(&self, tag: TableTag) -> &dyn WriteTable {
        match tag {
            TableTag::HEAD => &self.head,
            TableTag::OS2 => &self.os2,
            TableTag::HHEA => &self.hhea,
            TableTag::MAXP => &self.maxp,
            TableTag::NAME => &self.name,
            _ => unreachable!("not called with other tables"),
        }
    }
}

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

fn test_table_roundtrip(font: TestFont, table: TableTag) {
    let raw = font.bytes;
    let font = Font::new(raw).unwrap();

    let mut buffer = vec![];
    let table_writer = font.table(table);
    assert_eq!(table_writer.tag(), table);
    table_writer.write_to_vec(&mut buffer);

    let expected_data = Font::parse_header(raw)
        .unwrap()
        .map(Result::unwrap)
        .find_map(|(tag, cursor)| (tag == table).then_some(cursor.bytes()))
        .unwrap();
    assert_eq!(buffer, expected_data);
}

#[test_casing(2, FONTS)]
fn head_table_roundtrip(font: TestFont) {
    test_table_roundtrip(font, TableTag::HEAD);
}

#[test_casing(2, FONTS)]
fn os2_table_roundtrip(font: TestFont) {
    test_table_roundtrip(font, TableTag::OS2);
}

#[test_casing(2, FONTS)]
fn hhea_table_roundtrip(font: TestFont) {
    test_table_roundtrip(font, TableTag::HHEA);
}

#[test_casing(2, FONTS)]
fn maxp_table_roundtrip(font: TestFont) {
    test_table_roundtrip(font, TableTag::MAXP);
}

#[test_casing(2, FONTS)]
fn name_table_roundtrip(font: TestFont) {
    test_table_roundtrip(font, TableTag::NAME);
}

#[test_casing(10, Product((FONTS, SUBSET_CHARS)))]
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
