//! OpenType font subsetting.
//!
//! # Examples
//!
//! ```
//! # use std::collections::BTreeSet;
//! use font_subset::Font;
//!
//! let font_bytes = // font in the OpenType format
//! # include_bytes!("../examples/FiraMono-Regular.ttf");
//! // Parse the font.
//! let font = Font::new(font_bytes)?;
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

#![cfg_attr(not(feature = "std"), no_std)]
// Documentation settings.
#![doc(html_root_url = "https://docs.rs/font-subset/0.1.0")]

mod errors;
mod font;
mod subset;
#[cfg(test)]
pub(crate) mod tests;
mod write;

mod alloc {
    #[cfg(not(feature = "std"))]
    extern crate alloc as std;

    pub(crate) use std::{
        boxed::Box,
        collections::{BTreeMap, BTreeSet},
        vec,
        vec::Vec,
    };
}

pub use crate::{
    errors::{ParseError, ParseErrorKind},
    font::{Font, TableTag},
    subset::FontSubset,
};

#[cfg(doctest)]
doc_comment::doctest!("../README.md");
