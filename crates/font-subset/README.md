# OpenType Font Subsetting

This is a simple, no-std-compatible library that provides OpenType font *subsetting*, i.e.,
retaining only glyphs and other related data that correspond to specific chars. The subset can then be
saved in the OpenType (`.ttf`) or WOFF2 format.

As an example, it is possible to subset visible ASCII chars (`' '..='~'`) from a font that originally supported
multiple languages. Subsetting may lead to significant space savings; e.g., a subset of Roboto (the standard
sans-serif font for Android) with visible ASCII chars occupies just 19 kB in the OpenType format
(and 11 kB in the WOFF2 format) vs the original 457 kB.

The motivating use case for this library is embedding the produced font as a data URL in HTML or SVG,
so that it's guaranteed to be rendered in the same way across platforms.

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
let font = Font::new(font_bytes)?;
let retained_chars: BTreeSet<char> = (' '..='~').collect();
// Create a subset.
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

## License

All code is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `font-tools` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
