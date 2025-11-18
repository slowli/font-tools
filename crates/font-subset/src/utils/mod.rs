//! Miscellaneous utils.

use core::ops;

#[cfg(feature = "woff2")]
pub(crate) mod brotli;

macro_rules! impl_tag {
    ($tag_type:ident) => {
        impl ::core::fmt::Debug for $tag_type {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                if let Ok(s) = ::core::str::from_utf8(&self.0) {
                    ::core::fmt::Debug::fmt(&s, formatter)
                } else {
                    ::core::write!(formatter, "0x{:x}", u32::from_be_bytes(self.0))
                }
            }
        }

        impl ::core::fmt::Display for $tag_type {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                if let Ok(s) = ::core::str::from_utf8(&self.0) {
                    ::core::fmt::Display::fmt(&s, formatter)
                } else {
                    ::core::write!(formatter, "0x{:x}", u32::from_be_bytes(self.0))
                }
            }
        }

        impl From<u32> for $tag_type {
            fn from(val: u32) -> Self {
                Self(val.to_be_bytes())
            }
        }
    };
}

/// Returns a Unicode char with the next numeric code.
pub(crate) fn next_char_code(ch: char) -> Option<char> {
    char::try_from(u32::from(ch) + 1).ok()
}

#[derive(Debug, Clone)]
pub(crate) enum Either<A, B> {
    Left(A),
    Right(B),
}

impl<A, B> Iterator for Either<A, B>
where
    A: Iterator,
    B: Iterator<Item = A::Item>,
{
    type Item = A::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Left(it) => it.next(),
            Self::Right(it) => it.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Left(it) => it.size_hint(),
            Self::Right(it) => it.size_hint(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct RangeConcat<I> {
    inner: I,
    buffered: Option<ops::RangeInclusive<char>>,
}

impl<I> RangeConcat<I>
where
    I: Iterator<Item = ops::RangeInclusive<char>>,
{
    pub(crate) fn new(inner: I) -> Self {
        Self {
            inner,
            buffered: None,
        }
    }
}

impl<I> Iterator for RangeConcat<I>
where
    I: Iterator<Item = ops::RangeInclusive<char>>,
{
    type Item = ops::RangeInclusive<char>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let next = self.inner.next();
            if let Some(next) = next {
                if let Some(buffered) = self.buffered.take() {
                    if next_char_code(*buffered.end()) == Some(*next.start()) {
                        // Concatenate ranges and continue the loop.
                        self.buffered = Some(*buffered.start()..=*next.end());
                    } else {
                        // There's a gap; swap to the next range and return the buffered one.
                        self.buffered = Some(next);
                        return Some(buffered);
                    }
                } else {
                    self.buffered = Some(next);
                }
            } else {
                return self.buffered.take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_char_code_works() {
        assert_eq!(next_char_code('a'), Some('b'));
        assert_eq!(next_char_code('ф'), Some('х'));
        assert_eq!(next_char_code('\u{d7ff}'), None);
        assert_eq!(next_char_code(char::MAX), None);
    }

    #[test]
    fn concatenating_ranges() {
        let concat: Vec<_> = RangeConcat::new(['a'..='a'].into_iter()).collect();
        assert_eq!(concat, ['a'..='a']);

        let concat: Vec<_> = RangeConcat::new(['a'..='a', 'b'..='c'].into_iter()).collect();
        assert_eq!(concat, ['a'..='c']);

        let concat: Vec<_> = RangeConcat::new(['a'..='a', 'c'..='f'].into_iter()).collect();
        assert_eq!(concat, ['a'..='a', 'c'..='f']);

        let concat: Vec<_> =
            RangeConcat::new(['a'..='a', 'b'..='e', 'g'..='g', 'h'..='i'].into_iter()).collect();
        assert_eq!(concat, ['a'..='e', 'g'..='i']);
    }
}
