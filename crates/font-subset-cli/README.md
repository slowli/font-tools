# font-subset CLI

[![Build status](https://github.com/slowli/font-tools/actions/workflows/ci.yml/badge.svg)](https://github.com/slowli/font-tools/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue)](https://github.com/slowli/tracing-toolbox#license)

This crate provides command-line interface for [`font-subset`]. It allows to subset OpenType and WOFF2 fonts,
and to print general info about fonts.

## Installation

Install with

```shell
cargo install --locked font-subset-cli
# This will install `font-subset` executable, which can be checked
# as follows:
font-subset --help
```

### Minimum supported Rust version

The crate supports the latest stable Rust version. It may support previous stable Rust versions,
but this is not guaranteed.

### Crate feature: `tracing`

Specify `--features tracing` in the installation command to enable tracing
of the main performed operations. This could be useful for debugging purposes.
Tracing is performed with the `font_subset::*` targets, mostly on the `DEBUG` and `TRACE` levels.
Tracing events are output to the stderr using [the standard subscriber][fmt-subscriber];
its filtering can be configured using the `RUST_LOG` env variable
(e.g., `RUST_LOG=warn,font_subset=debug`).

## Usage

### `info` subcommand

The `info` subcommand prints general info about an OpenType or WOFF2 font. Add `--verbose` for more details.

![`info` subcommand for a sample font](examples/info.svg)

### `subset` subcommand

The `subset` subcommand allows to perform subsetting. It allows to specify string(s) with the subset chars,
and/or char range(s) with a RegEx-like syntax (e.g., `a-zA-Z0-9()` will select ASCII uppercase and lowercase latin chars,
decimal digits and parentheses).

![`subset` subcommand](examples/subset.svg)

The subcommand supports [variable fonts] and retains variation data by default. This data can be dropped by supplying
`--drop-var` flag.

![`subset` subcommand with `--drop-var` flag](examples/subset-drop-var.svg)

By specifying `-` for input / output file names, it's possible to invoke the `font-subset` as a part of a pipeline.

![`subset` subcommand in a pipeline](examples/subset-streaming.svg)

## License

All code is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `font-tools` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[`font-subset`]: https://crates.io/crates/font-subset/
[fmt-subscriber]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/index.html
[variable fonts]: https://learn.microsoft.com/en-us/typography/opentype/spec/otvaroverview
