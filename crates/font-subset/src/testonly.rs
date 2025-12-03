use std::{collections::BTreeSet, fmt, ops};

#[derive(Clone, Copy)]
pub(crate) struct TestFont {
    pub(crate) name: &'static str,
    pub(crate) bytes: &'static [u8],
}

impl fmt::Debug for TestFont {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.name, formatter)
    }
}

impl TestFont {
    pub(crate) const FIRA_MONO: Self = Self {
        name: "Fira Mono",
        bytes: include_bytes!("../examples/FiraMono-Regular.ttf"),
    };
    pub(crate) const FIRA_MONO_BOLD: Self = Self {
        name: "Fira Mono Bold",
        bytes: include_bytes!("../examples/FiraMono-Bold.ttf"),
    };
    pub(crate) const ROBOTO: Self = Self {
        name: "Roboto",
        bytes: include_bytes!("../examples/Roboto-VariableFont_wdth,wght.ttf"),
    };
    pub(crate) const ROBOTO_MONO: Self = Self {
        name: "Roboto Mono",
        bytes: include_bytes!("../examples/RobotoMono-VariableFont_wght.ttf"),
    };
    pub(crate) const ROBOTO_MONO_ITALIC: Self = Self {
        name: "Roboto Mono Italic",
        bytes: include_bytes!("../examples/RobotoMono-Italic-VariableFont_wght.ttf"),
    };

    pub(crate) const ALL: [Self; 5] = [Self::FIRA_MONO, Self::ROBOTO, Self::ROBOTO_MONO, Self::FIRA_MONO_BOLD, Self::ROBOTO_MONO_ITALIC];

    /// Variable fonts.
    pub(crate) const VAR: [Self; 3] = [Self::ROBOTO, Self::ROBOTO_MONO, Self::ROBOTO_MONO_ITALIC];
}

#[derive(Debug, Clone)]
pub(crate) enum TestCharSubset {
    Range(ops::RangeInclusive<char>),
    Str(&'static str),
}

impl TestCharSubset {
    pub(crate) fn into_set(self) -> BTreeSet<char> {
        match self {
            Self::Range(range) => range.collect(),
            Self::Str(s) => s.chars().collect(),
        }
    }
}

pub(crate) const SUBSET_CHARS: [TestCharSubset; 5] = [
    TestCharSubset::Range(' '..='~'),
    TestCharSubset::Range('a'..='z'),
    TestCharSubset::Range('0'..='9'),
    TestCharSubset::Str("Hello world!"),
    TestCharSubset::Str("A"),
];
