//! `hhea` table support.

use super::Cursor;
use crate::{
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct HheaTable<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) number_of_h_metrics: u16,
}

impl<'a> HheaTable<'a> {
    pub(crate) const EXPECTED_LEN: usize = 36; // 18 words as per spec

    pub(super) fn parse(cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let bytes = cursor.bytes();
        if bytes.len() != Self::EXPECTED_LEN {
            return Err(cursor.err(ParseErrorKind::UnexpectedTableLen {
                expected: Self::EXPECTED_LEN,
                actual: bytes.len(),
            }));
        }
        let number_of_h_metrics =
            u16::from_be_bytes([bytes[Self::EXPECTED_LEN - 2], bytes[Self::EXPECTED_LEN - 1]]);
        Ok(Self {
            raw: bytes,
            number_of_h_metrics,
        })
    }
}

impl WriteTable for HheaTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::HHEA
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&self.raw[..Self::EXPECTED_LEN - 2]);
        buffer.write_u16(self.number_of_h_metrics);
    }
}
