use core::{fmt, ops};

use crate::{
    alloc::{format, vec, String, Vec},
    TableTag,
};

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
    /// Invalid Unicode char code. Unicode char codes must be in `0..=0xd7ff` or `0xe000..=0x10ffff`.
    InvalidCharCode(u32),
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
    /// UTF-16 decoding error.
    Utf16,
    /// `uint128` decoding error (used in WOFF2).
    Uint128,
    /// Unsupported WOFF2 table tag.
    UnsupportedWoff2Table(u8),
    /// Font size is too large.
    TooLargeFont(usize),
    /// Brotli decompression error for WOFF2.
    BrotliDecompression,
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
            Self::InvalidCharCode(val) => {
                write!(formatter, "invalid Unicode char code: {val}")
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
            Self::Utf16 => formatter.write_str("failed decoding UTF-16 string"),
            Self::Uint128 => formatter.write_str("failed decoding uint128 value"),
            Self::UnsupportedWoff2Table(tag) => {
                write!(formatter, "unsupported WOFF2 table tag ({tag})")
            }
            Self::TooLargeFont(len) => {
                write!(formatter, "font size ({len}) is too large to be processed")
            }
            Self::BrotliDecompression => formatter.write_str("Brotli decompression error"),
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

/// Kind of a [`Warning`].
#[derive(Debug)]
#[non_exhaustive]
pub enum WarningKind {
    /// Mismatch between the computed and recorded value.
    ValueMismatch {
        /// Name of the value.
        name: &'static str,
        /// Computed value.
        computed: String,
        /// Actual encountered value.
        recorded: String,
    },
}

impl fmt::Display for WarningKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ValueMismatch {
                name,
                computed,
                recorded,
            } => {
                write!(
                    formatter,
                    "mismatch between computed ({computed}) and recorded ({recorded}) values of `{name}`"
                )
            }
        }
    }
}

/// Warning that can occur [validating a font](crate::Font::validate()).
#[derive(Debug)]
pub struct Warning {
    kind: WarningKind,
    table: Option<TableTag>,
}

impl fmt::Display for Warning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(table) = self.table {
            write!(formatter, "[{table}] ")?;
        }
        fmt::Display::fmt(&self.kind, formatter)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Warning {}

impl Warning {
    /// Returns the kind of this warning.
    pub fn kind(&self) -> &WarningKind {
        &self.kind
    }

    /// Gets the table this warning relates to.
    pub fn table(&self) -> Option<TableTag> {
        self.table
    }
}

/// Set of [`Warning`]s.
#[derive(Debug)]
pub struct Warnings(Vec<Warning>);

impl fmt::Display for Warnings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for warn in &self.0 {
            writeln!(formatter, "- {warn}")?;
        }
        Ok(())
    }
}

impl IntoIterator for Warnings {
    type Item = Warning;
    type IntoIter = vec::IntoIter<Warning>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Warnings {}

impl Warnings {
    pub(crate) fn empty() -> Self {
        Self(vec![])
    }

    /// Checks if there's at least one warning.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of contained warnings.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Iterates over the contained warnings in no particular order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Warning> + '_ {
        self.0.iter()
    }

    pub(crate) fn for_table(&mut self, table: TableTag) -> TableWarnings<'_> {
        TableWarnings { inner: self, table }
    }

    /// Converts these warnings into a `Result`. This is useful to treat warnings as errors.
    ///
    /// # Errors
    ///
    /// Returns `Err(_)` if there's at least one warning.
    pub fn into_result(self) -> Result<(), Self> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

#[derive(Debug)]
pub(crate) struct TableWarnings<'a> {
    inner: &'a mut Warnings,
    table: TableTag,
}

impl TableWarnings<'_> {
    pub(crate) fn check_match<T: Copy + PartialEq + fmt::Debug>(
        &mut self,
        name: &'static str,
        computed: T,
        recorded: T,
    ) {
        if computed != recorded {
            self.inner.0.push(Warning {
                kind: WarningKind::ValueMismatch {
                    name,
                    computed: format!("{computed:?}"),
                    recorded: format!("{recorded:?}"),
                },
                table: Some(self.table),
            });
        }
    }
}
