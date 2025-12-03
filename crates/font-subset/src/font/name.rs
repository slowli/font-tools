//! `name` table.

use core::{cmp, ops};

use super::Cursor;
use crate::{
    alloc::{BTreeMap, String, Vec},
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag,
};

#[derive(Debug, Clone, Copy)]
enum PlatformId {
    Unicode,
    Macintosh,
    Windows,
}

#[derive(Debug)]
struct NameRecord {
    name_id: u16,
    value: Option<String>,
}

impl NameRecord {
    const COPYRIGHT_NOTICE_ID: u16 = 0;
    const FAMILY_NAME_ID: u16 = 1;
    const SUBFAMILY_NAME_ID: u16 = 2;
    const VERSION_ID: u16 = 5;
    const MANUFACTURER_ID: u16 = 8;
    const DESIGNER_ID: u16 = 9;
    const DESIGNER_URL_ID: u16 = 12;
    const LICENSE_ID: u16 = 13;
    const LICENSE_URL_ID: u16 = 14;
    const MAX_STANDARD_ID: u16 = 25;

    const BYTE_SIZE: usize = 12;

    fn parse(cursor: &mut Cursor<'_>, string_storage: Cursor<'_>) -> Result<Self, ParseError> {
        let platform_id = cursor.read_u16_checked(|raw| match raw {
            0 => Ok(PlatformId::Unicode),
            1 => Ok(PlatformId::Macintosh),
            3 => Ok(PlatformId::Windows),
            _ => Err(ParseErrorKind::UnexpectedValue {
                name: "platform_id",
                expected: "one of 0, 1, or 3".into(),
                actual: raw.into(),
            }),
        })?;
        let encoding_id = cursor.read_u16()?;
        cursor.skip(2)?; // language_id; TODO: take into account?
        let name_id = cursor.read_u16()?;
        let length = cursor.read_u16()?;
        let offset = cursor.read_u16()?;

        let offset_usize = usize::from(offset);
        let data_cursor =
            string_storage.read_range(offset_usize..(offset_usize + usize::from(length)))?;
        let is_utf16 = matches!(
            (platform_id, encoding_id),
            (PlatformId::Unicode, _) | (PlatformId::Windows, 1 | 10)
        );

        let value: Option<String> = if is_utf16 {
            if length % 2 != 0 {
                return Err(data_cursor.err(ParseErrorKind::UnexpectedValue {
                    name: "length",
                    expected: "even value".into(),
                    actual: length.into(),
                }));
            }

            // This is how (unstable) `String::from_utf16be()` is implemented on low-endian architectures.
            let u16_iter = data_cursor.bytes().chunks(2).map(|chunk| {
                // `unwrap()` is safe due to the oddity check above
                u16::from_be_bytes(chunk.try_into().unwrap())
            });
            let string = char::decode_utf16(u16_iter)
                .collect::<Result<_, _>>()
                .map_err(|_| data_cursor.err(ParseErrorKind::Utf16))?;
            Some(string)
        } else {
            None
        };

        Ok(Self { name_id, value })
    }
}

/// OpenType font naming information extracted from the `name` table.
#[derive(Debug, Clone, Copy, Default)]
pub struct FontNaming<'a> {
    /// Family name, e.g. "Fira Mono".
    pub family: Option<&'a str>,
    /// Subfamily name, e.g. "Regular".
    pub subfamily: Option<&'a str>,
    version: Option<&'a str>,
    /// Font manufacturer.
    pub manufacturer: Option<&'a str>,
    /// Font designer.
    pub designer: Option<&'a str>,
    /// URL of the font designer.
    pub designer_url: Option<&'a str>,
    /// Copyright notice.
    pub copyright_notice: Option<&'a str>,
    /// Font license.
    pub license: Option<&'a str>,
    /// Font license URL.
    pub license_url: Option<&'a str>,
}

impl<'a> FontNaming<'a> {
    fn new(map: &'a BTreeMap<u16, String>) -> Self {
        Self {
            family: map.get(&NameRecord::FAMILY_NAME_ID).map(String::as_str),
            subfamily: map.get(&NameRecord::SUBFAMILY_NAME_ID).map(String::as_str),
            version: map.get(&NameRecord::VERSION_ID).map(String::as_str),
            manufacturer: map.get(&NameRecord::MANUFACTURER_ID).map(String::as_str),
            designer: map.get(&NameRecord::DESIGNER_ID).map(String::as_str),
            designer_url: map.get(&NameRecord::DESIGNER_URL_ID).map(String::as_str),
            copyright_notice: map
                .get(&NameRecord::COPYRIGHT_NOTICE_ID)
                .map(String::as_str),
            license: map.get(&NameRecord::LICENSE_ID).map(String::as_str),
            license_url: map.get(&NameRecord::LICENSE_URL_ID).map(String::as_str),
        }
    }

