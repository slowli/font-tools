//! OS/2 table parsing.

use core::ops;

use super::types::Cursor;
use crate::{
    alloc::Vec,
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag,
};

/// Embedding permissions recorded in the OS/2 font table.
///
/// See [the Microsoft docs](https://learn.microsoft.com/en-us/typography/opentype/spec/os2)
/// for details on what these permissions mean.
#[derive(Debug, Clone, Copy)]
pub enum EmbeddingPermissions {
    /// Installable embedding.
    Installable,
    /// Restricted license embedding.
    RestrictedLicense,
    /// Preview & print embedding.
    PreviewAndPrint,
    /// Editable embedding.
    Editable,
}

impl EmbeddingPermissions {
    /// Are these permissions lenient? [`Self::Installable`] and [`Self::Editable`] permissions are considered
    /// lenient, while the others are restrictive. YMMV depending on the use case,
    /// so be sure to consult the font license if in doubt.
    pub fn is_lenient(self) -> bool {
        matches!(self, Self::Installable | Self::Editable)
    }
}

/// Usage permissions for a font recorded in its OS/2 table.
#[derive(Debug, Clone, Copy)]
pub struct UsagePermissions {
    pub(crate) raw: u16,
    /// Embedding permissions.
    pub embedding: EmbeddingPermissions,
    /// If set, only bitmap embedding is allowed (as opposed to embedding bitmaps and outlines
    /// ordinarily encoded in font glyphs). If the font doesn't contain bitmaps, no embedding is allowed at all.
    pub embed_only_bitmaps: bool,
    /// If set, the font can be subset during embedding.
    pub allow_subsetting: bool,
}

impl UsagePermissions {
    fn parse(cursor: &mut Cursor<'_>) -> Result<Self, ParseError> {
        const EMBEDDING_MASK: u16 = 0x0f;
        const SUBSETTING_MASK: u16 = 0x0100;
        const EMBED_BITMAPS_MASK: u16 = 0x0200;

        cursor.read_u16_checked(|raw| {
            let raw_embedding = raw & EMBEDDING_MASK;
            let embedding = match raw_embedding {
                0 => EmbeddingPermissions::Installable,
                2 => EmbeddingPermissions::RestrictedLicense,
                4 => EmbeddingPermissions::PreviewAndPrint,
                8 => EmbeddingPermissions::Editable,
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
                allow_subsetting: can_subset,
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
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip(cursor), fields(range = ?cursor.range()))
    )]
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
        #[cfg(feature = "tracing")]
        tracing::debug!(version, "parsed table version");

        let not_parsed_after_version = cursor.read_byte_array::<6>()?;
        let usage_permissions = UsagePermissions::parse(&mut cursor)?;
        let not_parsed_after_permissions = cursor.read_byte_array::<32>()?;
        let unicode_ranges = cursor.read_u128()?;
        let not_parsed_after_unicode_ranges = cursor.read_byte_array::<6>()?;
        let first_char_index = cursor.read_u16()?;
        let last_char_index = cursor.read_u16()?;
        let not_parsed_after_char_index = cursor.read_byte_array::<10>()?;
        let code_page_ranges = cursor.read_u64()?;

        #[cfg(feature = "tracing")]
        tracing::debug!(
            ?usage_permissions,
            unicode_ranges,
            first_char_index,
            last_char_index,
            code_page_ranges,
            "parsed basic info"
        );

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

    pub(crate) fn subset(&mut self, char_range: ops::RangeInclusive<char>) {
        // Mark that the font doesn't support any specific Unicode / code page ranges. This is the safest option.
        self.unicode_ranges = 0;
        self.code_page_ranges = 0;

        self.first_char_index = u16::try_from(*char_range.start()).unwrap_or(u16::MAX);
        self.last_char_index = u16::try_from(*char_range.end()).unwrap_or(u16::MAX);
    }
}

impl WriteTable for Os2Table<'_> {
    fn tag(&self) -> TableTag {
        TableTag::OS2
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.write_u16(self.version);
        buffer.extend_from_slice(&self.not_parsed_after_version);
        buffer.write_u16(self.usage_permissions.raw);
        buffer.extend_from_slice(&self.not_parsed_after_permissions);
        buffer.extend_from_slice(&self.unicode_ranges.to_be_bytes());
        buffer.extend_from_slice(&self.not_parsed_after_unicode_ranges);
        buffer.write_u16(self.first_char_index);
        buffer.write_u16(self.last_char_index);
        buffer.extend_from_slice(&self.not_parsed_after_char_index);
        buffer.write_u64(self.code_page_ranges);
        buffer.extend_from_slice(self.not_parsed_tail);
    }
}
