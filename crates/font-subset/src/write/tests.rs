use std::borrow::Cow;

use allsorts::{binary::read::ReadScope, font_data::FontData, tables::FontTableProvider};
use test_casing::{test_casing, Product};

use super::*;
use crate::{
    testonly::{TestCharSubset, TestFont, FONTS, SUBSET_CHARS},
    OpenTypeReader,
};

impl Font<'_> {
    fn table(&self, tag: TableTag) -> &dyn WriteTable {
        match tag {
            // We don't test `cmap` because the test fonts have multiple subtables, and we only retain one of them.
            TableTag::HEAD => &self.head,
            TableTag::OS2 => &self.os2,
            TableTag::HHEA => &self.hhea,
            TableTag::MAXP => &self.maxp,
            TableTag::NAME => &self.name,
            TableTag::POST => &self.post,
            TableTag::GLYF => &self.glyf,
            TableTag::LOCA => &self.loca,
            _ => unreachable!("not called with other tables"),
        }
    }
}

fn test_table_roundtrip(font: TestFont, table: TableTag) {
    let raw = font.bytes;
    let reader = OpenTypeReader::new(raw).unwrap();
    let font = reader.read().unwrap();

    let mut buffer = vec![];
    let table_writer = font.table(table);
    assert_eq!(table_writer.tag(), table);
    table_writer.write_to_vec(&mut buffer);

    let expected_data = reader
        .iter()
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

#[test_casing(2, FONTS)]
fn post_table_roundtrip(font: TestFont) {
    test_table_roundtrip(font, TableTag::POST);
}

#[test_casing(2, FONTS)]
fn glyf_table_roundtrip(font: TestFont) {
    test_table_roundtrip(font, TableTag::GLYF);
}

#[test_casing(2, FONTS)]
fn loca_table_roundtrip(font: TestFont) {
    test_table_roundtrip(font, TableTag::LOCA);
}

fn test_tables_correctness(
    font: TestFont,
    chars: TestCharSubset,
    write: impl FnOnce(FontWriter) -> Vec<u8>,
) {
    let font = Font::opentype(font.bytes).unwrap();
    let writer = font.subset(&chars.into_set()).unwrap().to_writer();
    let FontWriter {
        tables, table_data, ..
    } = writer.clone();
    let serialized = write(writer);

    let font_file = ReadScope::new(&serialized).read::<FontData>().unwrap();
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

#[test_casing(10, Product((FONTS, SUBSET_CHARS)))]
fn opentype_tables_are_written_correctly(font: TestFont, chars: TestCharSubset) {
    test_tables_correctness(font, chars, FontWriter::into_opentype);
}

#[cfg(feature = "woff2")]
#[test_casing(10, Product((FONTS, SUBSET_CHARS)))]
fn woff2_tables_are_written_correctly(font: TestFont, chars: TestCharSubset) {
    test_tables_correctness(font, chars, FontWriter::into_woff2);
}
