//! OpenType parsing logic.

use core::ops;

pub(crate) use self::{
    cmap::{CmapTable, SegmentDeltas, SegmentWithDelta, SegmentedCoverage, SequentialMapGroup},
    glyph::{Glyph, GlyphComponent, GlyphWithMetrics, TransformData, GlyphComponentArgs},
};
use crate::errors::{MapError, ParseError};

mod cmap;
mod glyph;

fn skip(bytes: &mut &[u8], n: usize) -> Result<(), ParseError> {
    if bytes.len() < n {
        Err(ParseError::UnexpectedEof)
    } else {
        *bytes = &bytes[n..];
        Ok(())
    }
}

fn read_u16(bytes: &mut &[u8]) -> Result<u16, ParseError> {
    let [a, b, rest @ ..] = bytes else {
        return Err(ParseError::UnexpectedEof);
    };
    *bytes = rest;
    Ok(u16::from_be_bytes([*a, *b]))
}

fn read_u32(bytes: &mut &[u8]) -> Result<u32, ParseError> {
    let [a, b, c, d, rest @ ..] = bytes else {
        return Err(ParseError::UnexpectedEof);
    };
    *bytes = rest;
    Ok(u32::from_be_bytes([*a, *b, *c, *d]))
}

fn read_prefix<'a>(bytes: &mut &'a [u8], len: usize) -> Result<&'a [u8], ParseError> {
    if bytes.len() < len {
        Err(ParseError::UnexpectedEof)
    } else {
        let (head, tail) = bytes.split_at(len);
        *bytes = tail;
        Ok(head)
    }
}

fn read_byte_array<const N: usize>(bytes: &mut &[u8]) -> Result<[u8; N], ParseError> {
    if bytes.len() < N {
        Err(ParseError::UnexpectedEof)
    } else {
        let (head, tail) = bytes.split_at(N);
        *bytes = tail;
        Ok(head.try_into().unwrap())
    }
}

fn offset_bytes(bytes: &[u8], offset: u32) -> Result<&[u8], ParseError> {
    let offset = offset as usize;
    if bytes.len() < offset {
        Err(ParseError::UnexpectedEof)
    } else {
        Ok(&bytes[offset..])
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HheaTable<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) number_of_h_metrics: u16,
}

impl<'a> HheaTable<'a> {
    pub(crate) const EXPECTED_LEN: usize = 36; // 18 words as per spec

    fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        if bytes.len() != Self::EXPECTED_LEN {
            return Err(ParseError::UnexpectedTableLen {
                table: "hhea",
                expected: Self::EXPECTED_LEN,
                actual: bytes.len(),
            });
        }
        let number_of_h_metrics =
            u16::from_be_bytes([bytes[Self::EXPECTED_LEN - 2], bytes[Self::EXPECTED_LEN - 1]]);
        Ok(Self {
            raw: bytes,
            number_of_h_metrics,
        })
    }
}

#[derive(Debug)]
pub(crate) struct HmtxTable<'a> {
    raw: &'a [u8],
    number_of_h_metrics: u16,
}

impl HmtxTable<'_> {
    fn advance_and_lsb(&self, glyph_idx: u16) -> Result<(u16, u16), ParseError> {
        let (advance, lsb);
        if glyph_idx < self.number_of_h_metrics {
            let offset = u32::from(glyph_idx) * 4;
            let mut bytes = offset_bytes(self.raw, offset)?;
            advance = read_u16(&mut bytes)?;
            lsb = read_u16(&mut bytes)?;
        } else {
            let advance_offset = u32::from(self.number_of_h_metrics - 1) * 4;
            let mut bytes = offset_bytes(self.raw, advance_offset)?;
            advance = read_u16(&mut bytes)?;

            let lsb_offset = u32::from(self.number_of_h_metrics) * 4
                + u32::from(glyph_idx - self.number_of_h_metrics) * 2;
            let mut bytes = offset_bytes(self.raw, lsb_offset)?;
            lsb = read_u16(&mut bytes)?;
        }
        Ok((advance, lsb))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum LocaFormat {
    Short,
    Long,
}

impl LocaFormat {
    const fn bytes_per_offset(self) -> usize {
        match self {
            Self::Short => 2,
            Self::Long => 4,
        }
    }
}

#[derive(Debug)]
pub(crate) struct LocaTable<'a> {
    format: LocaFormat,
    bytes: &'a [u8],
}

