//! `font-subset` command-line application.

use std::{fmt, fs, io, io::Read, path::PathBuf};

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

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum CliCommand {
    /// Prints information about a font.
    Info {
        #[command(flatten)]
        shared: SharedArgs,
    },
}

#[derive(Debug, Clone, Args)]
struct SharedArgs {
    /// Path to the font file, or `-` to read from stdin.
    path: PathBuf,
}

impl SharedArgs {
    fn read_font<'font>(&self, buffer: &'font mut Vec<u8>) -> anyhow::Result<FontReader<'font>> {
        if self.path.as_os_str() == "-" {
            io::stdin()
                .read_to_end(buffer)
                .context("cannot read font bytes from stdin")?;
        } else {
            *buffer = fs::read(&self.path).with_context(|| {
                format!("cannot read font bytes from `{}`", self.path.display())
            })?;
        }
        FontReader::new(&*buffer).context("failed parsing font header")
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FontFormat {
    OpenType,
    Woff2,
}

impl Cli {
    fn run(self) -> anyhow::Result<()> {
        match self.command {
            CliCommand::Info { shared } => {
                let mut buffer = vec![];
                let font_reader = shared.read_font(&mut buffer)?;
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
