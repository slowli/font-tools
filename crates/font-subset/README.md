# OpenType Font Subsetting

[![Build status](https://github.com/slowli/font-tools/actions/workflows/ci.yml/badge.svg)](https://github.com/slowli/font-tools/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue)](https://github.com/slowli/tracing-toolbox#license)
![rust 1.83+ required](https://img.shields.io/badge/rust-1.83+-blue.svg?label=Required%20Rust)
![no_std tested](https://img.shields.io/badge/no__std-tested-green.svg)

This is a simple, no-std-compatible library that provides OpenType font *subsetting*, i.e.,
retaining only glyphs and other related data that correspond to specific chars. The subset can then be
saved in the OpenType (`.otf` / `.ttf`) or WOFF2 format.

As an example, it is possible to subset visible ASCII chars (`' '..='~'`) from a font that originally supported
multiple languages. Subsetting may lead to significant space savings; e.g., a subset of Roboto (the standard
sans-serif font for Android) with visible ASCII chars occupies just 19 kB in the OpenType format
(and 11 kB in the WOFF2 format) vs the original 457 kB.

The motivating use case for this library is embedding the produced font as a data URL in HTML or SVG,
so that it's guaranteed to be rendered in the same way across platforms.

## Features

- Can read and write fonts to/from OpenType and WOFF2 (the latter via an opt-in crate feature).
- Supports [variable fonts]. This allows to embed a continuous set of fonts across one or more dimensions, such as font weight
  or width, into a single file.
- Provides general info about fonts, e.g., font naming / license info and variation axis parameters.
- Single dependency for WOFF2 (de)compression; no-std compatible.

## Design philosophy

- **Keep parsing simple.** The library generally assumes that the input is well-formed, and defers parsing
  when possible. For example, simple glyph data is not parsed at all because it's not necessary for subsetting;
  it can be copied as opaque bytes.
- **Keep focus.** The library is focused on subsetting. For example, it doesn't strive to parse all tables from
  the OpenType spec.
- **Keep dependencies lean.** The library has the only opt-in dependency ([`brotli`]) to support WOFF2 serialization.
  The library is unconditionally no-std-compatible.

## Usage

Add this to your `Crate.toml`:

```toml
[dependencies]
font-subset = "0.1.0"
```

### Subsetting

```rust
use std::collections::BTreeSet;
use font_subset::{Font, ParseError};

// Load the Fira Mono monospace font (~129 kB in the OpenType format).
let font_bytes = include_bytes!("../examples/FiraMono-Regular.ttf");
// Parse the font.
let font = Font::opentype(font_bytes)?;
// Ensure that the font license permits embedding and subsetting.
let permissions = font.permissions();
assert!(permissions.embedding.is_lenient());
assert!(permissions.allow_subsetting);

// Create a subset.
let retained_chars: BTreeSet<char> = (' '..='~').collect();
let subset = font.subset(&retained_chars)?;
// Serialize the subset in OpenType and WOFF2 formats.
let ttf: Vec<u8> = subset.to_opentype();
println!("OpenType size: {}", ttf.len());
assert!(ttf.len() < 20 * 1_024);

let woff2: Vec<u8> = subset.to_woff2();
println!("WOFF2 size: {}", woff2.len());
assert!(woff2.len() < 15 * 1_024);
Ok::<_, ParseError>(())
```

## Known limitations

- `glyf` + `loca` transforms are not supported when writing / reading WOFF2 files (yet?).
- Subsetting drops advanced layout tables like `GPOS`, `kern` etc.
- Some table data (e.g., `maxp` fields like "maximum points in a non-composite glyph") are not updated
  in the subset font. Looks like some other subsetters (e.g., [`allsorts`]) do not update them either.

## Alternatives and similar tools

- [`allsorts`] is a library working with OpenType / WOFF / WOFF2 fonts, including
  their subsetting. It's more versatile, but at the cost of having more deps and requiring
  the standard library (although there is [a no-std fork](https://crates.io/crates/allsorts_no_std)).
  Its subsetting logic also produces fonts not parseable by browsers because of a missing `OS/2` table.
- [`subsetter`] is a specialized subsetting library. but it's geared towards PDF subsetting specifically
  (i.e., again not covering the motivating SVG use case).

## License

All code is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `font-tools` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[variable fonts]: https://learn.microsoft.com/en-us/typography/opentype/spec/otvaroverview
[`brotli`]: https://crates.io/crates/brotli
[`allsorts`]: https://crates.io/crates/allsorts
[`subsetter`]: https://crates.io/crates/subsetter