impl<'a> LocaTable<'a> {
    fn new(format: LocaFormat, glyph_count: u16, bytes: &'a [u8]) -> Result<Self, ParseError> {
        let expected_len = format.bytes_per_offset() * (glyph_count as usize + 1);
        if bytes.len() != expected_len {
            Err(ParseError::UnexpectedTableLen {
                table: "loca",
                expected: expected_len,
                actual: bytes.len(),
            })
        } else {
            Ok(Self { format, bytes })
        }
    }

    fn glyph_range(&self, glyph_idx: u16) -> Result<ops::Range<usize>, ParseError> {
        let glyph_idx = usize::from(glyph_idx);
        Ok(match self.format {
            LocaFormat::Short => {
                let mut bytes = self.bytes;
                skip(&mut bytes, glyph_idx * 2)?;
                let start_offset = usize::from(read_u16(&mut bytes)?) * 2;
                let end_offset = usize::from(read_u16(&mut bytes)?) * 2;
                start_offset..end_offset
            }
            LocaFormat::Long => {
                let mut bytes = self.bytes;
                skip(&mut bytes, glyph_idx * 4)?;
                let start_offset = read_u32(&mut bytes)? as usize;
                let end_offset = read_u32(&mut bytes)? as usize;
                start_offset..end_offset
            }
        })
    }
}

#[derive(Debug)]
pub struct Font<'a> {
    pub(crate) cmap: CmapTable<'a>,
    pub(crate) head: &'a [u8],
    pub(crate) hhea: HheaTable<'a>,
    pub(crate) hmtx: HmtxTable<'a>,
    pub(crate) maxp: &'a [u8],
    pub(crate) name: &'a [u8],
    pub(crate) os2: &'a [u8],
    pub(crate) post: &'a [u8],
    pub(crate) loca: LocaTable<'a>,
    pub(crate) glyf: &'a [u8],
    pub(crate) cvt: Option<&'a [u8]>,
    pub(crate) fpgm: Option<&'a [u8]>,
    pub(crate) prep: Option<&'a [u8]>,
}

impl<'a> Font<'a> {
    pub(crate) const SNFT_VERSION: u32 = 0x_0001_0000;
    pub(crate) const CMAP_TAG: [u8; 4] = *b"cmap";
    pub(crate) const HEAD_TAG: [u8; 4] = *b"head";
    pub(crate) const HHEA_TAG: [u8; 4] = *b"hhea";
    pub(crate) const HMTX_TAG: [u8; 4] = *b"hmtx";
    pub(crate) const MAXP_TAG: [u8; 4] = *b"maxp";
    pub(crate) const NAME_TAG: [u8; 4] = *b"name";
    pub(crate) const OS2_TAG: [u8; 4] = *b"OS/2";
    pub(crate) const POST_TAG: [u8; 4] = *b"post";
    pub(crate) const LOCA_TAG: [u8; 4] = *b"loca";
    pub(crate) const GLYF_TAG: [u8; 4] = *b"glyf";
    pub(crate) const CVT_TAG: [u8; 4] = *b"cvt ";
    pub(crate) const FPGM_TAG: [u8; 4] = *b"fpgm";
    pub(crate) const PREP_TAG: [u8; 4] = *b"prep";

