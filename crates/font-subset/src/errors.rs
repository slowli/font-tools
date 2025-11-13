use core::{fmt, ops};

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
    /// Unexpected table version.
    UnexpectedTableVersion(u32),
    /// Unexpected table length.
    UnexpectedTableLen {
        /// Expected length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
    /// Unexpected table format (e.g., for a `cmap` subtable).
    UnexpectedTableFormat(u16),
    /// Checksum mismatch.
    Checksum {
        /// Expected checksum.
        expected: u32,
        /// Actual checksum read from the font data.
        actual: u32,
    },
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => formatter.write_str("unexpected end of the font data"),
            Self::UnexpectedFontVersion => formatter.write_str("unexpected font version"),
            Self::MissingTable => formatter.write_str("missing required font table"),
            Self::UnalignedTable => {
                formatter.write_str("font table is not aligned to a 4-byte boundary")
            }
            Self::NoSupportedCmap => {
                formatter.write_str("no supported subtable in the `cmap` table")
            }
            Self::OffsetOutOfBounds(val) => {
                write!(
                    formatter,
                    "offset ({val}) inferred from the table data is out of bounds"
                )
            }
            Self::RangeOutOfBounds { range, len } => {
                write!(
                    formatter,
                    "range ({range:?}) inferred from the table data is out of bounds (..{len})"
                )
            }
            Self::UnexpectedTableVersion(val) => {
                write!(formatter, "unexpected table version ({val})")
            }
            Self::UnexpectedTableLen { expected, actual } => {
                write!(
                    formatter,
                    "unexpected table length: expected {expected}, got {actual}"
                )
            }
            Self::UnexpectedTableFormat(val) => {
                write!(formatter, "unexpected table format ({val})")
            }
            Self::Checksum { expected, actual } => {
                write!(
                    formatter,
                    "unexpected checksum: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ParseErrorKind {}

/// Errors that can occur when parsing an OpenType [`Font`](crate::Font).
#[derive(Debug)]
pub struct ParseError {
    pub(crate) kind: ParseErrorKind,
    pub(crate) offset: usize,
    pub(crate) table: Option<TableTag>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(table) = self.table {
            write!(formatter, "[{table}] ")?;
        }
        if self.offset > 0 {
            write!(formatter, "{}: ", self.offset)?;
        }
        fmt::Display::fmt(&self.kind, formatter)
    }
}

impl std::error::Error for ParseError {}

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
