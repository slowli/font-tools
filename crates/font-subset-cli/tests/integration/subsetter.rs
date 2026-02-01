//! Simplified font subsetter used by `term-transcript`. We cannot use the standard one, because it would
//! create a circular cross-repository dependency.

use std::{collections::BTreeSet, fs};

use anyhow::Context;
use font_subset::OwnedFont;
use term_transcript::svg::{EmbeddedFont, EmbeddedFontFace, FontEmbedder, FontMetrics};

#[derive(Debug)]
pub(crate) struct SimpleSubsetter {
    font: OwnedFont,
}

impl SimpleSubsetter {
    pub(crate) fn roboto_mono() -> anyhow::Result<Self> {
        let font_bytes = fs::read(crate::ROBOTO_MONO_PATH)?;
        let font = OwnedFont::new(font_bytes.into())?;
        Ok(Self { font })
    }
}

impl FontEmbedder for SimpleSubsetter {
    type Error = anyhow::Error;

    fn embed_font(&self, mut used_chars: BTreeSet<char>) -> Result<EmbeddedFont, Self::Error> {
        let font = self.font.get();
        let metrics = font.metrics();
        used_chars.remove(&'\n');
        let missing_chars: Vec<_> = used_chars
            .iter()
            .copied()
            .filter(|&ch| !font.contains_char(ch))
            .collect();
        anyhow::ensure!(missing_chars.is_empty(), "missing chars: {missing_chars:?}");

        let subset_bytes = font
            .subset(&used_chars)
            .context("failed subsetting")?
            .to_woff2();

        Ok(EmbeddedFont {
            family_name: font.naming().family.context("no family name")?.to_owned(),
            metrics: FontMetrics {
                units_per_em: metrics.units_per_em,
                advance_width: metrics.monospace_advance_width.context("not monospace")?,
                ascent: metrics.ascent,
                descent: metrics.descent,
                bold_spacing: 0.0,
                italic_spacing: 0.0,
            },
            faces: vec![EmbeddedFontFace::woff2(subset_bytes)],
        })
    }
}
