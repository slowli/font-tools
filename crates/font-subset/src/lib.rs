//! OpenType font subsetting.

#![cfg_attr(not(feature = "std"), no_std)]

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
