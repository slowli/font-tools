use core::ops;

#[derive(Debug)]
#[non_exhaustive]
pub enum MapError {
    CharTooLarge,
    InvalidOffset,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ParseError {
    UnexpectedEof,
    UnexpectedFontVersion,
    MissingTable(&'static str),
    UnexpectedTableVersion {
        table: &'static str,
        version: u32,
    },
    UnexpectedLocaFormat(u16),
    UnexpectedTableLen {
        table: &'static str,
        expected: usize,
        actual: usize,
    },
    UnexpectedCmapTableFormat {
        expected: u16,
        actual: u16,
    },
    MissingGlyph {
        glyph_idx: u16,
        range: ops::Range<usize>,
    },
    Map(MapError),
}

impl From<MapError> for ParseError {
    fn from(err: MapError) -> Self {
        Self::Map(err)
    }
}
