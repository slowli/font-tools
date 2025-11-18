//! High-level tests for font subsetting (including snapshot tests for the subset fonts in the `examples/` dir).

use std::{collections::BTreeSet, env, fs, io, io::Write, process::Command, sync::OnceLock};

use allsorts::{binary::read::ReadScope, font::MatchingPresentation, font_data::FontData};
use font_subset::{Font, FontReader, VariableAxisTag};
use test_casing::{test_casing, Product};

use crate::testonly::{TestCharSubset, TestFont, SUBSET_CHARS};

#[path = "../src/testonly.rs"]
mod testonly;

#[derive(Debug)]
struct OpenTypeSanitizer {
    path: Option<String>,
}

impl Default for OpenTypeSanitizer {
    fn default() -> Self {
        let Ok(path) = env::var("OTS_SANITIZER") else {
            return Self { path: None };
        };
        let output = Command::new(&path)
            .arg("--version")
            .output()
            .unwrap_or_else(|err| {
                panic!("failed getting version for ots-sanitize at {path}: {err}");
            });
        assert!(
            output.status.success(),
            "failed getting version for ots-sanitize at {path}: non-zero exit code"
        );
        let version = String::from_utf8(output.stdout).unwrap_or_else(|err| {
            panic!("failed getting version for ots-sanitize at {path}: {err}");
        });
        println!("ots-sanitize version: {version}");
        Self { path: Some(path) }
    }
}

impl OpenTypeSanitizer {
    fn get() -> &'static Self {
        static SANITIZER: OnceLock<OpenTypeSanitizer> = OnceLock::new();
        SANITIZER.get_or_init(Self::default)
    }

    fn validate(&self, content: &[u8]) {
        let Some(path) = &self.path else {
            println!("OTS_SANITIZER env var is missing; skipping checks");
            return;
        };

        // Save content to the temporary file.
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.as_file_mut().write_all(content).unwrap();
        file.as_file_mut().flush().unwrap();
        let file_path = file.into_temp_path();

        let output = Command::new(path)
            .arg(&file_path)
            .output()
            .expect("failed running ots-sanitize");
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("ots-sanitize failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}");
        }
    }
}

#[test]
fn subsetting_mono_font_with_ascii_chars() {
    let font = Font::opentype(TestFont::FIRA_MONO.bytes).unwrap();
    let chars: BTreeSet<char> = (' '..='~').collect();
    let (ttf, woff2) = test_subsetting_font(&font, &chars);
    assert_snapshot("examples/FiraMono-ascii.ttf", &ttf);
    assert_snapshot("examples/FiraMono-ascii.woff", &woff2);
}

#[test]
fn subsetting_variable_mono_font_with_ascii_chars() {
    let font = Font::opentype(TestFont::ROBOTO_MONO.bytes).unwrap();
    let chars: BTreeSet<char> = (' '..='~').collect();
    let (ttf, woff2) = test_subsetting_font(&font, &chars);
    assert_snapshot("examples/RobotoMono-ascii.ttf", &ttf);
    assert_snapshot("examples/RobotoMono-ascii.woff", &woff2);
}

#[test_casing(15, Product((TestFont::ALL, SUBSET_CHARS)))]
fn subsetting_font(font: TestFont, chars: TestCharSubset) {
    let chars = chars.into_set();
    let font = Font::opentype(font.bytes).unwrap();
    test_subsetting_font(&font, &chars);
}

#[test_casing(10, Product((TestFont::VAR, SUBSET_CHARS)))]
fn subsetting_font_with_dropped_vars(font: TestFont, chars: TestCharSubset) {
    let chars = chars.into_set();
    let mut font = Font::opentype(font.bytes).unwrap();

    assert!(font.is_variable());
    let weight_axis = font
        .variable_axes()
        .unwrap()
        .iter()
        .find(|axis| axis.tag == VariableAxisTag::WEIGHT)
        .unwrap();
    assert_eq!(weight_axis.default_value, 400_i16.into());
    assert_eq!(weight_axis.name.as_deref(), Some("Weight"));

    font.drop_variables();
    test_subsetting_font(&font, &chars);
}

fn test_subsetting_font(font: &Font<'_>, chars: &BTreeSet<char>) -> (Vec<u8>, Vec<u8>) {
    let subset = font.subset(chars).unwrap();
    subset.validate().unwrap().into_result().unwrap();

    let ttf = subset.to_opentype();
    assert_valid_font(&ttf, true, chars);
    let woff2 = subset.to_woff2();
    assert_valid_font(&woff2, false, chars);
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
fn subsetting_sans_font_with_ascii_chars_and_dropped_vars() {
    let mut font = Font::opentype(TestFont::ROBOTO.bytes).unwrap();
    font.drop_variables();
    let chars: BTreeSet<char> = (' '..='~').collect();
    let (ttf, woff2) = test_subsetting_font(&font, &chars);
    assert_snapshot("examples/Roboto-ascii.ttf", &ttf);
    assert_snapshot("examples/Roboto-ascii.woff", &woff2);
}

#[test]
fn subsetting_subset() {
    let font = Font::opentype(TestFont::FIRA_MONO.bytes).unwrap();
    let ascii_chars: BTreeSet<char> = (' '..='~').collect();
    let large_subset = font.subset(&ascii_chars).unwrap();

    for range in ['0'..='9', 'a'..='z', 'A'..='Z'] {
        println!("Testing subset: {range:?}");
        let chars: BTreeSet<char> = range.collect();
        let small_subset = large_subset.subset(&chars).unwrap();
        small_subset.validate().unwrap().into_result().unwrap();
        let ttf = small_subset.to_opentype();
        assert_valid_font(&ttf, true, &chars);

        let subset_from_src = font.subset(&chars).unwrap();
        let ttf_from_src = subset_from_src.to_opentype();
        assert_eq!(ttf, ttf_from_src);
    }
}

fn assert_valid_font(raw: &[u8], is_ttf: bool, expected_chars: &BTreeSet<char>) {
    let reader = FontReader::new(raw).unwrap();
    assert_eq!(is_ttf, matches!(&reader, FontReader::OpenType(_)));
    let parsed_font = reader.read().unwrap();
    parsed_font.validate().unwrap().into_result().unwrap();

    let actual_chars = parsed_font
        .char_ranges()
        .flatten()
        .map(char::try_from)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        actual_chars.iter().eq(expected_chars),
        "expected={expected_chars:?}, got={actual_chars:?}"
    );

    let font_file = ReadScope::new(raw).read::<FontData>().unwrap();
    let font_provider = font_file.table_provider(0).unwrap();
    let mut font = allsorts::Font::new(font_provider).unwrap();
    for &ch in expected_chars {
        let (glyph_id, _) = font.lookup_glyph_index(ch, MatchingPresentation::NotRequired, None);
        assert_ne!(glyph_id, 0);
    }

    OpenTypeSanitizer::get().validate(raw);
}
