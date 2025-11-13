use core::ops;

use crate::TableTag;

/// Kind of a font [`ParseError`].
#[derive(Debug)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// Unexpected end of the font data.
    UnexpectedEof,
    /// Unexpected font version.
    UnexpectedFontVersion,
    /// Missing required font table (e.g., `head`).
    MissingTable,
    /// A font table is not aligned to a 4-byte boundary.
    UnalignedTable,
    /// No supported subtable in the `cmap` table.
    NoSupportedCmap,
    /// Offset inferred from the table data is out of bounds.
    OffsetOutOfBounds(usize),
    /// Range inferred from the table data is out of bounds.
    RangeOutOfBounds {
        /// Inferred range.
        range: ops::Range<usize>,
        /// Length of the indexed data.
        len: usize,
    },
    /// Unexpected version of a table.
    UnexpectedTableVersion {
        /// Actual table version.
        version: u32,
    },
    /// Unexpected table length.
    UnexpectedTableLen {
        /// Expected length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
    /// Unexpected table format (e.g., for a `cmap` subtable).
    UnexpectedTableFormat {
        /// Actual format.
        format: u16,
    },
    /// Checksum mismatch.
    Checksum {
        /// Expected checksum.
        expected: u32,
        /// Actual checksum read from the font data.
        actual: u32,
    },
}

/// Errors that can occur when parsing an OpenType [`Font`](crate::Font).
#[derive(Debug)]
pub struct ParseError {
    pub(crate) kind: ParseErrorKind,
    pub(crate) offset: usize,
    pub(crate) table: Option<TableTag>,
}

impl ParseError {
    pub(crate) fn missing_table(tag: TableTag) -> Self {
        Self {
            kind: ParseErrorKind::MissingTable,
            offset: 0,
            table: Some(tag),
        }
    }

    /// Gets the error kind.
    pub fn kind(&self) -> &ParseErrorKind {
        &self.kind
    }

    /// Gets the table this error relates to.
    pub fn table(&self) -> Option<TableTag> {
        self.table
    }

    /// Gets the offset in the font data.
    pub fn offset(&self) -> usize {
        self.offset
    }
}
