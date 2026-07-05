# rvision

A hand-rolled [TurboVision](https://en.wikipedia.org/wiki/Turbo_Vision)-style
terminal UI framework, written in Rust.

This started as part of a Rust learning project: as much as practical is
built from scratch. The only external runtime crates are the OS/terminal
boundary (`crossterm`) and the Unicode data tables the standard library
doesn't ship (`unicode-width`, `unicode-segmentation`). It was extracted from
its original home in the [`edit`](https://github.com/SuzukiStumpy/edit) text
editor once it needed a life of its own.

## Layout

```
src/            the framework
examples/       manual-verification demos (`cargo run --example <name>`)
docs/
  getting-started.md   quickstart for building on rvision
  adr/          architecture decision records
  specs/        one spec per module
  module-spec-template.md
```

## Build & test

```sh
cargo build
cargo test
cargo doc --open
```

The toolchain is pinned in `rust-toolchain.toml` (MSRV 1.85, Rust 2024
edition). If you don't have Rust, install it via [rustup](https://rustup.rs).

## Where to start reading

New to `rvision`? [`docs/getting-started.md`](docs/getting-started.md) walks
through building an application on it, from a bare event loop up to a
`Shell`/`Desktop` with a modal dialog.

For the *why* behind each major design decision, `docs/adr/`, one numbered
record per decision. `CLAUDE.md` holds the working conventions.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
