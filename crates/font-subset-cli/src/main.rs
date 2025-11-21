//! `font-subset` command-line application.

use std::{
    collections::BTreeSet,
    fmt, fs, io,
    io::{Read, Write},
    ops,
    path::{Path, PathBuf},
    str::FromStr,
};

use anstream::{print, println};
use anstyle::{AnsiColor, Color, Style};
use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use font_subset::{EmbeddingPermissions, Font, FontReader, VariationAxis};

const SECTION: Style = Style::new().bold();
const DIMMED: Style = Style::new().dimmed();
const BORDER: Style = Style::new().bold();

#[derive(Debug)]
struct HorizontalBar {
    char_length: usize,
    filled_count: usize,
}

impl fmt::Display for HorizontalBar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const FILLED: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));

        write!(formatter, "{BORDER}[{BORDER:#}{FILLED}")?;
        for _ in 0..self.filled_count {
            write!(formatter, "#")?;
        }
        write!(formatter, "{FILLED:#}")?;

        for _ in 0..self.char_length - self.filled_count {
            write!(formatter, " ")?;
        }
        write!(formatter, "{BORDER}]{BORDER:#}")
    }
}

impl HorizontalBar {
    fn new(char_length: usize, filled_count: usize) -> Self {
        assert!(filled_count <= char_length);
        Self {
            char_length,
            filled_count,
        }
    }
}

#[derive(Debug)]
struct Checkbox(bool);

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

#[derive(Debug, Clone)]
struct CharRange(ops::RangeInclusive<char>);

impl FromStr for CharRange {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // FIXME: parse regex-like syntax: a-zA-Z!,0-9\u{77a}

        let (start, end) = s
            .split_once('-')
            .context("range does not contain `-` delimiter")?;
        let start = parse_char(start)
            .with_context(|| format!("invalid start char of the range {start:?}"))?;
        let end =
            parse_char(end).with_context(|| format!("invalid end char of the range {end:?}"))?;

        Ok(Self(start..=end))
    }
}

fn parse_char(s: &str) -> anyhow::Result<char> {
    const INVALID_FORMAT: &str = "invalid code point format; should be \\u{<hex>}";

    // Check for a single-char string.
    let mut s_chars = s.chars();
    let first_char = s_chars.next().context("empty")?;
    if s_chars.next().is_none() {
        return Ok(first_char);
    }

    // Check for \u|U{..}
    let code = s.strip_prefix("\\u{").or_else(|| s.strip_prefix("\\U{"));
    let code = code.context(INVALID_FORMAT)?;
    let code = code.strip_suffix('}').context(INVALID_FORMAT)?;
    let code = u32::from_str_radix(code, 16).context(INVALID_FORMAT)?;
    char::from_u32(code).with_context(|| format!("{code} is not a valid Unicode char code"))
}

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum CliCommand {
    /// Prints information about a font.
    Info {
        /// Path to the font file, or `-` to read from stdin.
        path: PathBuf,
    },
    /// Subsets a font.
    Subset(SubsetCommand),
}

#[derive(Debug, Clone, Args)]
struct SubsetCommand {
    /// Subset the ASCII chars (' ' / 0x20 to '~' / 0x7e).
    #[arg(long)]
    ascii: bool,
    /// Chars to include in the subset.
    #[arg(long, short = 'C')]
    chars: Vec<String>,
    /// Char ranges to include in the subset.
    #[arg(long, short = 'R')]
    ranges: Vec<CharRange>,

    /// Output format. If not specified, will be determined by the output path extension.
    #[arg(long)]
    format: Option<FontFormat>,
    /// Forces subsetting even if the font permissions don't support it.
    #[arg(long)]
    force: bool,
    /// Allows chars mapped to the undefined glyph.
    #[arg(long)]
    allow_missing: bool,
    /// Path to the output font, or `-` to print to stdout.
    #[arg(long = "out", short = 'o', value_name = "PATH")]
    output: PathBuf,
    /// Path to the font file, or `-` to read from stdin.
    path: PathBuf,
}

impl SubsetCommand {
    fn run(self) -> anyhow::Result<()> {
        let format = if let Some(format) = self.format {
            format
        } else {
            FontFormat::detect(&self.output).with_context(|| {
                format!(
                    "cannot detect font format for output path `{}`",
                    self.output.display()
                )
            })?
        };

        let mut buffer = vec![];
        let font_reader = read_font(&self.path, &mut buffer)?;
        let font = font_reader.read().context("failed parsing font")?;

        anyhow::ensure!(
            self.force || font.permissions().allow_subsetting,
            "font permissions do not allow subsetting"
        );

        let mut chars = BTreeSet::new();
        if self.ascii {
            chars.extend(' '..='~');
        }
        chars.extend(self.chars.iter().flat_map(|s| s.chars()));
        chars.extend(self.ranges.iter().flat_map(|range| range.0.clone()));

        anyhow::ensure!(!chars.is_empty(), "The char subset is empty");

        if !self.allow_missing {
            let missing_chars: Vec<_> = chars
                .iter()
                .copied()
                .filter(|&ch| !font.contains_char(ch))
                .collect();
            anyhow::ensure!(
                missing_chars.is_empty(),
                "Source font does not contain some of subset chars: {missing_chars:?}"
            );
        }

        let subset = font.subset(&chars).context("failed subsetting")?;
        let output_bytes = match format {
            FontFormat::OpenType => subset.to_opentype(),
            FontFormat::Woff2 => subset.to_woff2(),
        };
        if self.output.as_os_str() == "-" {
            io::stdout()
                .write_all(&output_bytes)
                .context("failed writing subset font to stdout")?;
        } else {
            fs::write(&self.output, &output_bytes).with_context(|| {
                format!("failed writing subset font to `{}`", self.output.display())
            })?;
        }
        Ok(())
    }
}

