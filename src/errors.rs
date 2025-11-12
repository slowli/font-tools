use core::ops;

use crate::TableTag;

#[derive(Debug)]
#[non_exhaustive]
pub enum MapError {
    CharTooLarge,
    InvalidOffset,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ParseErrorKind {
    UnexpectedEof,
    UnexpectedFontVersion,
    MissingTable,
    UnalignedTable,
    NoSupportedCmap,
    RangeOutOfBounds {
        range: ops::Range<usize>,
        len: usize,
    },
    UnexpectedTableVersion {
        version: u32,
    },
    UnexpectedTableLen {
        expected: usize,
        actual: usize,
    },
    UnexpectedTableFormat {
        format: u16,
    },
    Checksum {
        expected: u32,
        actual: u32,
    },
    Map(MapError),
}

impl From<MapError> for ParseErrorKind {
    fn from(err: MapError) -> Self {
        Self::Map(err)
    }
}

#[derive(Debug)]
pub struct ParseError {
    pub(crate) kind: ParseErrorKind,
    pub(crate) offset: usize,
    pub(crate) table: Option<TableTag>,
}

impl ParseError {
    pub(crate) fn missing_table(tag: [u8; 4]) -> Self {
        Self {
            kind: ParseErrorKind::MissingTable,
            offset: 0,
            table: Some(TableTag(tag)),
        }
    }

    pub fn kind(&self) -> &ParseErrorKind {
        &self.kind
    }

    pub fn table(&self) -> Option<TableTag> {
        self.table
    }

    pub fn offset(&self) -> usize {
        self.offset
    }
}
