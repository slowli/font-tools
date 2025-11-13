use std::{
    collections::BTreeSet, env, fmt, fs, io, io::Write, ops, process::Command, sync::OnceLock,
};

use allsorts::{binary::read::ReadScope, font::MatchingPresentation, font_data::FontData};
use test_casing::{test_casing, Product};

use crate::{Font, FontSubset};

#[derive(Clone, Copy)]
pub(crate) struct TestFont {
    pub(crate) name: &'static str,
    pub(crate) bytes: &'static [u8],
}

impl fmt::Debug for TestFont {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.name, formatter)
    }
}

const MONO_FONT: TestFont = TestFont {
    name: "Fira Mono",
    bytes: include_bytes!("../../examples/FiraMono-Regular.ttf"),
};
const SANS_FONT: TestFont = TestFont {
    name: "Roboto",
    bytes: include_bytes!("../../examples/Roboto-VariableFont_wdth,wght.ttf"),
};

pub(crate) const FONTS: [TestFont; 2] = [MONO_FONT, SANS_FONT];

#[derive(Debug, Clone)]
pub(crate) enum TestCharSubset {
    Range(ops::RangeInclusive<char>),
    Str(&'static str),
}

impl TestCharSubset {
    pub(crate) fn into_set(self) -> BTreeSet<char> {
        match self {
            Self::Range(range) => range.collect(),
            Self::Str(s) => s.chars().collect(),
        }
    }
}

pub(crate) const SUBSET_CHARS: [TestCharSubset; 5] = [
    TestCharSubset::Range(' '..='~'),
    TestCharSubset::Range('a'..='z'),
    TestCharSubset::Range('0'..='9'),
    TestCharSubset::Str("Hello world!"),
    TestCharSubset::Str("A"),
];

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
fn reading_font() {
    let font = Font::new(MONO_FONT.bytes).unwrap();

    let font_file = ReadScope::new(MONO_FONT.bytes).read::<FontData>().unwrap();
    let font_provider = font_file.table_provider(0).unwrap();
    let mut reference_font = allsorts::Font::new(font_provider).unwrap();

    let test_str = "Hello, world! More text ├└█▒";
    let mut glyph_ids = vec![];
    for ch in test_str.chars() {
        let id = font.map_char(ch).unwrap();
        let (expected_idx, _) =
            reference_font.lookup_glyph_index(ch, MatchingPresentation::NotRequired, None);
        assert_eq!(id, expected_idx);
        glyph_ids.push(id);
    }
}

#[test]
fn subsetting_mono_font_with_ascii_chars() {
    let chars: BTreeSet<char> = (' '..='~').collect();
    let (ttf, woff2) = test_subsetting_font(MONO_FONT, &chars);
    assert_snapshot("examples/FiraMono-ascii.ttf", &ttf);
    assert_snapshot("examples/FiraMono-ascii.woff", &woff2);
}

#[test_casing(10, Product((FONTS, SUBSET_CHARS)))]
fn subsetting_font(font: TestFont, chars: TestCharSubset) {
    let chars = chars.into_set();
    test_subsetting_font(font, &chars);
}

fn test_subsetting_font(font: TestFont, chars: &BTreeSet<char>) -> (Vec<u8>, Vec<u8>) {
    let font = Font::new(font.bytes).unwrap();
    let subset = FontSubset::new(font, chars).unwrap();

    let ttf = subset.to_truetype();
    assert_valid_font(&ttf, true, chars.iter().copied());
    let woff2 = subset.to_woff2();
    assert_valid_font(&woff2, false, chars.iter().copied());
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
    let (ttf, woff2) = test_subsetting_font(SANS_FONT, &chars);
    assert_snapshot("examples/Roboto-ascii.ttf", &ttf);
    assert_snapshot("examples/Roboto-ascii.woff", &woff2);
}

fn assert_valid_font(raw: &[u8], is_ttf: bool, expected_chars: impl Iterator<Item = char>) {
    if is_ttf {
        Font::new(raw).unwrap();
    }

    let font_file = ReadScope::new(raw).read::<FontData>().unwrap();
    let font_provider = font_file.table_provider(0).unwrap();
    let mut font = allsorts::Font::new(font_provider).unwrap();
    for ch in expected_chars {
        let (glyph_id, _) = font.lookup_glyph_index(ch, MatchingPresentation::NotRequired, None);
        assert_ne!(glyph_id, 0);
    }

    OpenTypeSanitizer::get().validate(raw);
}
