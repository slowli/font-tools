use super::Cursor;
use crate::{
    alloc::Vec,
    write::{VecExt, WriteTable},
    ParseError, TableTag,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct MaxpTable<'a> {
    pub(crate) glyph_count: u16,
    pub(crate) unparsed_tail: &'a [u8],
}

impl<'a> MaxpTable<'a> {
    pub(crate) const VERSION: u32 = 0x_0001_0000;

    pub(super) fn parse(mut cursor: Cursor<'a>) -> Result<Self, ParseError> {
        cursor.read_u32_checked(|version| check_exact!(version, Self::VERSION))?;
        let glyph_count = cursor.read_u16()?;
        Ok(Self {
            glyph_count,
            unparsed_tail: cursor.bytes(),
        })
    }

    pub(crate) fn subset(&mut self, glyph_count: u16) {
        self.glyph_count = glyph_count;
    }
}

impl WriteTable for MaxpTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::MAXP
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.write_u32(Self::VERSION);
        buffer.write_u16(self.glyph_count);
        buffer.extend_from_slice(self.unparsed_tail);
    }
}
