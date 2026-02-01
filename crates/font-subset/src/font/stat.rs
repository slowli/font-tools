//! `STAT` table handling.

use super::types::Cursor;
use crate::{
    alloc::Vec,
    write::{VecExt, WriteTable},
    ParseError, ParseErrorKind, TableTag, VariationAxis, VariationAxisTag,
};

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
struct AxisRecord {
    tag: VariationAxisTag,
    name_id: u16,
    ordering: u16,
}

impl AxisRecord {
    const SIZE: u16 = 8;

    fn parse(cursor: &mut Cursor<'_>) -> Result<Self, ParseError> {
        let tag = VariationAxisTag(cursor.read_byte_array::<4>()?);
        let name_id = cursor.read_u16()?;
        let ordering = cursor.read_u16()?;
        Ok(Self {
            tag,
            name_id,
            ordering,
        })
    }

    fn write_to_vec(self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&self.tag.0);
        buffer.write_u16(self.name_id);
        buffer.write_u16(self.ordering);
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StatTable<'a> {
    design_axes: Vec<AxisRecord>,
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

        cursor.read_u16_checked(|design_axis_size| {
            check_exact!(design_axis_size, AxisRecord::SIZE)
        })?;
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
        let design_axes_len = usize::from(AxisRecord::SIZE) * usize::from(design_axis_count);
        let mut design_axes_cursor =
            full_cursor.read_range(design_axes_offset..design_axes_offset + design_axes_len)?;
        let design_axes = (0..design_axis_count)
            .map(|_| AxisRecord::parse(&mut design_axes_cursor))
            .collect::<Result<Vec<_>, _>>()?;

        let axis_value_count = cursor.read_u16()?;
        let offset_to_axis_values = usize::try_from(cursor.read_u32()?).unwrap();
        let axis_values = if axis_value_count == 0 {
            &[]
        } else {
            let mut axis_values_cursor = full_cursor;
            axis_values_cursor.skip(offset_to_axis_values)?;
            axis_values_cursor.bytes()
        };

        let elided_fallback_name_id = if has_elided_fallback_name {
            cursor.read_u16()?
        } else {
            Self::SAFE_FALLBACK_NAME_ID
        };

        Ok(Self {
            design_axes,
            axis_value_count,
            elided_fallback_name_id,
            axis_values,
        })
    }

    /// Drops all axis values which are likely to occupy most space.
    pub(crate) fn subset(&mut self, var_axes: &[VariationAxis]) {
        self.design_axes.retain_mut(|design_axis| {
            let matching_var_axis = var_axes
                .iter()
                .find(|var_axis| var_axis.tag == design_axis.tag);
            let Some(matching_var_axis) = matching_var_axis else {
                return false; // trim the non-var axis
            };

            if matching_var_axis.name_id != design_axis.name_id {
                #[cfg(feature = "tracing")]
                tracing::warn!(
                    tag = %design_axis.tag,
                    matching_var_axis.name_id,
                    design_axis.name_id,
                    "name ID mismatch between fvar and STAT axes"
                );
                design_axis.name_id = matching_var_axis.name_id;
            }
            true
        });

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
        buffer.write_u16(AxisRecord::SIZE);
        let design_axis_count = self.design_axes.len().try_into().unwrap();
        buffer.write_u16(design_axis_count);
        buffer.write_u32(DESIGN_AXES_OFFSET);
        buffer.write_u16(self.axis_value_count);
        let values_offset = if self.axis_values.is_empty() {
            0
        } else {
            DESIGN_AXES_OFFSET + u32::from(AxisRecord::SIZE) * u32::from(design_axis_count)
        };
        buffer.write_u32(values_offset);
        buffer.write_u16(self.elided_fallback_name_id);
        debug_assert_eq!(
            buffer.len() - start_pos,
            usize::try_from(DESIGN_AXES_OFFSET).unwrap()
        );

        for design_axis in &self.design_axes {
            design_axis.write_to_vec(buffer);
        }

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
    use std::collections::HashSet;

    use test_casing::test_casing;

    use super::*;
    use crate::{font::FvarTable, testonly::TestFont, OpenTypeReader};

    #[test_casing(3, TestFont::VAR)]
    fn full_table_roundtrip(font: TestFont) {
        let reader = OpenTypeReader::new(font.bytes).unwrap();
        let stat_cursor = reader.table(TableTag::STAT);
        let stat = StatTable::parse(stat_cursor).unwrap();

        let mut buffer = vec![];
        stat.write_to_vec(&mut buffer);
        assert_eq!(buffer, stat_cursor.bytes());
    }

    #[test_casing(3, TestFont::VAR)]
    fn table_roundtrip_with_subsetting(font: TestFont) {
        let reader = OpenTypeReader::new(font.bytes).unwrap();
        let mut stat = StatTable::parse(reader.table(TableTag::STAT)).unwrap();
        let fvar = FvarTable::parse(reader.table(TableTag::FVAR)).unwrap();

        let var_axis_tags: HashSet<_> = fvar.axes().iter().map(|axis| axis.tag).collect();
        let design_axis_tags: HashSet<_> = stat.design_axes.iter().map(|axis| axis.tag).collect();
        assert!(
            var_axis_tags.is_subset(&design_axis_tags),
            "var_axis_tags={var_axis_tags:?}, design_axis_tags={design_axis_tags:?}"
        );

        stat.subset(fvar.axes());
        let mut buffer = vec![];
        stat.write_to_vec(&mut buffer);

        let restored_stat = StatTable::parse(Cursor::new(&buffer)).unwrap();
        assert_eq!(restored_stat, stat);
    }
}
