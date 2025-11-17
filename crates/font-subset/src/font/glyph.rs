//! `Glyph` and related types.

use super::types::{BoundingBox, Cursor};
use crate::{
    alloc::Vec,
    write::{VecExt, WriteTable},
    ParseError, TableTag,
};

#[derive(Debug, Clone)]
pub(crate) enum Glyph<'a> {
    Empty,
    Simple {
        bounding_box: BoundingBox,
        all_bytes: &'a [u8],
    },
    Composite {
        bounding_box: BoundingBox,
        components: Vec<GlyphComponent>,
        /// Optional instructions after the last component descriptor
        instructions: &'a [u8],
    },
}

impl<'a> Glyph<'a> {
    pub(super) fn new(raw: Cursor<'a>) -> Result<Self, ParseError> {
        if raw.bytes().is_empty() {
            return Ok(Self::Empty);
        }

        let mut cursor = raw;
        let number_of_contours = cursor.read_u16()?;
        let bounding_box = BoundingBox::parse(&mut cursor)?;
        if number_of_contours > i16::MAX as u16 {
            let mut has_more_components = true;
            let mut components = Vec::with_capacity(1);
            while has_more_components {
                let (component, new_has_more_components) = GlyphComponent::new(&mut cursor)?;
                components.push(component);
                has_more_components = new_has_more_components;
            }
            Ok(Self::Composite {
                bounding_box,
                components,
                instructions: cursor.bytes(),
            })
        } else {
            // Simple glyph
            Ok(Self::Simple {
                bounding_box,
                all_bytes: raw.bytes(),
            })
        }
    }

    /// Returns `None` for empty glyphs.
    pub(crate) fn bounding_box(&self) -> Option<BoundingBox> {
        match self {
            Self::Empty => None,
            Self::Simple { bounding_box, .. } | Self::Composite { bounding_box, .. } => {
                Some(*bounding_box)
            }
        }
    }

