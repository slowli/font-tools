//! `post` table processing.

use super::types::Cursor;
use crate::{
    alloc::Vec,
    write::{VecExt, WriteTable},
    TableTag,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PostTable<'a> {
    raw: Cursor<'a>,
    is_subset: bool,
}

impl<'a> PostTable<'a> {
    pub(super) fn new(raw: Cursor<'a>) -> Self {
        Self {
            raw,
            is_subset: false,
        }
    }

    pub(crate) fn subset(&mut self) {
        self.is_subset = true;
    }
}

impl WriteTable for PostTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::POST
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        if self.is_subset {
            // Truncate the `post` table to not contain glyph names
            buffer.write_u32(0x_0003_0000); // version
            buffer.extend_from_slice(&self.raw.bytes()[4..32]);
        } else {
            buffer.extend_from_slice(self.raw.bytes());
        }
    }
}