fn read_font<'font>(path: &Path, buffer: &'font mut Vec<u8>) -> anyhow::Result<FontReader<'font>> {
    if path.as_os_str() == "-" {
        io::stdin()
            .read_to_end(buffer)
            .context("cannot read font bytes from stdin")?;
    } else {
        *buffer = fs::read(path)
            .with_context(|| format!("cannot read font bytes from `{}`", path.display()))?;
    }
    FontReader::new(&*buffer).context("failed parsing font header")
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FontFormat {
    OpenType,
    Woff2,
}

impl FontFormat {
    fn detect(path: &Path) -> Option<Self> {
        Some(match path.extension()?.to_str()? {
            "otf" | "ttf" => Self::OpenType,
            "woff" | "woff2" => Self::Woff2,
            _ => return None,
        })
    }
}

impl Cli {
    fn run(self) -> anyhow::Result<()> {
        match self.command {
            CliCommand::Info { path } => {
                let mut buffer = vec![];
                let font_reader = read_font(&path, &mut buffer)?;
                let font = font_reader.read().context("failed parsing font")?;
                Self::print_font_naming(&font);
                println!();
                Self::print_numeric_stats(&font);
                if let Some(axes) = font.variation_axes() {
                    println!();
                    Self::print_variation_axes(axes);
                }
                println!();
                Self::print_table_stats(&font_reader);
            }
            CliCommand::Subset(cmd) => cmd.run()?,
        }
        Ok(())
    }

    fn print_font_naming(font: &Font<'_>) {
        let naming = font.naming();
        if let Some(family) = &naming.family {
            let subfamily = naming.subfamily.as_deref().unwrap_or("");
            println!("{SECTION}{family}{SECTION:#} {DIMMED}{subfamily}{DIMMED:#}");
        }
        if let Some(manufacturer) = &naming.manufacturer {
            println!("{SECTION}by{SECTION:#} {manufacturer}");
        }
        if let Some(license) = &naming.license {
            println!("{SECTION}License:{SECTION:#} {license}");
        }
        if let Some(url) = &naming.license_url {
            println!("{SECTION}License URL:{SECTION:#} {url}");
        }

        let permissions = font.permissions();
        let embedding = match permissions.embedding {
            EmbeddingPermissions::Installable => "Installable",
            EmbeddingPermissions::RestrictedLicense => "Restricted license",
            EmbeddingPermissions::PreviewAndPrint => "Preview & print",
            EmbeddingPermissions::Editable => "Editable",
        };
        let is_lenient =
            Checkbox(permissions.embedding.is_lenient() && !permissions.embed_only_bitmaps);
        let only_bitmaps = if permissions.embed_only_bitmaps {
            " (only bitmaps)"
        } else {
            ""
        };
        println!("{SECTION}Embedding:{SECTION:#} {is_lenient} {embedding}{only_bitmaps}");
        let checkbox = Checkbox(permissions.allow_subsetting);
        let subsetting = if permissions.allow_subsetting {
            "Allowed"
        } else {
            "Not allowed"
        };
        println!("{SECTION}Subsetting:{SECTION:#} {checkbox} {subsetting}");
    }

    fn print_numeric_stats(font: &Font<'_>) {
        const CHAR: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));

        let char_ranges: Vec<_> = font.char_ranges().collect();
        let char_count = char_ranges
            .iter()
            .map(|range| range.clone().count())
            .sum::<usize>();
        print!("{SECTION}Chars{SECTION:#} ({char_count}): ");
        for (i, range) in char_ranges.iter().enumerate() {
            let (start, end) = (*range.start(), *range.end());
            print!("{CHAR}{start:?}{CHAR:#}");
            if start != end {
                print!("–{CHAR}{end:?}{CHAR:#}");
            }
            if i + 1 < char_ranges.len() {
                print!(", ");
            }
        }
        println!();

        let glyph_count = font.glyph_count();
        println!("{SECTION}Glyphs:{SECTION:#} {glyph_count}");
    }

    fn print_variation_axes(axes: &[VariationAxis]) {
        const VAL: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));

        println!("{SECTION}Variations:{SECTION:#}");
        for axis in axes {
            if let Some(name) = &axis.name {
                print!(
                    "  {SECTION}{name}{SECTION:#} {DIMMED}[{tag}]{DIMMED:#}: ",
                    tag = axis.tag
                );
            } else {
                print!("  {SECTION}{tag}{SECTION:#}: ", tag = axis.tag);
            }
            let min = axis.min_value;
            let max = axis.max_value;
            let def = axis.default_value;
            println!("{VAL}{min}{VAL:#}–{VAL}{max}{VAL:#} (default: {VAL}{def}{VAL:#})");
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn print_table_stats(reader: &FontReader<'_>) {
        const BAR_LEN: usize = 50;

        let total_len = reader
            .raw_tables()
            .map(|(_, bytes)| bytes.len())
            .sum::<usize>();
        let tables = reader.raw_tables();
        println!("{SECTION}Font tables{SECTION:#} ({}):", tables.len());
        for (tag, bytes) in tables {
            let frac = bytes.len() as f32 / total_len as f32;
            let filled = (frac * BAR_LEN as f32).round() as usize;
            let bar = HorizontalBar::new(BAR_LEN, filled);
            println!(
                "  {SECTION}{tag}{SECTION:#} {:8} B  {:5.1}% {bar}",
                bytes.len(),
                frac * 100.0
            );
        }
    }
}

fn main() -> anyhow::Result<()> {
    Cli::parse().run()
}
