use core::{fmt, ops};

use crate::{alloc::String, TableTag};

/// Kind of a font [`ParseError`].
#[derive(Debug)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// Unexpected end of the font data.
    UnexpectedEof,
    /// Unexpected numerical value.
    UnexpectedValue {
        /// Name of the value parsed.
        name: &'static str,
        /// Description of the expected value.
        expected: String,
        /// Actual encountered value.
        actual: u32,
    },
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
    /// Unexpected table length.
    UnexpectedTableLen {
        /// Expected length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
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
            Self::UnexpectedValue {
                name,
                expected,
                actual,
            } => {
                write!(
                    formatter,
                    "unexpected value of `{name}`: expected {expected}, got {actual}"
                )
            }
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
            Self::UnexpectedTableLen { expected, actual } => {
                write!(
                    formatter,
                    "unexpected table length: expected {expected}, got {actual}"
                )
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

#[cfg(feature = "std")]
impl std::error::Error for ParseErrorKind {}

macro_rules! check_exact {
    ($val:ident, $expected:expr) => {
        if $val == $expected {
            Ok(())
        } else {
            Err($crate::ParseErrorKind::UnexpectedValue {
                name: ::core::stringify!($val),
                expected: $crate::alloc::ToString::to_string(&$expected),
                actual: u32::from($val),
            })
        }
    };
}

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

#[cfg(feature = "std")]
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
