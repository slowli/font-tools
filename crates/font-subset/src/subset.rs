use crate::{
    alloc::{vec, BTreeMap, BTreeSet, Vec},
    font::{Font, Glyph, GlyphWithMetrics},
    ParseError,
};

/// Subset of a [`Font`] produced by removing some of its glyphs and related data.
#[derive(Debug)]
pub struct FontSubset<'a> {
    pub(crate) font: Font<'a>,
    pub(crate) char_map: Vec<(char, u16)>,
    pub(crate) old_to_new_glyph_idx: BTreeMap<u16, u16>,
    pub(crate) glyphs: Vec<GlyphWithMetrics<'a>>,
}

impl<'a> FontSubset<'a> {
    pub(crate) fn new(font: Font<'a>, distinct_chars: &BTreeSet<char>) -> Result<Self, ParseError> {
        let mut this = Self::empty(font)?;
        for &ch in distinct_chars {
            this.push_char(ch)?;
        }
        Ok(this)
    }

    fn empty(font: Font<'a>) -> Result<Self, ParseError> {
        let empty_glyph = font.glyph(0)?;
        Ok(Self {
            font,
            char_map: vec![],
            // The 0th glyph must always be mapped to itself
            old_to_new_glyph_idx: BTreeMap::from([(0, 0)]),
            glyphs: vec![empty_glyph],
        })
    }

    fn ensure_glyph(&mut self, old_idx: u16) -> Result<u16, ParseError> {
        if let Some(new_idx) = self.old_to_new_glyph_idx.get(&old_idx) {
            return Ok(*new_idx);
        }

        let mut glyph = self.font.glyph(old_idx)?;
        match &mut glyph.inner {
            Glyph::Empty | Glyph::Simple(_) => { /* do not transform the glyph */ }
            Glyph::Composite { components, .. } => {
                for component in components {
                    component.glyph_idx = self.ensure_glyph(component.glyph_idx)?;
                }
            }
        }

        let new_idx = u16::try_from(self.glyphs.len()).expect("too many glyphs");
        self.glyphs.push(glyph);
        self.old_to_new_glyph_idx.insert(old_idx, new_idx);
        Ok(new_idx)
    }

    /// Must be called with increasing `ch`.
    fn push_char(&mut self, ch: char) -> Result<(), ParseError> {
        let old_idx = self.font.map_char(ch)?;
        let new_idx = self.ensure_glyph(old_idx)?;
        self.char_map.push((ch, new_idx));
        Ok(())
    }
}
