//! OpenType font subsetting.

mod errors;
mod font;
mod subset;
#[cfg(test)]
mod tests;
mod write;

pub use crate::{
    errors::{MapError, ParseError, ParseErrorKind},
    font::{Font, TableTag},
    subset::FontSubset,
};
