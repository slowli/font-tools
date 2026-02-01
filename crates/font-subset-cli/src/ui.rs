//! UI components.

use std::fmt;

use anstyle::{AnsiColor, Color, Style};

pub(crate) const SECTION: Style = Style::new().bold();
pub(crate) const DIMMED: Style = Style::new().dimmed();
pub(crate) const VAL: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));
const BORDER: Style = Style::new().bold();

#[derive(Debug)]
pub(crate) struct HorizontalBar {
    char_length: usize,
    filled_count: usize,
}

impl fmt::Display for HorizontalBar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const FILLED: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));

        write!(formatter, "{BORDER}[{BORDER:#}{FILLED}")?;
        for _ in 0..self.filled_count {
            write!(formatter, "=")?;
        }
        write!(formatter, "{FILLED:#}")?;

        for _ in 0..self.char_length - self.filled_count {
            write!(formatter, " ")?;
        }
        write!(formatter, "{BORDER}]{BORDER:#}")
    }
}

impl HorizontalBar {
    pub(crate) fn new(char_length: usize, filled_count: usize) -> Self {
        assert!(filled_count <= char_length);
        Self {
            char_length,
            filled_count,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Checkbox(pub(crate) bool);

impl fmt::Display for Checkbox {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const CHECKED: Style = Style::new()
            .bold()
            .fg_color(Some(Color::Ansi(AnsiColor::Green)));
        const NOT_CHECKED: Style = Style::new()
            .bold()
            .fg_color(Some(Color::Ansi(AnsiColor::Red)));

        write!(formatter, "{BORDER}[{BORDER:#}")?;
        if self.0 {
            write!(formatter, "{CHECKED}√{CHECKED:#}")?;
        } else {
            write!(formatter, "{NOT_CHECKED}x{NOT_CHECKED:#}")?;
        }
        write!(formatter, "{BORDER}]{BORDER:#}")
    }
}
