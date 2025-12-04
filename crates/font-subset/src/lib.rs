//! OpenType font subsetting.
//!
//! This is a simple, no-std-compatible library that provides OpenType font *subsetting*, i.e.,
//! retaining only glyphs and other related data that correspond to specific chars. The subset can then be
//! saved in the OpenType (`.otf` / `.ttf`) or WOFF2 format.
//!
//! As an example, it is possible to subset visible ASCII chars (`' '..='~'`) from a font that originally supported
//! multiple languages. Subsetting may lead to significant space savings; e.g., a subset of Roboto (the standard
//! sans-serif font for Android) with visible ASCII chars occupies just 19 kB in the OpenType format
//! (and 11 kB in the WOFF2 format) vs the original 457 kB.
//!
//! The motivating use case for this library is embedding the produced font as a data URL in HTML or SVG,
//! so that it's guaranteed to be rendered in the same way across platforms.
//!
//! # Features
//!
//! - Can read and write fonts to/from OpenType and WOFF2 (the latter via an opt-in crate feature).
//! - Supports [variable fonts]. This allows to embed a continuous set of fonts across one or more dimensions, such as font weight
//!   or width, into a single file.
//! - Provides general info about fonts, e.g., font naming / license info and variation axis parameters.
//! - Single dependency for WOFF2 (de)compression; no-std compatible.
//!
//! # Design philosophy
//!
//! - **Keep parsing simple.** The library generally assumes that the input is well-formed, and defers parsing
//!   when possible. For example, simple glyph data is not parsed at all because it's not necessary for subsetting;
//!   it can be copied as opaque bytes.
//! - **Keep focus.** The library is focused on subsetting. For example, it doesn't strive to parse all tables from
//!   the OpenType spec.
//! - **Keep dependencies lean.** The library has the only opt-in dependency ([`brotli`]) to support WOFF2 serialization.
//!   The library is unconditionally no-std-compatible.
//!
//! # Known limitations
//!
//! - Subsetting drops advanced layout tables like `GPOS`, `kern` etc.
//! - Some table data (e.g., `maxp` fields like "maximum points in a non-composite glyph") are not updated
//!   in the subset font. Looks like some other subsetters (e.g., [`allsorts`]) do not update them either.
//!
//! # Alternatives and similar tools
//!
//! - [`allsorts`] is a library working with OpenType / WOFF / WOFF2 fonts, including
//!   their subsetting. It's more versatile, but at the cost of having more deps and requiring
//!   the standard library (although there is [a no-std fork](https://crates.io/crates/allsorts_no_std)).
//!   Its subsetting logic also produces fonts not parseable by browsers because of a missing `OS/2` table.
//! - [`subsetter`] is a specialized subsetting library. but it's geared towards PDF subsetting specifically
//!   (i.e., again not covering the motivating SVG use case).
//!
//! # Crate features
//!
//! ## `std`
//!
//! *(On by default)*
//!
//! Enables `std`-specific functionality, such as `Error` trait implementations for error types.
//!
//! ## `woff2`
//!
//! *(Off by default)*
//!
//! Enables writing fonts in the WOFF2 format.
//!
//! ## `tracing`
//!
//! *(Off by default)*
//!
//! Enables logging / tracing via the `tracing` facade, mostly on the `DEBUG` and `TRACE` levels.
//!
//! [variable fonts]: https://learn.microsoft.com/en-us/typography/opentype/spec/otvaroverview
//! [`brotli`]: https://crates.io/crates/brotli
//! [`allsorts`]: https://crates.io/crates/allsorts
//! [`subsetter`]: https://crates.io/crates/subsetter
//!
//! # Examples
//!
//! ## Subsetting
//!
//! ```
//! # use std::collections::BTreeSet;
//! use font_subset::{Font, FontReader};
//!
//! let font_bytes = // font in the OpenType or WOFF2 format
//! # include_bytes!("../examples/FiraMono-Regular.ttf");
//! // Create a font reader. This starts parsing and, in case of WOFF2,
//! // decompresses the font data.
//! let reader = FontReader::new(font_bytes)?;
//! // Parse the font from the reader.
//! let font: Font<'_> = reader.read()?;
//! // Ensure that the font license permits embedding and subsetting.
//! let permissions = font.permissions();
//! assert!(permissions.embedding.is_lenient());
//! assert!(permissions.allow_subsetting);
//!
//! let retained_chars: BTreeSet<char> = (' '..='~').collect();
//! // Create a subset.
//! let subset = font.subset(&retained_chars)?;
//! // Serialize the subset in OpenType and WOFF2 formats.
//! let ttf: Vec<u8> = subset.to_opentype();
//! println!("OpenType size: {}", ttf.len());
//! # assert!(ttf.len() < 20 * 1_024);
//!
//! let woff2: Vec<u8> = subset.to_woff2();
//! println!("WOFF2 size: {}", woff2.len());
//! # assert!(woff2.len() < 15 * 1_024);
//! # Ok::<_, font_subset::ParseError>(())
//! ```
//!
//! ## Getting info about font
//!
//! ```
//! use font_subset::OpenTypeReader;
//!
//! let font_bytes = // font in the OpenType format
//! # include_bytes!("../examples/RobotoMono-VariableFont_wght.ttf");
//! let font_reader = OpenTypeReader::new(font_bytes)?;
//! for (table, bytes) in font_reader.raw_tables() {
//!     println!("{table} length: {} B", bytes.len());
//! }
//!
//! // Parse the font to get table-specific info.
//! let font = font_reader.read()?;
//! println!("Name: {:?}", font.naming().family);
//! println!("Subfamily: {:?}", font.naming().subfamily);
//! println!("License: {:?}", font.naming().license);
//! let covered_chars: Vec<_> = font.char_ranges().collect();
//! println!("Covered chars: {covered_chars:?}");
//! println!("Glyph count: {}", font.glyph_count());
//!
//! let variation_axes = font.variation_axes().unwrap_or(&[]);
//! for axis in variation_axes {
//!     println!("Variation axis: {:?} [{}]", axis.name, axis.tag);
//!     println!(
//!         "Range: {:?} (default: {})",
//!         axis.min_value..=axis.max_value,
//!         axis.default_value
//!     );
//! }
//! # Ok::<_, font_subset::ParseError>(())
//! ```

#![cfg_attr(not(feature = "std"), no_std)]
// Documentation settings.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/font-subset/0.1.0")]

#[macro_use]
mod errors;
#[macro_use]
mod utils;
mod font;
mod subset;
#[cfg(test)]
mod testonly;
mod write;

mod alloc {
    #[cfg(not(feature = "std"))]
    extern crate alloc as std;

    pub(crate) use std::{
        borrow::Cow,
        boxed::Box,
        collections::{BTreeMap, BTreeSet},
        format,
        string::{String, ToString},
        vec,
        vec::Vec,
    };
}

#[cfg(feature = "woff2")]
pub use crate::font::Woff2Reader;
pub use crate::{
    errors::{ParseError, ParseErrorKind, Warning, WarningKind, Warnings},
    font::{
        EmbeddingPermissions, FileFormat, Fixed, Font, FontCategory, FontMetrics, FontNaming,
        FontReader, LongDateTime, OpenTypeReader, OwnedFont, TableTag, UsagePermissions,
        VariationAxis, VariationAxisTag,
    },
};

#[cfg(doctest)]
doc_comment::doctest!("../README.md");
