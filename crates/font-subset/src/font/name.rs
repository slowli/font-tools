//! `name` table.

use super::Cursor;
use crate::{
    alloc::{BTreeMap, String, Vec},
    write::WriteTable,
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
    const FAMILY_NAME_ID: u16 = 1;
    const SUBFAMILY_NAME_ID: u16 = 2;
    const MANUFACTURER_ID: u16 = 8;
    const LICENSE_ID: u16 = 13;
    const LICENSE_URL_ID: u16 = 14;

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
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FontNaming {
    /// Family name, e.g. "Fira Mono".
    pub family: Option<String>,
    /// Subfamily name, e.g. "Regular".
    pub subfamily: Option<String>,
    /// Font manufacturer.
    pub manufacturer: Option<String>,
    /// Font license.
    pub license: Option<String>,
    /// Font license URL.
    pub license_url: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct NameTable<'a> {
    pub(super) parsed: FontNaming,
    pub(super) additional_names: BTreeMap<u16, String>,
    all_bytes: &'a [u8],
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

        let mut parsed = FontNaming::default();
        let mut additional_names = BTreeMap::new();
        for _ in 0..record_count {
            let record = NameRecord::parse(&mut cursor, string_storage)?;
            #[cfg(feature = "tracing")]
            tracing::trace!(?record, "parsed name record");

            let Some(value) = record.value else {
                continue;
            };
            match record.name_id {
                NameRecord::FAMILY_NAME_ID => parsed.family = Some(value),
                NameRecord::SUBFAMILY_NAME_ID => parsed.subfamily = Some(value),
                NameRecord::LICENSE_ID => parsed.license = Some(value),
                NameRecord::LICENSE_URL_ID => parsed.license_url = Some(value),
                NameRecord::MANUFACTURER_ID => parsed.manufacturer = Some(value),
                id if additional_ids.contains(&id) => {
                    additional_names.insert(id, value);
                }
                _ => { /* do nothing */ }
            }
        }
        #[cfg(feature = "tracing")]
        tracing::debug!(?parsed, "parsed well-known names");

        Ok(Self {
            parsed,
            additional_names,
            all_bytes,
        })
    }
}

impl WriteTable for NameTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::NAME
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(self.all_bytes);
    }
}