    /// Returns the font version, with the "Version " prefix stripped.
    pub fn version(&self) -> Option<&str> {
        let version = self.version?;
        Some(version.strip_prefix("Version ").unwrap_or(version))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NameTable<'a> {
    pub(super) parsed_names: BTreeMap<u16, String>,
    /// `None` for subset fonts
    all_bytes: Option<&'a [u8]>,
}

impl<'a> NameTable<'a> {
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip(cursor), fields(range = ?cursor.range()))
    )]
    pub(super) fn parse(
        mut cursor: Cursor<'a>,
        additional_ids: &[u16],
    ) -> Result<Self, ParseError> {
        let mut string_storage = cursor;
        let all_bytes = cursor.bytes();

        cursor.read_u16_checked(|version| {
            if version != 0 && version != 1 {
                return Err(ParseErrorKind::UnexpectedValue {
                    name: "version",
                    expected: "0 or 1".into(),
                    actual: version.into(),
                });
            }
            Ok(())
        })?;

        let record_count = cursor.read_u16()?;
        let storage_offset = cursor.read_u16()?;
        string_storage.skip(storage_offset.into())?;

        let mut parsed_names = BTreeMap::new();
        for _ in 0..record_count {
            let record = NameRecord::parse(&mut cursor, string_storage)?;
            #[cfg(feature = "tracing")]
            tracing::trace!(?record, "parsed name record");

            let Some(value) = record.value else {
                continue;
            };
            let id = record.name_id;
            if id <= NameRecord::MAX_STANDARD_ID || additional_ids.contains(&id) {
                parsed_names.insert(id, value);
            }
        }
        #[cfg(feature = "tracing")]
        tracing::debug!(?parsed_names, "parsed well-known names");

        Ok(Self {
            parsed_names,
            all_bytes: Some(all_bytes),
        })
    }

    pub(super) fn parsed(&self) -> FontNaming<'_> {
        FontNaming::new(&self.parsed_names)
    }

    pub(crate) fn subset(&mut self, modify_version: bool) {
        const VERSION_APPENDIX: &str = concat!(
            "; subset w/ ",
            env!("CARGO_PKG_NAME"),
            " ",
            env!("CARGO_PKG_VERSION")
        );

        self.all_bytes = None;
        if modify_version {
            let version = self.parsed_names.get_mut(&NameRecord::VERSION_ID);
            if let Some(version) = version {
                if !version.ends_with(VERSION_APPENDIX) {
                    version.push_str(VERSION_APPENDIX);
                }
            }
        }
    }

    /// Interns the provided strings into a single piece of data, encodes it in UTF-16, and
    /// provides `u16` offsets for each string.
    ///
    /// The used approach is quite slow, but it should work for small strings `name` typically deals with.
    fn intern_strings<'s>(
        strings: impl Iterator<Item = (u16, &'s str)>,
    ) -> (Vec<u16>, Vec<ops::Range<usize>>) {
        let mut strings: Vec<_> = strings.collect();
        // Sort strings from longer ones to shorter ones.
        strings.sort_unstable_by_key(|(_, s)| cmp::Reverse(s.len()));

        let (mut data, mut ranges) = (String::new(), Vec::with_capacity(strings.len()));
        for (id, s) in strings {
            let new_offset = if let Some(pos) = data.find(s) {
                pos
            } else {
                let prev_len = data.len();
                data.push_str(s);
                prev_len
            };
            ranges.push((id, new_offset..new_offset + s.len()));
        }

        // Now, we need to translate UTF-8 offsets to UTF-16.
        let mut offsets_mut: Vec<_> = ranges
            .iter_mut()
            .flat_map(|(_, range)| [&mut range.start, &mut range.end])
            .collect();

        offsets_mut.sort_unstable_by_key(|offset| **offset);
        let mut utf16_data = Vec::new();
        let mut prev_offset = 0;
        for offset in &mut offsets_mut {
            utf16_data.extend(data[prev_offset..**offset].encode_utf16());
            prev_offset = **offset;
            **offset = utf16_data.len();
        }
        debug_assert_eq!(prev_offset, data.len());

        ranges.sort_unstable_by_key(|(id, _)| *id);
        let offsets = ranges.into_iter().map(|(_, offset)| offset).collect();
        (utf16_data, offsets)
    }
}

