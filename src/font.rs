//! OpenType parsing logic.

use core::ops;

use crate::errors::{MapError, ParseError};

#[derive(Debug)]
enum CmapTableFormat {
    /// Segment mapping to delta values (format 4).
    SegmentDeltas,
    /// Segmented coverage (format 12).
    SegmentedCoverage,
}

#[derive(Debug)]
pub(crate) struct SegmentWithDelta {
    pub(crate) start_code: u16,
    pub(crate) end_code: u16,
    pub(crate) id_delta: u16,
    pub(crate) id_range_offset: u16,
}

/// Segment mapping to delta values (format 4) subtable of the `cmap` table.
#[derive(Debug)]
pub(crate) struct SegmentDeltas<'a> {
    pub(crate) segments: Vec<SegmentWithDelta>,
    pub(crate) glyph_id_array: &'a [u8],
}

impl<'a> SegmentDeltas<'a> {
    fn parse(mut bytes: &'a [u8]) -> Result<Self, ParseError> {
        let format = read_u16(&mut bytes)?;
        if format != 4 {
            return Err(ParseError::UnexpectedCmapTableFormat {
                expected: 4,
                actual: format,
            });
        }
        let subtable_len = read_u16(&mut bytes)?;
        let remaining_len = subtable_len
            .checked_sub(4)
            .ok_or(ParseError::UnexpectedEof)? as usize;
        if remaining_len > bytes.len() {
            return Err(ParseError::UnexpectedEof);
        }
        bytes = &bytes[..remaining_len];

        skip(&mut bytes, 2)?; // language
        let segment_count = read_u16(&mut bytes)? / 2;
        skip(&mut bytes, 6)?; // searchRange, entrySelector, rangeShift

        let vec_len = 2 * usize::from(segment_count);
        let mut end_codes = read_prefix(&mut bytes, vec_len)?;
        skip(&mut bytes, 2)?; // reserved padding
        let mut start_codes = read_prefix(&mut bytes, vec_len)?;
        let mut id_deltas = read_prefix(&mut bytes, vec_len)?;
        let mut id_range_offsets = read_prefix(&mut bytes, vec_len)?;

        let segments = (0..segment_count).map(|_| {
            Ok(SegmentWithDelta {
                start_code: read_u16(&mut start_codes)?,
                end_code: read_u16(&mut end_codes)?,
                id_delta: read_u16(&mut id_deltas)?,
                id_range_offset: read_u16(&mut id_range_offsets)?,
            })
        });

        Ok(Self {
            segments: segments.collect::<Result<_, ParseError>>()?,
            glyph_id_array: bytes,
        })
    }

    fn map_char(&self, c: char) -> Result<u16, MapError> {
        let c = u16::try_from(c as u32).map_err(|_| MapError::CharTooLarge)?;

        let segment_idx = self
            .segments
            .binary_search_by_key(&c, |segment| segment.end_code)
            .unwrap_or_else(|pos| pos);
        let segment = &self.segments[segment_idx];
        if segment.start_code > c {
            return Ok(0); // missing glyph
        }

        if segment.id_range_offset == 0 {
            Ok(segment.id_delta.wrapping_add(c))
        } else {
            // Offset is counted from the start of `idRangeOffsets`
            let mut byte_offset = 2 * segment_idx;
            byte_offset += usize::from(segment.id_range_offset);
            byte_offset += 2 * usize::from(c - segment.start_code);

            if byte_offset < 2 * self.segments.len() {
                return Err(MapError::InvalidOffset);
            }
            // Shift the offset to count from the start of `glyphIdArray`
            byte_offset -= 2 * self.segments.len();
            let glyph_id_bytes = self
                .glyph_id_array
                .get(byte_offset..(byte_offset + 2))
                .ok_or(MapError::InvalidOffset)?;
            let glyph_id = u16::from_be_bytes(glyph_id_bytes.try_into().unwrap());
            Ok(segment.id_delta.wrapping_add(glyph_id))
        }
    }
}

#[derive(Debug)]
pub(crate) struct SequentialMapGroup {
    pub(crate) start_char_code: u32,
    pub(crate) end_char_code: u32,
    pub(crate) start_glyph_id: u32,
}

impl SequentialMapGroup {
    pub(crate) fn map_unchecked(&self, ch: char) -> u32 {
        u32::from(ch) - self.start_char_code + self.start_glyph_id
    }
}

/// Segmented coverage (format 12) subtable of the `cmap` table.
#[derive(Debug, Default)]
pub(crate) struct SegmentedCoverage {
    pub(crate) groups: Vec<SequentialMapGroup>,
}

impl SegmentedCoverage {
    fn parse(mut bytes: &[u8]) -> Result<Self, ParseError> {
        let format = read_u16(&mut bytes)?;
        if format != 12 {
            return Err(ParseError::UnexpectedCmapTableFormat {
                expected: 12,
                actual: format,
            });
        }
        skip(&mut bytes, 2)?; // reserved

        let subtable_len = read_u32(&mut bytes)?;
        let remaining_len = subtable_len
            .checked_sub(8)
            .ok_or(ParseError::UnexpectedEof)? as usize;
        if remaining_len > bytes.len() {
            return Err(ParseError::UnexpectedEof);
        }
        bytes = &bytes[..remaining_len];

        skip(&mut bytes, 4)?; // language
        let num_groups = read_u32(&mut bytes)?;
        let groups = (0..num_groups).map(|_| {
            Ok(SequentialMapGroup {
                start_char_code: read_u32(&mut bytes)?,
                end_char_code: read_u32(&mut bytes)?,
                start_glyph_id: read_u32(&mut bytes)?,
            })
        });

        Ok(Self {
            groups: groups.collect::<Result<_, ParseError>>()?,
        })
    }
}

