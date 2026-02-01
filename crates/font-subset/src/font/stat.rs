//! `STAT` table handling.

use super::types::Cursor;
use crate::{
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct StatTable<'a> {
    design_axis_size: u16,
    design_axis_count: u16,
    design_axes: &'a [u8],
    axis_value_count: u16,
    elided_fallback_name_id: u16,
    axis_values: &'a [u8],
}

impl<'a> StatTable<'a> {
    const SAFE_FALLBACK_NAME_ID: u16 = 2;

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip_all, fields(range = ?cursor.range()))
    )]
    pub(super) fn parse(mut cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let full_cursor = cursor;

        cursor.read_u16_checked(|major_version| check_exact!(major_version, 1))?;
        let has_elided_fallback_name = cursor.read_u16_checked(|minor_version| {
            Ok(match minor_version {
                0 => false,
                1 | 2 => true,
                _ => {
                    return Err(ParseErrorKind::UnexpectedValue {
                        name: "minor_version",
                        expected: "0, 1 or 2".into(),
                        actual: minor_version.into(),
                    })
                }
            })
        })?;

        let design_axis_size = cursor.read_u16()?;
        let design_axis_count = cursor.read_u16_checked(|count| {
            if count == 0 {
                return Err(ParseErrorKind::UnexpectedValue {
                    name: "design_axis_count",
                    expected: "positive value".into(),
                    actual: 0,
                });
            }
            Ok(count)
        })?;
        let design_axes_offset = usize::try_from(cursor.read_u32()?).unwrap();
        let design_axes_len = usize::from(design_axis_size) * usize::from(design_axis_count);
        let design_axes = full_cursor
            .read_range(design_axes_offset..design_axes_offset + design_axes_len)?
            .bytes();

        let axis_value_count = cursor.read_u16()?;
        let offset_to_axis_values = usize::try_from(cursor.read_u32()?).unwrap();
        let mut axis_values_cursor = full_cursor;
        axis_values_cursor.skip(offset_to_axis_values)?;
        let axis_values = axis_values_cursor.bytes();

        let elided_fallback_name_id = if has_elided_fallback_name {
            cursor.read_u16()?
        } else {
            Self::SAFE_FALLBACK_NAME_ID
        };

        Ok(Self {
            design_axis_size,
            design_axis_count,
            design_axes,
            axis_value_count,
            elided_fallback_name_id,
            axis_values,
        })
    }

    /// Drops all axis values which are likely to occupy most space.
    pub(crate) fn subset(&mut self) {
        self.axis_value_count = 0;
        self.axis_values = &[];
    }
}

impl WriteTable for StatTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::STAT
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        const DESIGN_AXES_OFFSET: u32 = 20;

        let start_pos = buffer.len();
        buffer.write_u16(1); // major version
        buffer.write_u16(1); // minor version
        buffer.write_u16(self.design_axis_size);
        buffer.write_u16(self.design_axis_count);
        buffer.write_u32(DESIGN_AXES_OFFSET);
        buffer.write_u16(self.axis_value_count);
        let values_offset = if self.axis_values.is_empty() {
            0
        } else {
            DESIGN_AXES_OFFSET
                + u32::from(self.design_axis_size) * u32::from(self.design_axis_count)
        };
        buffer.write_u32(values_offset);
        buffer.write_u16(self.elided_fallback_name_id);
        debug_assert_eq!(
            buffer.len() - start_pos,
            usize::try_from(DESIGN_AXES_OFFSET).unwrap()
        );

        buffer.extend_from_slice(self.design_axes);
        if !self.axis_values.is_empty() {
            debug_assert_eq!(
                buffer.len() - start_pos,
                usize::try_from(values_offset).unwrap()
            );
            buffer.extend_from_slice(self.axis_values);
        }
    }
}

#[cfg(test)]
mod tests {
    use test_casing::test_casing;

    use super::*;
    use crate::{testonly::TestFont, OpenTypeReader};

    #[test_casing(3, TestFont::VAR)]
    fn full_table_roundtrip(font: TestFont) {
        let reader = OpenTypeReader::new(font.bytes).unwrap();
        let stat_cursor = reader
            .iter()
            .find_map(|(tag, cursor)| (tag == TableTag::STAT).then_some(cursor))
            .unwrap();
        let stat = StatTable::parse(stat_cursor).unwrap();

        let mut buffer = vec![];
        stat.write_to_vec(&mut buffer);
        assert_eq!(buffer, stat_cursor.bytes());
    }
}