impl WriteTable for NameTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::NAME
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        const HEADER_SIZE: usize = 6;

        if let Some(all_bytes) = self.all_bytes {
            buffer.extend_from_slice(all_bytes);
            return;
        }

        let start_offset = buffer.len();
        buffer.write_u16(0); // version
        let record_count = self.parsed_names.len();
        buffer.write_u16(record_count.try_into().expect("record_count overflow"));
        let storage_offset = HEADER_SIZE + NameRecord::BYTE_SIZE * record_count;
        buffer.write_u16(storage_offset.try_into().expect("storage_offset overflow"));

        let (string_data, u16_ranges) =
            Self::intern_strings(self.parsed_names.iter().map(|(&id, s)| (id, s.as_str())));

        for (&id, range) in self.parsed_names.keys().zip(u16_ranges) {
            let len = (range.end - range.start) * 2;
            let len = u16::try_from(len).expect("len overflow");
            let offset = range.start * 2;
            let offset = u16::try_from(offset).expect("offset overflow");

            buffer.write_u16(3); // platform_id = Windows
            buffer.write_u16(1); // encoding_id = Unicode BMP
            buffer.write_u16(0x409); // language_id = en_US
            buffer.write_u16(id);
            buffer.write_u16(len);
            buffer.write_u16(offset);
        }

        debug_assert_eq!(buffer.len() - start_offset, storage_offset);
        buffer.extend(string_data.into_iter().flat_map(u16::to_be_bytes));
    }
}

#[cfg(test)]
mod tests {
    use test_casing::test_casing;

    use super::*;
    use crate::{testonly::TestFont, OpenTypeReader};

    #[test]
    fn interning_strings() {
        let strings = [(0, "Roboto"), (1, "Roboto Regular"), (2, "Regular")];
        let (utf16_data, ranges) = NameTable::intern_strings(strings.into_iter());
        assert_eq!(
            utf16_data,
            "Roboto Regular".encode_utf16().collect::<Vec<_>>(),
        );
        assert_eq!(ranges, [0..6, 0..14, 7..14]);
    }

    #[test_casing(5, TestFont::ALL)]
    fn interning_string_from_font(font: TestFont) {
        let reader = OpenTypeReader::new(font.bytes).unwrap();
        let table_cursor = reader.table(TableTag::NAME);
        let name = NameTable::parse(table_cursor, &[]).unwrap();
        assert!(!name.parsed_names.is_empty());

        let (utf16_data, ranges) =
            NameTable::intern_strings(name.parsed_names.iter().map(|(&id, s)| (id, s.as_str())));

        assert!(utf16_data.len() * 2 < table_cursor.bytes().len());
        for (s, range) in name.parsed_names.values().zip(ranges) {
            let interned_s = String::from_utf16(&utf16_data[range]).unwrap();
            assert_eq!(*s, interned_s);
        }
    }

    #[test_casing(5, TestFont::ALL)]
    fn subsetting_roundtrip(font: TestFont) {
        let reader = OpenTypeReader::new(font.bytes).unwrap();
        let table_cursor = reader.table(TableTag::NAME);
        let mut name = NameTable::parse(table_cursor, &[]).unwrap();
        let original_names = name.parsed_names.clone();

        name.subset(false);
        let mut buffer = vec![];
        name.write_to_vec(&mut buffer);
        let subset_name = NameTable::parse(Cursor::new(&buffer), &[]).unwrap();
        assert_eq!(subset_name.parsed_names, original_names);
    }

    #[test]
    fn modifying_font_version() {
        let reader = OpenTypeReader::new(TestFont::FIRA_MONO.bytes).unwrap();
        let table_cursor = reader.table(TableTag::NAME);
        let mut name = NameTable::parse(table_cursor, &[]).unwrap();

        name.subset(true);
        assert_eq!(
            name.parsed().version(),
            Some("3.111; subset w/ font-subset 0.1.0")
        );

        let mut buffer = vec![];
        name.write_to_vec(&mut buffer);
        let subset_name = NameTable::parse(Cursor::new(&buffer), &[]).unwrap();

        assert_eq!(
            subset_name.parsed_names[&NameRecord::VERSION_ID],
            "Version 3.111; subset w/ font-subset 0.1.0"
        );
        assert_eq!(
            subset_name.parsed().version(),
            Some("3.111; subset w/ font-subset 0.1.0")
        );
    }
}