    pub fn parse(mut bytes: &'a [u8]) -> Result<Self, ParseError> {
        let font_bytes = bytes;
        let snft_version = read_u32(&mut bytes)?;
        if snft_version != Self::SNFT_VERSION {
            return Err(ParseError::UnexpectedFontVersion);
        }
        let table_count = read_u16(&mut bytes)?;
        skip(&mut bytes, 6)?; // searchRange, entrySelector, rangeShift

        let table_records =
            (0..table_count).map(|_| Self::parse_table_record(&mut bytes, font_bytes));

        let (mut cmap, mut head, mut hhea, mut maxp, mut hmtx) = (None, None, None, None, None);
        let (mut name, mut os2, mut post, mut loca, mut glyf) = (None, None, None, None, None);
        let (mut cvt, mut fpgm, mut prep) = (None, None, None);
        for record in table_records {
            let (tag, table_bytes) = record?;
            match tag.to_be_bytes() {
                Self::CMAP_TAG => {
                    cmap = Some(CmapTable::parse(table_bytes)?);
                }
                Self::HEAD_TAG => head = Some(table_bytes),
                Self::HHEA_TAG => hhea = Some(HheaTable::parse(table_bytes)?),
                Self::HMTX_TAG => hmtx = Some(table_bytes),
                Self::MAXP_TAG => maxp = Some(table_bytes),
                Self::NAME_TAG => name = Some(table_bytes),
                Self::OS2_TAG => os2 = Some(table_bytes),
                Self::POST_TAG => post = Some(table_bytes),
                Self::LOCA_TAG => loca = Some(table_bytes),
                Self::GLYF_TAG => glyf = Some(table_bytes),
                Self::CVT_TAG => cvt = Some(table_bytes),
                Self::FPGM_TAG => fpgm = Some(table_bytes),
                Self::PREP_TAG => prep = Some(table_bytes),
                _ => { /* skip table */ }
            }
        }

        let head = head.ok_or(ParseError::MissingTable("head"))?;
        let loca_format = Self::parse_loca_format(head)?;
        let maxp = maxp.ok_or(ParseError::MissingTable("maxp"))?;
        let glyph_count = Self::parse_glyph_count(maxp)?;
        let loca = loca.ok_or(ParseError::MissingTable("loca"))?;
        let loca = LocaTable::new(loca_format, glyph_count, loca)?;
        let hhea = hhea.ok_or(ParseError::MissingTable("hhea"))?;
        let hmtx = HmtxTable {
            raw: hmtx.ok_or(ParseError::MissingTable("hmtx"))?,
            number_of_h_metrics: hhea.number_of_h_metrics,
        };

        Ok(Self {
            cmap: cmap.ok_or(ParseError::MissingTable("cmap"))?,
            head,
            hhea,
            hmtx,
            maxp,
            name: name.ok_or(ParseError::MissingTable("name"))?,
            os2: os2.ok_or(ParseError::MissingTable("OS/2"))?,
            post: post.ok_or(ParseError::MissingTable("post"))?,
            loca,
            glyf: glyf.ok_or(ParseError::MissingTable("glyf"))?,
            cvt,
            fpgm,
            prep,
        })
    }

    fn parse_table_record(
        header_bytes: &mut &[u8],
        font_bytes: &'a [u8],
    ) -> Result<(u32, &'a [u8]), ParseError> {
        let tag = read_u32(header_bytes)?;
        skip(header_bytes, 4)?; // checksum
        let offset = read_u32(header_bytes)? as usize;
        let len = read_u32(header_bytes)? as usize;
        let table_bytes = font_bytes
            .get(offset..(offset + len))
            .ok_or(ParseError::UnexpectedEof)?;
        Ok((tag, table_bytes))
    }

    fn parse_loca_format(mut head_bytes: &[u8]) -> Result<LocaFormat, ParseError> {
        let version = read_u32(&mut head_bytes)?;
        if version != 0x_0001_0000 {
            return Err(ParseError::UnexpectedTableVersion {
                table: "head",
                version,
            });
        }
        skip(&mut head_bytes, 46)?;
        // ^ fontRevision, checksumAdjustment, magicNumber, flags, unitsPerEm, created, modified,
        // bounding box, macStyle, lowestRecPPEM, fontDirectionHint

        let raw_format = read_u16(&mut head_bytes)?;
        match raw_format {
            0 => Ok(LocaFormat::Short),
            1 => Ok(LocaFormat::Long),
            _ => Err(ParseError::UnexpectedLocaFormat(raw_format)),
        }
    }

    fn parse_glyph_count(mut maxp_bytes: &[u8]) -> Result<u16, ParseError> {
        let version = read_u32(&mut maxp_bytes)?;
        if version != 0x_0000_5000 && version != 0x_0001_0000 {
            return Err(ParseError::UnexpectedTableVersion {
                table: "maxp",
                version,
            });
        }
        read_u16(&mut maxp_bytes)
    }

    pub(crate) fn map_char(&self, ch: char) -> Result<u16, MapError> {
        self.cmap.map_char(ch)
    }

    pub(crate) fn glyph(&self, glyph_idx: u16) -> Result<GlyphWithMetrics<'a>, ParseError> {
        let range = self.loca.glyph_range(glyph_idx)?;
        let raw = self
            .glyf
            .get(range.clone())
            .ok_or(ParseError::MissingGlyph { glyph_idx, range })?;
        let inner = Glyph::new(raw)?;
        let (advance, lsb) = self.hmtx.advance_and_lsb(glyph_idx)?;
        Ok(GlyphWithMetrics {
            inner,
            advance,
            lsb,
        })
    }
}
