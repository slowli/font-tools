use std::fs;

use allsorts::{binary::read::ReadScope, font::MatchingPresentation, font_data::FontData};

use crate::{Font, FontSubset};

#[test]
fn reading_font() {
    let font_bytes = fs::read("src/tests/FiraMono-Regular.ttf").unwrap();
    let font = Font::parse(&font_bytes).unwrap();

    let font_file = ReadScope::new(&font_bytes).read::<FontData>().unwrap();
    let font_provider = font_file.table_provider(0).unwrap();
    let mut reference_font = allsorts::Font::new(font_provider).unwrap();

    let test_str = "Hello, world! More text ├└█▒";
    let mut glyph_ids = vec![];
    for ch in test_str.chars() {
        let glyph_idx = font.map_char(ch).unwrap();
        let (expected_idx, _) =
            reference_font.lookup_glyph_index(ch, MatchingPresentation::NotRequired, None);
        assert_eq!(glyph_idx, expected_idx);
        glyph_ids.push(glyph_idx);
    }

    let subset = FontSubset::new(font, test_str.chars().collect()).unwrap();
    let ttf = subset.write().into_truetype();
    assert_valid_font(&ttf, test_str);

    fs::write("src/tests/FiraMono-subset.ttf", ttf).unwrap();

    /*let used_chars: BTreeSet<char> = ('!'..='~').collect();
    dbg!(used_chars.len());
    let subset = FontSubset::new(font, used_chars).unwrap();
    subset.write();*/
}

fn assert_valid_font(raw: &[u8], expected_chars: &str) {
    Font::parse(raw).unwrap();

    let font_file = ReadScope::new(raw).read::<FontData>().unwrap();
    let font_provider = font_file.table_provider(0).unwrap();
    let mut font = allsorts::Font::new(font_provider).unwrap();
    for ch in expected_chars.chars() {
        font.lookup_glyph_index(ch, MatchingPresentation::NotRequired, None);
    }
}
