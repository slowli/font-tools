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

    fn read_u8_checked<T>(
        &mut self,
        check: impl FnOnce(u8) -> Result<T, ParseErrorKind>,
    ) -> Result<T, ParseError> {
        check(self.read_u8()?).map_err(|kind| ParseError {
            kind,
            table: self.table,
            offset: self.offset - 1, // use the starting offset for the value
        })
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
    const NULL_TRANSFORM: u8 = 0b_1100_0000;
    pub(crate) const NULL_TRANSFORM_GLYF: u8 = Self::NULL_TRANSFORM | 10;
    pub(crate) const NULL_TRANSFORM_LOCA: u8 = Self::NULL_TRANSFORM | 11;

    fn parse_woff2(cursor: &mut Cursor<'_>) -> Result<Option<Self>, ParseError> {
        let (mut tag, is_custom) = cursor.read_u8_checked(|raw| {
            let tag = match raw {
                0 => TableTag::CMAP,
                1 => TableTag::HEAD,
                2 => TableTag::HHEA,
                3 => TableTag::HMTX,
                4 => TableTag::MAXP,
                5 => TableTag::NAME,
                6 => TableTag::OS2,
                7 => TableTag::POST,
                8 => TableTag::CVT,
                9 => TableTag::FPGM,
                Self::NULL_TRANSFORM_GLYF => TableTag::GLYF,
                Self::NULL_TRANSFORM_LOCA => TableTag::LOCA,
                12 => TableTag::PREP,
                13..=62 => return Ok((None, false)),
                63 => return Ok((None, true)),
                _ => return Err(ParseErrorKind::UnsupportedWoff2Table(raw)),
            };
            Ok((Some(tag), false))
        })?;

        if is_custom {
            tag = Some(TableTag(cursor.read_byte_array::<4>()?));
        }
        Ok(tag)
    }
}

#[derive(Debug)]
struct Woff2TableRecord {
    tag: Option<TableTag>,
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
#[derive(Debug)]
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
        self.table_records.iter().filter_map(move |record| {
            let table_offset = offset;
            offset += usize::try_from(record.len).unwrap();
            let tag = record.tag?;
            let table_data = &self.table_data[table_offset..offset];
            let table_cursor = Cursor::for_table(table_data, table_offset, tag);
            Some((tag, table_cursor))
        })
    }

    /// Reads a [`Font`] from this reader. The font will borrow data from the reader.
    ///
    /// # Errors
    ///
    /// Returns parsing errors (e.g., on missing required tables).
    pub fn read(&self) -> Result<Font<'_>, ParseError> {
        Font::from_tables(self.iter().map(Ok))
    }
}
