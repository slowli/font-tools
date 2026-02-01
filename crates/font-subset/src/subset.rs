use core::ops;

use crate::{
    alloc::{vec, BTreeMap, BTreeSet, Vec},
    font::{
        CmapTable, Font, GlyfTable, Glyph, GlyphWithMetrics, HmtxTable, LocaTable,
        VariableFontTables,
    },
    ParseError, TableTag,
};

/// Subset of a [`Font`] produced by removing some of its glyphs and related data.
#[derive(Debug)]
pub(crate) struct FontSubset<'a> {
    char_map: Vec<(char, u16)>,
    old_to_new_glyph_idx: BTreeMap<u16, u16>,
    old_glyph_ids: Vec<u16>,
    glyphs: Vec<GlyphWithMetrics<'a>>,
}

impl<'a> FontSubset<'a> {
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", err, skip_all, fields(chars.len = distinct_chars.len()))
    )]
    pub(crate) fn subset(
        font: &Font<'a>,
        distinct_chars: &BTreeSet<char>,
    ) -> Result<Font<'a>, ParseError> {
        let mut this = Self::empty(font)?;
        for &ch in distinct_chars {
            this.push_char(font, ch)?;
        }
        this.build(font)
    }

    fn empty(font: &Font<'a>) -> Result<Self, ParseError> {
        let empty_glyph = font.glyph(0)?;
        Ok(Self {
            char_map: vec![],
            // The 0th glyph must always be mapped to itself
            old_to_new_glyph_idx: BTreeMap::from([(0, 0)]),
            old_glyph_ids: vec![0],
            glyphs: vec![empty_glyph],
        })
    }

    fn ensure_glyph(&mut self, font: &Font<'a>, old_idx: u16) -> Result<u16, ParseError> {
        if let Some(new_idx) = self.old_to_new_glyph_idx.get(&old_idx) {
            return Ok(*new_idx);
        }

        let mut glyph = font.glyph(old_idx)?;
        match &mut glyph.inner {
            Glyph::Empty | Glyph::Simple { .. } => { /* do not transform the glyph */ }
            Glyph::Composite { components, .. } => {
                #[cfg(feature = "tracing")]
                tracing::trace!(
                    old_idx,
                    components.len = components.len(),
                    "recursing into composite glyph"
                );

                for component in components {
                    component.glyph_idx = self.ensure_glyph(font, component.glyph_idx)?;
                }
            }
        }

        let new_idx = u16::try_from(self.glyphs.len()).expect("too many glyphs");
        self.glyphs.push(glyph);
        self.old_to_new_glyph_idx.insert(old_idx, new_idx);
        self.old_glyph_ids.push(old_idx);

        #[cfg(feature = "tracing")]
        tracing::trace!(old_idx, new_idx, "pushed new glyph");
        Ok(new_idx)
    }

    /// Must be called with increasing `ch`.
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "trace", err, skip_all, fields(ch = ?ch))
    )]
    fn push_char(&mut self, font: &Font<'a>, ch: char) -> Result<(), ParseError> {
        let old_idx = font.map_char(ch)?;
        let new_idx = self.ensure_glyph(font, old_idx)?;
        self.char_map.push((ch, new_idx));
        #[cfg(feature = "tracing")]
        tracing::trace!(old_idx, new_idx, "handled char");
        Ok(())
    }

    fn char_range(&self) -> ops::RangeInclusive<char> {
        let &(first, _) = self.char_map.first().expect("empty subset");
        let &(last, _) = self.char_map.last().expect("empty subset");
        first..=last
    }

    fn build(self, src: &Font<'a>) -> Result<Font<'a>, ParseError> {
        debug_assert_eq!(self.old_glyph_ids, {
            let mut ids: Vec<_> = self.old_to_new_glyph_idx.iter().collect();
            ids.sort_unstable_by_key(|(_, new_idx)| **new_idx);
            ids.into_iter()
                .map(|(old_idx, _)| *old_idx)
                .collect::<Vec<_>>()
        });

        let (hmtx, number_of_h_metrics) = HmtxTable::subset(&self.glyphs);
        let mut hhea = src.hhea;
        hhea.subset(&self.glyphs, number_of_h_metrics);

        let mut post = src.post;
        post.subset();

        let mut maxp = src.maxp;
        // `unwrap()` should be safe: the subset shouldn't contain >65536 glyphs because the original font doesn't.
        let glyph_count = u16::try_from(self.glyphs.len()).unwrap();
        maxp.subset(glyph_count);

        let mut os2 = src.os2;
        os2.subset(self.char_range());

        let glyph_offsets = GlyfTable::compute_offsets(&self.glyphs);
        let loca = LocaTable::subset(glyph_offsets);
        let mut head = src.head;
        head.subset(loca.format(), &self.glyphs);

        let mut name = src.name.clone();
        name.subset(true); // FIXME: make configurable?

        let variable = src
            .variable
            .as_ref()
            .map(|variable| {
                let unparsed: Vec<_> = variable
                    .unparsed
                    .iter()
                    .copied()
                    .filter(|(tag, _)| {
                        let retained = *tag == TableTag::AVAR;
                        #[cfg(feature = "tracing")]
                        tracing::debug!(?tag, retained, "filtered variation table");
                        retained
                    })
                    .collect();

                let mut fvar = variable.fvar.clone();
                fvar.subset();
                let mut stat = variable.stat;
                stat.subset();

                Ok(VariableFontTables {
                    fvar,
                    gvar: variable.gvar.subset(self.old_glyph_ids.iter().copied())?,
                    stat,
                    unparsed,
                })
            })
            .transpose()?;

        let unparsed = src
            .unparsed
            .iter()
            .copied()
            .filter(|(tag, _)| {
                let retained = matches!(*tag, TableTag::FPGM | TableTag::CVT | TableTag::PREP);
                #[cfg(feature = "tracing")]
                tracing::debug!(?tag, retained, "filtered table");
                retained
            })
            .collect();

        Ok(Font {
            cmap: CmapTable::from_map(&self.char_map),
            head,
            hhea,
            hmtx,
            maxp,
            name,
            os2,
            post,
            loca,
            glyf: GlyfTable::Subset(self.glyphs),
            variable,
            unparsed,
        })
    }
}
