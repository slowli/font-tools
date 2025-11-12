//! `Glyph` and related types.

use super::Cursor;
use crate::ParseError;

#[derive(Debug)]
pub(crate) enum Glyph<'a> {
    Empty,
    Simple(&'a [u8]),
    Composite {
        /// xMin, yMin, xMax, yMax
        header: [u8; 8],
        components: Vec<GlyphComponent>,
        /// Optional instructions after the last component descriptor
        instructions: &'a [u8],
    },
}

impl<'a> Glyph<'a> {
    pub(super) fn new(raw: Cursor<'a>) -> Result<Self, ParseError> {
        if raw.bytes.is_empty() {
            return Ok(Self::Empty);
        }

        let mut cursor = raw;
        let number_of_contours = cursor.read_u16()?;
        if number_of_contours > i16::MAX as u16 {
            // Composite glyph
            let header = cursor.read_byte_array::<8>()?;
            let mut has_more_components = true;
            let mut components = Vec::with_capacity(1);
            while has_more_components {
                let (component, new_has_more_components) = GlyphComponent::new(&mut cursor)?;
                components.push(component);
                has_more_components = new_has_more_components;
            }
            Ok(Self::Composite {
                header,
                components,
                instructions: cursor.bytes,
            })
        } else {
            // Simple glyph
            Ok(Self::Simple(raw.bytes))
        }
    }
}

#[derive(Debug)]
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
}

#[derive(Debug)]
pub(crate) enum GlyphComponentArgs {
    U16(u16),
    U32(u32),
}

#[derive(Debug)]
pub(crate) enum TransformData {
    None,
    Scale(u16),
    TwoScales([u16; 2]),
    Affine([u16; 4]),
}

/// [`Glyph`] together with metrics read from the `hmtx` table.
#[derive(Debug)]
pub(crate) struct GlyphWithMetrics<'a> {
    pub(crate) inner: Glyph<'a>,
    pub(crate) advance: u16,
    pub(crate) lsb: u16,
}