#[derive(Debug)]
pub(crate) struct CmapTable<'a> {
    pub(crate) segment_deltas: Option<SegmentDeltas<'a>>,
    pub(crate) segmented_coverage: Option<SegmentedCoverage>,
}

impl<'a> CmapTable<'a> {
    pub(crate) const UNICODE_PLATFORM: u16 = 0;
    const WINDOWS_PLATFORM: u16 = 3;

    fn parse(mut bytes: &'a [u8]) -> Result<Self, ParseError> {
        let table_bytes = bytes;
        let version = read_u16(&mut bytes)?;
        if version != 0 {
            return Err(ParseError::UnexpectedTableVersion {
                table: "cmap",
                version: version.into(),
            });
        }

        let num_tables = read_u16(&mut bytes)?;
        let (mut segment_deltas, mut segmented_coverage) = (None, None);
        for _ in 0..num_tables {
            let platform_id = read_u16(&mut bytes)?;
            let encoding_id = read_u16(&mut bytes)?;
            let offset = read_u32(&mut bytes)?;
            let expected_table_format = match (platform_id, encoding_id) {
                (Self::UNICODE_PLATFORM, 3) | (Self::WINDOWS_PLATFORM, 1) => {
                    CmapTableFormat::SegmentDeltas
                }
                (Self::UNICODE_PLATFORM, 4) | (Self::WINDOWS_PLATFORM, 10) => {
                    CmapTableFormat::SegmentedCoverage
                }
                _ => continue, // unsupported table format
            };

            match expected_table_format {
                CmapTableFormat::SegmentDeltas if segment_deltas.is_none() => {
                    let subtable_bytes = offset_bytes(table_bytes, offset)?;
                    segment_deltas = Some(SegmentDeltas::parse(subtable_bytes)?);
                }
                CmapTableFormat::SegmentedCoverage if segmented_coverage.is_none() => {
                    let subtable_bytes = offset_bytes(table_bytes, offset)?;
                    segmented_coverage = Some(SegmentedCoverage::parse(subtable_bytes)?);
                }
                _ => { /* We've already got a necessary table; do nothing */ }
            }
        }

        Ok(Self {
            segment_deltas,
            segmented_coverage,
        })
    }

    fn map_char(&self, ch: char) -> Result<u16, MapError> {
        // FIXME: incorrect in the general case
        self.segment_deltas.as_ref().unwrap().map_char(ch)
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
    fn new(raw: &'a [u8]) -> Result<Self, ParseError> {
        if raw.is_empty() {
            return Ok(Self::Empty);
        }

        let mut bytes = raw;
        let number_of_contours = read_u16(&mut bytes)?;
        if number_of_contours > i16::MAX as u16 {
            // Composite glyph
            let header = read_byte_array::<8>(&mut bytes)?;
            let mut has_more_components = true;
            let mut components = Vec::with_capacity(1);
            while has_more_components {
                let (component, new_has_more_components) = GlyphComponent::new(&mut bytes)?;
                components.push(component);
                has_more_components = new_has_more_components;
            }
            Ok(Self::Composite {
                header,
                components,
                instructions: bytes,
            })
        } else {
            // Simple glyph
            Ok(Self::Simple(raw))
        }
    }
}

#[derive(Debug)]
pub(crate) struct GlyphComponent {
    pub(crate) flags: u16,
    pub(crate) glyph_idx: u16,
    pub(crate) args: U16OrU32,
    pub(crate) transform: TransformData,
}

impl GlyphComponent {
    fn new(bytes: &mut &[u8]) -> Result<(Self, bool), ParseError> {
        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const WE_HAVE_A_SCALE: u16 = 0x008;
        const MORE_COMPONENTS: u16 = 0x0020;
        const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
        const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

        let flags = read_u16(bytes)?;
        let glyph_idx = read_u16(bytes)?;
        let args = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            U16OrU32::U32(read_u32(bytes)?)
        } else {
            U16OrU32::U16(read_u16(bytes)?)
        };
        let transform = if flags & WE_HAVE_A_SCALE != 0 {
            TransformData::Scale(read_u16(bytes)?)
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            TransformData::TwoScales([read_u16(bytes)?, read_u16(bytes)?])
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            TransformData::FourScales([
                read_u16(bytes)?,
                read_u16(bytes)?,
                read_u16(bytes)?,
                read_u16(bytes)?,
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
pub(crate) enum U16OrU32 {
    U16(u16),
    U32(u32),
}

#[derive(Debug)]
pub(crate) enum TransformData {
    None,
    Scale(u16),
    TwoScales([u16; 2]),
    FourScales([u16; 4]),
}

#[derive(Debug)]
pub(crate) struct GlyphWithMetrics<'a> {
    pub(crate) inner: Glyph<'a>,
    pub(crate) advance: u16,
    pub(crate) lsb: u16,
}
