//! `font-subset` command-line application.

use std::{
    collections::BTreeSet,
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anstream::{print, println};
use anstyle::{AnsiColor, Color, Style};
use anyhow::Context;
use chrono::{TimeZone, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use font_subset::{EmbeddingPermissions, Font, FontReader, VariationAxis};

use crate::{
    chars::CharSet,
    ui::{Checkbox, HorizontalBar, DIMMED, SECTION, VAL},
};

mod chars;
mod ui;

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum CliCommand {
    /// Prints information about a font.
    Info {
        /// Print more information.
        #[arg(long)]
        verbose: bool,
        /// Path to the font file, or `-` to read from stdin.
        path: PathBuf,
    },
    /// Subsets a font.
    Subset(SubsetCommand),
}

#[derive(Debug, Clone, Args)]
#[allow(clippy::struct_excessive_bools)] // required by `clap`
struct SubsetCommand {
    /// Subset the ASCII chars (' ' / 0x20 to '~' / 0x7e).
    #[arg(long)]
    ascii: bool,
    /// Chars to include in the subset.
    #[arg(long = "str")]
    strings: Vec<String>,
    /// Chars or char ranges to include in the subset. Support RegExp-like syntax like `a-zA-z!`.
    /// Chars can be escaped using `\u{<hex>}` notation.
    #[arg(long, short = 'C')]
    chars: Vec<CharSet>,

    /// Output format. If not specified, will be determined by the output path extension.
    #[arg(long)]
    format: Option<FontFormat>,
    /// Forces subsetting even if the font permissions don't support it.
    #[arg(long)]
    force: bool,
    /// Allows chars mapped to the undefined glyph.
    #[arg(long)]
    allow_missing: bool,
    /// Drop variation axes in the source font.
    #[arg(long)]
    drop_var: bool,
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

        let buffer = read_from_path(&self.path)?;
        let font_reader = FontReader::new(&buffer).context("failed parsing font header")?;
        let mut font = font_reader.read().context("failed parsing font")?;

        anyhow::ensure!(
            self.force || font.permissions().allow_subsetting,
            "font permissions do not allow subsetting"
        );

        let mut chars = BTreeSet::new();
        if self.ascii {
            chars.extend(' '..='~');
        }
        chars.extend(self.strings.iter().flat_map(|s| s.chars()));
        chars.extend(self.chars.iter().flat_map(CharSet::iter));

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

        if self.drop_var {
            font.drop_variation();
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

fn read_from_path(path: &Path) -> anyhow::Result<Vec<u8>> {
    if path.as_os_str() == "-" {
        let mut buffer = vec![];
        io::stdin()
            .read_to_end(&mut buffer)
            .context("cannot read font bytes from stdin")?;
        Ok(buffer)
    } else {
        fs::read(path).with_context(|| format!("cannot read font bytes from `{}`", path.display()))
    }
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
            CliCommand::Info { verbose, path } => {
                let buffer = read_from_path(&path)?;
                let font_reader = FontReader::new(&buffer).context("failed parsing font header")?;
                let font = font_reader.read().context("failed parsing font")?;
                Self::print_font_naming(&font, &font_reader, buffer.len());
                println!();
                Self::print_numeric_stats(&font);
                if let Some(axes) = font.variation_axes() {
                    println!();
                    Self::print_variation_axes(axes);
                }
                if verbose {
                    println!();
                    Self::print_table_stats(&font_reader);
                }
            }
            CliCommand::Subset(cmd) => cmd.run()?,
        }
        Ok(())
    }

    fn print_font_naming(font: &Font<'_>, reader: &FontReader<'_>, file_len: usize) {
        let naming = font.naming();
        if let Some(family) = &naming.family {
            let subfamily = naming.subfamily.as_deref().unwrap_or("");
            print!("{SECTION}{family}{SECTION:#} {DIMMED}{subfamily}{DIMMED:#}");

            if let Some(version) = naming.version() {
                println!(", {SECTION}ver.{SECTION:#} {version}");
            } else {
                println!();
            }
        }
        if let Some(manufacturer) = &naming.manufacturer {
            println!("{SECTION}by{SECTION:#} {manufacturer}");
        }

        let created_at = font
            .created_at()
            .as_unix_timestamp()
            .and_then(|ts| Utc.timestamp_opt(ts, 0).single());
        if let Some(created_at) = created_at {
            print!("{SECTION}Created at:{SECTION:#} {created_at}");

            let modified_at = font
                .modified_at()
                .as_unix_timestamp()
                .and_then(|ts| Utc.timestamp_opt(ts, 0).single());
            if let Some(modified_at) = modified_at {
                println!(", {SECTION}modified at:{SECTION:#} {modified_at}");
            } else {
                println!();
            }
        }

        if let Some(license) = &naming.license {
            println!("{SECTION}License:{SECTION:#} {license}");
        }
        if let Some(url) = &naming.license_url {
            println!("{SECTION}License URL:{SECTION:#} {url}");
        }

        let format = reader.format();
        let compression = match &reader {
            FontReader::Woff2(reader) => {
                #[allow(clippy::cast_precision_loss)] // OK for general info
                let compression = 1.0 - file_len as f32 / reader.opentype_len() as f32;
                format!(" ({VAL}{:.1}%{VAL:#} compression)", compression * 100.0)
            }
            FontReader::OpenType(_) => String::new(),
            _ => unreachable!(),
        };
        println!("{SECTION}Format:{SECTION:#} {format}, {VAL}{file_len}{VAL:#} B{compression}");

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
        const PERCENT: Style = Style::new().italic();

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
                "  {SECTION}{tag}{SECTION:#} {:8} B  {PERCENT}{:5.1}%{PERCENT:#} {bar}",
                bytes.len(),
                frac * 100.0
            );
        }
    }
}

#[cfg(feature = "tracing")]
fn setup_tracing() {
    use tracing_subscriber::{EnvFilter, FmtSubscriber};

    FmtSubscriber::builder()
        .pretty()
        .with_writer(io::stderr)
        .with_env_filter(EnvFilter::from_default_env())
        .init();
}

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "tracing")]
    setup_tracing();

    Cli::parse().run()
}
