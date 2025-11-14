//! OS/2 table parsing.

use super::types::Cursor;
use crate::{ParseError, ParseErrorKind};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Embedding {
    Installable,
    RestrictedLicense,
    PreviewAndPrint,
    Editable,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // FIXME: expose from `Font`?
pub(crate) struct UsagePermissions {
    pub(crate) raw: u16,
    pub(crate) embedding: Embedding,
    pub(crate) embed_only_bitmaps: bool,
    pub(crate) can_subset: bool,
}

impl UsagePermissions {
    fn parse(cursor: &mut Cursor<'_>) -> Result<Self, ParseError> {
        const EMBEDDING_MASK: u16 = 0x0f;
        const SUBSETTING_MASK: u16 = 0x0100;
        const EMBED_BITMAPS_MASK: u16 = 0x0200;

        cursor.read_u16_checked(|raw| {
            let raw_embedding = raw & EMBEDDING_MASK;
            let embedding = match raw_embedding {
                0 => Embedding::Installable,
                2 => Embedding::RestrictedLicense,
                4 => Embedding::PreviewAndPrint,
                8 => Embedding::Editable,
                _ => {
                    return Err(ParseErrorKind::UnexpectedValue {
                        name: "usage_permissions",
                        expected: "one of 0, 2, 4, or 8".into(),
                        actual: raw_embedding.into(),
                    })
                }
            };

            let can_subset = raw & SUBSETTING_MASK == 0;
            let embed_only_bitmaps = raw & EMBED_BITMAPS_MASK != 0;

            Ok(Self {
                raw,
                embedding,
                embed_only_bitmaps,
                can_subset,
            })
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Os2Table<'a> {
    pub(crate) version: u16,
    /// xAvgCharWidth, usWeightClass, usWidthClass
    pub(crate) not_parsed_after_version: [u8; 6],
    pub(crate) usage_permissions: UsagePermissions,
    /// ySubscriptXSize ..= PANOSE
    pub(crate) not_parsed_after_permissions: [u8; 32],
    pub(crate) unicode_ranges: u128,
    /// achVendID, fsSelection
    pub(crate) not_parsed_after_unicode_ranges: [u8; 6],
    pub(crate) first_char_index: u16,
    pub(crate) last_char_index: u16,
    /// sTypoAscender ..= usWinDescent
    pub(crate) not_parsed_after_char_index: [u8; 10],
    pub(crate) code_page_ranges: u64,
    pub(crate) not_parsed_tail: &'a [u8],
}

impl<'a> Os2Table<'a> {
    pub(super) fn parse(mut cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let version = cursor.read_u16_checked(|version| {
            if !(2..=5).contains(&version) {
                return Err(ParseErrorKind::UnexpectedValue {
                    name: "version",
                    expected: "value between 2 and 5".into(),
                    actual: version.into(),
                });
            }
            Ok(version)
        })?;

        let not_parsed_after_version = cursor.read_byte_array::<6>()?;
        let usage_permissions = UsagePermissions::parse(&mut cursor)?;
        let not_parsed_after_permissions = cursor.read_byte_array::<32>()?;
        let unicode_ranges = cursor.read_u128()?;
        let not_parsed_after_unicode_ranges = cursor.read_byte_array::<6>()?;
        let first_char_index = cursor.read_u16()?;
        let last_char_index = cursor.read_u16()?;
        let not_parsed_after_char_index = cursor.read_byte_array::<10>()?;
        let code_page_ranges = cursor.read_u64()?;

        Ok(Self {
            version,
            not_parsed_after_version,
            usage_permissions,
            not_parsed_after_permissions,
            unicode_ranges,
            not_parsed_after_unicode_ranges,
            first_char_index,
            last_char_index,
            not_parsed_after_char_index,
            code_page_ranges,
            not_parsed_tail: cursor.bytes(),
        })
    }
}
