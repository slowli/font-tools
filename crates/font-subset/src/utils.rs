//! Miscellaneous utils.

use core::ops;

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
    buffered: Option<ops::RangeInclusive<u32>>,
}

impl<I> RangeConcat<I>
where
    I: Iterator<Item = ops::RangeInclusive<u32>>,
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
    I: Iterator<Item = ops::RangeInclusive<u32>>,
{
    type Item = ops::RangeInclusive<u32>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let next = self.inner.next();
            if let Some(next) = next {
                if let Some(buffered) = self.buffered.take() {
                    if *buffered.end() + 1 == *next.start() {
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
    fn concatenating_ranges() {
        let concat: Vec<_> = RangeConcat::new([1..=1].into_iter()).collect();
        assert_eq!(concat, [1..=1]);

        let concat: Vec<_> = RangeConcat::new([1..=1, 2..=3].into_iter()).collect();
        assert_eq!(concat, [1..=3]);

        let concat: Vec<_> = RangeConcat::new([1..=1, 3..=6].into_iter()).collect();
        assert_eq!(concat, [1..=1, 3..=6]);

        let concat: Vec<_> = RangeConcat::new([1..=1, 2..=5, 7..=7, 8..=9].into_iter()).collect();
        assert_eq!(concat, [1..=5, 7..=9]);
    }
}
