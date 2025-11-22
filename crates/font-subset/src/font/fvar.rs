//! `fvar` table processing.

use super::{
    types::{Cursor, Fixed},
    NameTable,
};
use crate::{
    alloc::{String, Vec},
    write::WriteTable,
    ParseError, TableTag,
};

/// Tag of a [`VariationAxis`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariationAxisTag([u8; 4]);

impl_tag!(VariationAxisTag);

impl VariationAxisTag {
    /// Commonly used "weight" axis tag.
    pub const WEIGHT: Self = Self(*b"wght");
    /// Commonly used "width" axis tag.
    pub const WIDTH: Self = Self(*b"wdth");
}

/// Information about a variation axis of a variable font.
#[derive(Debug, Clone)]
pub struct VariationAxis {
    /// Tag of this axis.
    pub tag: VariationAxisTag,
    /// Minimum allowed value for the axis.
    pub min_value: Fixed,
    /// Maximum allowed value for the axis.
    pub max_value: Fixed,
    /// Default value for the axis.
    pub default_value: Fixed,
    /// Resolved name of the axis read from the `name` table. `None` if the name uses an unsupported encoding.
    pub name: Option<String>,
    pub(super) name_id: u16,
}

impl VariationAxis {
    fn parse(cursor: &mut Cursor<'_>) -> Result<Self, ParseError> {
        let tag = VariationAxisTag(cursor.read_byte_array::<4>()?);
        let min_value = Fixed(cursor.read_i32()?);
        let default_value = Fixed(cursor.read_i32()?);
        let max_value = Fixed(cursor.read_i32()?);
        cursor.skip(2)?; // flags
        let name_id = cursor.read_u16()?;

        #[cfg(feature = "tracing")]
        tracing::debug!(
            ?tag,
            ?min_value,
            ?max_value,
            ?default_value,
            name_id,
            "parsed variation axis"
        );

        Ok(Self {
            tag,
            min_value,
            max_value,
            default_value,
            name: None,
            name_id,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FvarTable<'a> {
    axes: Vec<VariationAxis>,
    all_bytes: &'a [u8],
}

impl<'a> FvarTable<'a> {
    const VERSION: u32 = 0x0001_0000;
    const AXIS_SIZE: u16 = 20;

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip_all, fields(range = ?cursor.range()))
    )]
    pub(super) fn parse(mut cursor: Cursor<'a>) -> Result<Self, ParseError> {
        let all_bytes = cursor;

        cursor.read_u32_checked(|version| check_exact!(version, Self::VERSION))?;
        let axes_array_offset = cursor.read_u16()?;
        cursor.skip(2)?; // reserved
        let axis_count = cursor.read_u16()?;
        cursor.read_u16_checked(|axis_size| check_exact!(axis_size, Self::AXIS_SIZE))?;
        #[cfg(feature = "tracing")]
        tracing::debug!(axis_count, "read basic info");

        let mut axes_cursor = all_bytes;
        axes_cursor.skip(axes_array_offset.into())?;
        let axes = (0..axis_count)
            .map(|_| VariationAxis::parse(&mut axes_cursor))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            axes,
            all_bytes: all_bytes.bytes(),
        })
    }

    pub(super) fn axis_name_ids(&self) -> Vec<u16> {
        self.axes.iter().map(|axis| axis.name_id).collect()
    }

    pub(super) fn resolve_axe_names(&mut self, name: &NameTable<'_>) {
        for axis in &mut self.axes {
            axis.name = name.additional_names.get(&axis.name_id).cloned();
        }
    }

    pub(super) fn axes(&self) -> &[VariationAxis] {
        &self.axes
    }
}

impl WriteTable for FvarTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::FVAR
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(self.all_bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{testonly::TestFont, OpenTypeReader, TableTag};

    #[test]
    fn reading_fvar_table_with_2_axes() {
        let reader = OpenTypeReader::new(TestFont::ROBOTO.bytes).unwrap();
        let fvar = reader
            .iter()
            .find_map(|(tag, cursor)| (tag == TableTag::FVAR).then_some(cursor))
            .unwrap();
        let fvar = FvarTable::parse(fvar).unwrap();

        assert_eq!(fvar.axes.len(), 2);
        assert_eq!(fvar.axes[0].tag, VariationAxisTag::WEIGHT);
        assert_eq!(fvar.axes[0].min_value, 100_i16.into());
        assert_eq!(fvar.axes[0].max_value, 900_i16.into());
        assert_eq!(fvar.axes[0].default_value, 400_i16.into());
        assert_eq!(fvar.axes[1].tag, VariationAxisTag::WIDTH);
        assert_eq!(fvar.axes[1].min_value, 75_i16.into());
        assert_eq!(fvar.axes[1].max_value, 100_i16.into());
        assert_eq!(fvar.axes[1].default_value, 100_i16.into());
    }

    #[test]
    fn reading_fvar_table_with_1_axe() {
        let reader = OpenTypeReader::new(TestFont::ROBOTO_MONO.bytes).unwrap();
        let fvar = reader
            .iter()
            .find_map(|(tag, cursor)| (tag == TableTag::FVAR).then_some(cursor))
            .unwrap();
        let fvar = FvarTable::parse(fvar).unwrap();

        assert_eq!(fvar.axes.len(), 1);
        assert_eq!(fvar.axes[0].tag, VariationAxisTag::WEIGHT);
        assert_eq!(fvar.axes[0].min_value, 100_i16.into());
        assert_eq!(fvar.axes[0].max_value, 700_i16.into());
        assert_eq!(fvar.axes[0].default_value, 400_i16.into());
    }
}