    pub(crate) fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        match self {
            Self::Empty => { /* do nothing */ }
            Self::Simple { all_bytes, .. } => {
                buffer.extend_from_slice(all_bytes);
            }
            Self::Composite {
                bounding_box,
                components,
                instructions,
            } => {
                buffer.write_u16(u16::MAX); // numberOfContours = -1
                bounding_box.write_to_vec(buffer);
                for component in components {
                    component.write_to_vec(buffer);
                }
                buffer.extend_from_slice(instructions);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GlyphComponent {
    pub(crate) flags: u16,
    pub(crate) glyph_idx: u16,
    pub(crate) args: GlyphComponentArgs,
    pub(crate) transform: TransformData,
}

impl GlyphComponent {
    fn new(cursor: &mut Cursor<'_>) -> Result<(Self, bool), ParseError> {
        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const WE_HAVE_A_SCALE: u16 = 0x008;
        const MORE_COMPONENTS: u16 = 0x0020;
        const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
        const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

        let flags = cursor.read_u16()?;
        let glyph_idx = cursor.read_u16()?;
        let args = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            GlyphComponentArgs::U32(cursor.read_u32()?)
        } else {
            GlyphComponentArgs::U16(cursor.read_u16()?)
        };
        let transform = if flags & WE_HAVE_A_SCALE != 0 {
            TransformData::Scale(cursor.read_u16()?)
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            TransformData::TwoScales([cursor.read_u16()?, cursor.read_u16()?])
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            TransformData::Affine([
                cursor.read_u16()?,
                cursor.read_u16()?,
                cursor.read_u16()?,
                cursor.read_u16()?,
            ])
        } else {
            TransformData::None
        };
        let this = Self {
            flags,
            glyph_idx,
            args,
            transform,
        };

        let has_more_components = flags & MORE_COMPONENTS != 0;
        Ok((this, has_more_components))
    }

    fn byte_len(&self) -> usize {
        let args_len = match self.args {
            GlyphComponentArgs::U16(_) => 2,
            GlyphComponentArgs::U32(_) => 4,
        };
        let transform_len = match self.transform {
            TransformData::None => 0,
            TransformData::Scale(_) => 2,
            TransformData::TwoScales(_) => 4,
            TransformData::Affine(_) => 8,
        };
        4 /* flags, glyph_idx */ + args_len + transform_len
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        buffer.write_u16(self.flags);
        buffer.write_u16(self.glyph_idx);
        match self.args {
            GlyphComponentArgs::U16(args) => buffer.write_u16(args),
            GlyphComponentArgs::U32(args) => buffer.write_u32(args),
        }
        match self.transform {
            TransformData::None => { /* do nothing */ }
            TransformData::Scale(val) => buffer.write_u16(val),
            TransformData::TwoScales([x, y]) => {
                buffer.write_u16(x);
                buffer.write_u16(y);
            }
            TransformData::Affine([xx, xy, yx, yy]) => {
                buffer.write_u16(xx);
                buffer.write_u16(xy);
                buffer.write_u16(yx);
                buffer.write_u16(yy);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum GlyphComponentArgs {
    U16(u16),
    U32(u32),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum TransformData {
    None,
    Scale(u16),
    TwoScales([u16; 2]),
    Affine([u16; 4]),
}

/// [`Glyph`] together with metrics read from the `hmtx` table.
#[derive(Debug, Clone)]
pub(crate) struct GlyphWithMetrics<'a> {
    pub(crate) inner: Glyph<'a>,
    pub(crate) advance: u16,
    pub(crate) lsb: i16,
}

#[derive(Debug, Clone)]
pub(crate) enum GlyfTable<'a> {
    Parsed(Cursor<'a>),
    Subset(Vec<GlyphWithMetrics<'a>>),
}

impl<'a> GlyfTable<'a> {
    /// Returns offsets into the `glyf` table (i.e., the contents of the `loca` table).
    pub(crate) fn compute_offsets(glyphs: &[GlyphWithMetrics<'a>]) -> Vec<usize> {
        let mut offsets = Vec::with_capacity(1 + glyphs.len());
        offsets.push(0);
        let mut len = 0;
        for glyph in glyphs {
            len += match &glyph.inner {
                Glyph::Empty => 0,
                Glyph::Simple { all_bytes, .. } => all_bytes.len(),
                Glyph::Composite {
                    components,
                    instructions,
                    ..
                } => {
                    let components_len = components
                        .iter()
                        .map(GlyphComponent::byte_len)
                        .sum::<usize>();
                    2 /* num_contours */ + BoundingBox::BYTE_LEN + components_len + instructions.len()
                }
            };
            offsets.push(len);
        }
        offsets
    }
}

impl WriteTable for GlyfTable<'_> {
    fn tag(&self) -> TableTag {
        TableTag::GLYF
    }

    fn write_to_vec(&self, buffer: &mut Vec<u8>) {
        match self {
            Self::Parsed(cursor) => {
                buffer.extend_from_slice(cursor.bytes());
            }
            Self::Subset(glyphs) => {
                for glyph in glyphs {
                    glyph.inner.write_to_vec(buffer);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use test_casing::test_casing;

    use super::*;
    use crate::{
        testonly::{TestFont, FONTS},
        Font,
    };

    #[test_casing(2, FONTS)]
    fn computed_offsets_are_correct(font: TestFont) {
        let font = Font::new(font.bytes).unwrap();
        let glyphs: Vec<_> = font
            .all_glyphs()
            .map(|res| res.unwrap().into_owned())
            .collect();
        let glyph_offsets = GlyfTable::compute_offsets(&glyphs);
        for (glyph_id, window) in glyph_offsets.windows(2).enumerate() {
            let &[start, end] = window else {
                unreachable!();
            };
            let range = font.loca.glyph_range(glyph_id.try_into().unwrap()).unwrap();
            assert_eq!(start..end, range);
        }
    }
}
