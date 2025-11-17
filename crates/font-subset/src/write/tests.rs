use test_casing::test_casing;

use super::*;
use crate::tests::{TestFont, FONTS};

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
