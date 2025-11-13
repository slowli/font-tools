use std::{collections::BTreeSet, env, fs, io};

use allsorts::{binary::read::ReadScope, font::MatchingPresentation, font_data::FontData};

use crate::{Font, FontSubset};

#[test]
fn reading_font() {
    let font_bytes = fs::read("examples/FiraMono-Regular.ttf").unwrap();
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
}

#[test]
fn subsetting_mono_font_with_ascii_chars() {
    let chars: BTreeSet<char> = (' '..='~').collect();
    let (ttf, woff2) = test_subsetting_font("examples/FiraMono-Regular.ttf", &chars);
    assert_snapshot("examples/FiraMono-ascii.ttf", &ttf);
    assert_snapshot("examples/FiraMono-ascii.woff", &woff2);
}

fn test_subsetting_font(path: &str, chars: &BTreeSet<char>) -> (Vec<u8>, Vec<u8>) {
    let font_bytes = fs::read(path).unwrap();
    let font = Font::parse(&font_bytes).unwrap();
    let subset = FontSubset::new(font, chars).unwrap();

    let ttf = subset.to_truetype();
    assert_valid_font(&ttf, true, chars.iter().copied());
    let woff2 = subset.to_woff2();
    assert_valid_font(&ttf, false, chars.iter().copied());
    (ttf, woff2)
}

fn assert_snapshot(path: &str, actual: &[u8]) {
    let is_ci = env::var("CI").is_ok_and(|var| var != "0");
    let expected = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(err) if matches!(err.kind(), io::ErrorKind::NotFound) && !is_ci => None,
        Err(err) => panic!("Error reading snapshot {path}: {err}"),
    };

    if expected.as_ref().is_none_or(|exp| exp != actual) && !is_ci {
        let save_path = format!("{path}.new");
        fs::write(save_path, actual).unwrap();
    }
    assert_eq!(expected.as_deref(), Some(actual));
}

#[test]
fn subsetting_sans_font_with_ascii_chars() {
    let chars: BTreeSet<char> = (' '..='~').collect();
    let (ttf, woff2) = test_subsetting_font("examples/Roboto-VariableFont_wdth,wght.ttf", &chars);
    assert_snapshot("examples/Roboto-ascii.ttf", &ttf);
    assert_snapshot("examples/Roboto-ascii.woff", &woff2);
}

fn assert_valid_font(raw: &[u8], is_ttf: bool, expected_chars: impl Iterator<Item = char>) {
    if is_ttf {
        Font::parse(raw).unwrap();
    }

    let font_file = ReadScope::new(raw).read::<FontData>().unwrap();
    let font_provider = font_file.table_provider(0).unwrap();
    let mut font = allsorts::Font::new(font_provider).unwrap();
    for ch in expected_chars {
        let (glyph_id, _) = font.lookup_glyph_index(ch, MatchingPresentation::NotRequired, None);
        assert_ne!(glyph_id, 0);
    }
}
