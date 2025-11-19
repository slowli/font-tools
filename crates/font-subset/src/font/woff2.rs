//! WOFF2 font parsing.

use super::{Cursor, Font};
use crate::{alloc::Vec, utils::brotli, ParseError, ParseErrorKind, TableTag};

impl Cursor<'_> {
    fn read_u8(&mut self) -> Result<u8, ParseError> {
        let [a, rest @ ..] = self.bytes else {
            return Err(self.err(ParseErrorKind::UnexpectedEof));
        };
        self.bytes = rest;
        self.offset += 1;
        Ok(*a)
    }

    // visible for testing
    pub(crate) fn read_uint_base128(&mut self) -> Result<u32, ParseError> {
        let offset = self.offset;
        let mut val = 0_u32;
        for _ in 0..5 {
            let byte = self.read_u8()?;
            val = (val << 7) + u32::from(byte & 0x7f);
            if byte < 0x80 {
                // This is the terminal byte
                return Ok(val);
            }
        }
        Err(ParseError {
            kind: ParseErrorKind::UintBase128,
            table: self.table,
            offset,
        })
    }
}

impl TableTag {
    pub(crate) const NULL_TRANSFORM_MASK: u8 = 0b_1100_0000;

    fn parse_woff2(cursor: &mut Cursor<'_>) -> Result<Self, ParseError> {
        // Stash the cursor for error handling.
        let start_cursor = *cursor;

        let raw_tag = cursor.read_u8()?;
        let tag = Self::from_u8(raw_tag);
        let tag = if let Some(tag) = tag {
            tag
        } else {
            Self(cursor.read_byte_array::<4>()?)
        };

        let transform_bits = raw_tag & Self::NULL_TRANSFORM_MASK;
        let expected_transform = match tag {
            Self::GLYF | Self::LOCA => Self::NULL_TRANSFORM_MASK,
            _ => 0,
        };
        if transform_bits != expected_transform {
            return Err(start_cursor.err(ParseErrorKind::UnsupportedWoff2Table {
                tag,
                transform_bits: transform_bits >> 6,
            }));
        }

        Ok(tag)
    }
}

#[derive(Debug, Clone, Copy)]
struct Woff2TableRecord {
    tag: TableTag,
    len: u32,
}

impl Woff2TableRecord {
    fn parse(cursor: &mut Cursor<'_>) -> Result<Self, ParseError> {
        let tag = TableTag::parse_woff2(cursor)?;
        let len = cursor.read_uint_base128()?;
        // Since we don't support non-null transforms, we don't need to read the transformed table length.
        Ok(Self { tag, len })
    }
}

impl Font<'_> {
    pub(crate) const WOFF2_SIGNATURE: u32 = 0x_774f_4632;
}

/// Reader for files in the WOFF2 format.
///
/// Unlike [`OpenTypeReader`](super::OpenTypeReader), this reader owns the table data since it needs
/// to be decompressed. As a result, [`Self::read()`] will borrow the data from the reader itself,
/// not from the original font bytes.
#[derive(Debug, Clone)]
pub struct Woff2Reader {
    table_records: Vec<Woff2TableRecord>,
    table_data: Vec<u8>,
}

impl Woff2Reader {
    /// Creates a reader from the specified raw bytes.
    ///
    /// This will parse the WOFF2 header and table records and decompress the table data.
    ///
    /// # Errors
    ///
    /// Returns parsing / decompression errors if any are encountered.
    #[allow(clippy::missing_panics_doc)] // false positive
    pub fn new(bytes: &[u8]) -> Result<Self, ParseError> {
        let mut header_cursor = Cursor::new(bytes);
        let bytes_len = u32::try_from(bytes.len())
            .map_err(|_| header_cursor.err(ParseErrorKind::TooLargeFont(bytes.len())))?;

        header_cursor
            .read_u32_checked(|signature| check_exact!(signature, Font::WOFF2_SIGNATURE))?;
        header_cursor.read_u32_checked(|version| check_exact!(version, Font::SFNT_VERSION))?;

        header_cursor.read_u32_checked(|file_len| check_exact!(file_len, bytes_len))?;
        let table_count = header_cursor.read_u16()?;
        header_cursor.skip(6)?; // reserved, decompressed_len
        let compressed_data_len = header_cursor.read_u32()?;
        let compressed_data_len = usize::try_from(compressed_data_len).unwrap();
        header_cursor.skip(24)?; // WOFF version ..= private block length

        let table_records = (0..table_count)
            .map(|_| Woff2TableRecord::parse(&mut header_cursor))
            .collect::<Result<Vec<_>, _>>()?;

        let data_cursor = header_cursor.range(0..compressed_data_len)?;
        let table_data = brotli::decompress(data_cursor.bytes())
            .map_err(|()| data_cursor.err(ParseErrorKind::BrotliDecompression))?;
        Ok(Self {
            table_records,
            table_data,
        })
    }

    // visible for testing
    pub(crate) fn iter(&self) -> impl Iterator<Item = (TableTag, Cursor<'_>)> + '_ {
        let mut offset = 0_usize;
        self.table_records.iter().map(move |record| {
            let table_offset = offset;
            offset += usize::try_from(record.len).unwrap();
            let tag = record.tag;
            let table_data = &self.table_data[table_offset..offset];
            let table_cursor = Cursor::for_table(table_data, table_offset, tag);
            (tag, table_cursor)
        })
    }

    /// Reads a [`Font`] from this reader. The font will borrow data from the reader.
    ///
    /// # Errors
    ///
    /// Returns parsing errors (e.g., on missing required tables).
    pub fn read(&self) -> Result<Font<'_>, ParseError> {
        Font::from_tables(self.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_woff2_tables_are_covered() {
        for val in 0_u8..=62 {
            let table = TableTag::from_u8(val).unwrap();
            assert_eq!(table, TableTag::from_u8(val + 64).unwrap());
            assert_eq!(table, TableTag::from_u8(val + 128).unwrap());
            assert_eq!(table, TableTag::from_u8(val + 192).unwrap());
        }
    }
}
