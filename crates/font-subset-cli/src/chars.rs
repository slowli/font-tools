//! Char ranges.

use std::{
    ops,
    str::{CharIndices, FromStr},
};

use anyhow::Context;

/// Regex-like char set like `A-Za-z!,@`.
#[derive(Debug, Clone)]
pub(crate) struct CharSet(Vec<ops::RangeInclusive<char>>);

impl CharSet {
    pub(crate) fn iter(&self) -> impl Iterator<Item = char> + '_ {
        self.0.iter().flat_map(ops::RangeInclusive::clone)
    }
}

impl FromStr for CharSet {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        #[derive(Debug, Clone, Copy)]
        enum PrevChar {
            Plain(char),
            RangeStart(char),
        }

        let mut chars = s.char_indices();
        let mut prev_char = None::<PrevChar>;
        let mut ranges = vec![];
        while let Some((idx, ch)) = chars.next() {
            let parsed = match ch {
                // Escaped char like `\u{7f}`
                '\\' => parse_escaped_char(s, &mut chars)?,
                '-' => {
                    match prev_char {
                        None => ch, // treat `-` as an ordinary char
                        Some(PrevChar::Plain(prev)) => {
                            prev_char = Some(PrevChar::RangeStart(prev));
                            continue;
                        }
                        Some(PrevChar::RangeStart(_)) => {
                            anyhow::bail!("at {idx}: repeated '-' char");
                        }
                    }
                }
                _ => ch,
            };

            match prev_char {
                None => {
                    prev_char = Some(PrevChar::Plain(parsed));
                }
                Some(PrevChar::Plain(prev)) => {
                    ranges.push(prev..=prev);
                    prev_char = Some(PrevChar::Plain(parsed));
                }
                Some(PrevChar::RangeStart(prev)) => {
                    anyhow::ensure!(prev <= parsed, "at {idx}: range {prev}-{parsed} is empty");
                    ranges.push(prev..=parsed);
                    prev_char = None;
                }
            }
        }

        match prev_char {
            None => { /* do nothing */ }
            Some(PrevChar::Plain(prev)) => {
                ranges.push(prev..=prev);
            }
            Some(PrevChar::RangeStart(prev)) => {
                ranges.push(prev..=prev);
                // Treat the ending `-` as an ordinary char.
                ranges.push('-'..='-');
            }
        }

        Ok(Self(ranges))
    }
}

fn parse_escaped_char(src: &str, chars: &mut CharIndices<'_>) -> anyhow::Result<char> {
    const UNFINISHED: &str = "unfinished escaped char";
    const INVALID_FORMAT: &str = "escaped chars should have \\u{..} or \\U{..} form";

    let (idx, sigil) = chars.next().context(UNFINISHED)?;
    anyhow::ensure!(sigil == 'u' || sigil == 'U', "at {idx}: {INVALID_FORMAT}");
    let (idx, open_brace) = chars.next().context(UNFINISHED)?;
    anyhow::ensure!(open_brace == '{', "at {idx}: {INVALID_FORMAT}");

    let start_pos = idx + 1;
    let end_pos = loop {
        let (idx, ch) = chars.next().context(UNFINISHED)?;
        if ch == '}' {
            break idx;
        }
    };
    let codepoint = &src[start_pos..end_pos];
    let codepoint = u32::from_str_radix(codepoint, 16).context(INVALID_FORMAT)?;
    char::from_u32(codepoint)
        .with_context(|| format!("{codepoint} is not a valid Unicode char codepoint"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_char_sets() {
        let set: CharSet = "a".parse().unwrap();
        assert_eq!(set.0, ['a'..='a']);

        let set: CharSet = "abc".parse().unwrap();
        assert_eq!(set.0, ['a'..='a', 'b'..='b', 'c'..='c']);

        let set: CharSet = "a-d".parse().unwrap();
        assert_eq!(set.0, ['a'..='d']);

        let set: CharSet = "a-dZ".parse().unwrap();
        assert_eq!(set.0, ['a'..='d', 'Z'..='Z']);

        let set: CharSet = "a-d-Z".parse().unwrap();
        assert_eq!(set.0, ['a'..='d', '-'..='-', 'Z'..='Z']);

        let set: CharSet = "a-zA-Z".parse().unwrap();
        assert_eq!(set.0, ['a'..='z', 'A'..='Z']);

        let set: CharSet = "a-zA-Z-".parse().unwrap();
        assert_eq!(set.0, ['a'..='z', 'A'..='Z', '-'..='-']);

        let set: CharSet = "-a-zA-Z".parse().unwrap();
        assert_eq!(set.0, ['-'..='-', 'a'..='z', 'A'..='Z']);

        let set: CharSet = "a-zA-\\u{7f}".parse().unwrap();
        assert_eq!(set.0, ['a'..='z', 'A'..='\u{7f}']);

        let set: CharSet = "a-zA-Z\\U{7f}".parse().unwrap();
        assert_eq!(set.0, ['a'..='z', 'A'..='Z', '\u{7f}'..='\u{7f}']);
    }

    #[test]
    fn char_set_errors() {
        let err = "x--D".parse::<CharSet>().unwrap_err().to_string();
        assert!(err.contains("repeated '-'"), "{err}");

        let err = "x-D".parse::<CharSet>().unwrap_err().to_string();
        assert!(err.contains("range x-D is empty"), "{err}");

        let err = "x-\\u{44}".parse::<CharSet>().unwrap_err().to_string();
        assert!(err.contains("range x-D is empty"), "{err}");

        for unfinished in ["\\", "\\u", "\\u{", "\\u{123"] {
            let err = unfinished.parse::<CharSet>().unwrap_err().to_string();
            assert!(err.contains("unfinished escaped char"), "{err}");
        }

        let err = "x-\\u{4?}".parse::<CharSet>().unwrap_err().to_string();
        assert!(err.contains("\\u{..} or \\U{..}"), "{err}");

        let err = "x-\\x44".parse::<CharSet>().unwrap_err().to_string();
        assert!(err.contains("\\u{..} or \\U{..}"), "{err}");
    }
}
