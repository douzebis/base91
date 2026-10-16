<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0015 — A crates.io page that sells the speed

**Status:** implemented
**App:** base91
**Implemented in:** 2026-10-16

## Problem

The crates.io page of `base91-rs` does not show what makes it worth
choosing: the speed of its SIMD variant.

- **The one-line description** is generic: *"basE91 binary-to-text
  encoding — fast, no_std-compatible Rust implementation"*.  "Fast" is
  not quantified, and "basE91" reads as jargon.
- **The README** (`rust/base91/README.md`, 245 lines, the page's body) is
  a reference manual: the speed table comes after "Why basE91?" and
  "Features", then eight usage sections (Henke and SIMD, one-shot and
  streaming, errors, the slice API, `std::io`, `no_std`) and a migration
  guide.  It looks complicated before it looks useful.
- **The keywords** (`base91`, `encoding`, `binary-to-text`, `codec`) omit
  `simd`.
- **The dependencies tab** lists an unrelated crate named `base91`: a
  dev-dependency of the comparison benchmark, which also brings
  `criterion`, a `build.rs` and an optional `cc` build-dependency into the
  published crate (feature `c-compat-tests`, which needs `src/` two
  directories up and so cannot work from crates.io anyway).

## Goals

- **G1.** One bold line that says it: base91, faster.
- **G2.** Above the fold: the line, the speed table per configuration,
  how to add the crate, a SIMD and a Henke example.  Everything else is
  one link away.
- **G3.** Nothing lost: the detailed usage moves to the crate's
  documentation on docs.rs, where its examples are compiled and run as
  doctests.
- **G4.** The published crate depends only on what it uses.
- **G5.** Shipped in 0.3.3, with spec 0014.

## Non-goals

- **N1.** Changing the API or the measurements' method.
- **N2.** The PyPI page and the nixpkgs `meta.description` (later, in the
  same voice, once this one is settled).
- **N3.** A website or a logo.

## Specification

### 1. Wording

- Write "base91" in prose, headings and descriptions.  "basE91" (Henke's
  spelling) appears once, where the README credits Henke's format.
  Identifiers keep their names (`basE91_*` in C).
- **Description** (`rust/base91/Cargo.toml`, shown under the crate's name
  on crates.io and in search results):
  > base91, but faster: a SIMD variant at 25× the speed
- **Keywords**: `base91`, `simd`, `encoding`, `binary-to-text`, `no-std`.
  **Categories**: `encoding`, `no-std`, `no-std::no-alloc`.
- The "25×" is the SIMD variant (AVX2) against the `base91` crate
  (0.1.0, Henke's format), the most used base91 crate, both measured by
  `throughput` in the same run (0.3.2: encode 24.8×, decode 25.3×).  The
  table below the line gives each figure, that crate included, so the
  claim's basis is on the page.

### 2. README (`rust/base91/README.md`)

Structure (reviewed 2026-10-16, after the `base91` crate's README: a
compact description, self-contained examples, license, acknowledgements):

1. Title `base91-rs`, the description in bold, badges (crates.io, docs.rs,
   license).
2. **What it is**, 3 to 5 lines: base91 against base64 (at most 23%
   overhead, base64: 33%), Henke's format and the SIMD variant.
3. **Speed**, a table per configuration (encode, decode), the `base91`
   crate included, with the machine, the input and the version in one line
   under it.  Figures measured for 0.3.3 (criterion, 1 MiB, one run on
   vCPU 8): SIMD AVX2 16.6 / 19.6 GiB/s, the `base91` crate 703 / 720
   MiB/s (24.2× / 27.9×).
4. **Features**, one line each: both formats, `no_std` and the slice
   API, streaming and `std::io`, strict decoding, the command and the
   bindings.
5. **Examples**: `cargo add base91-rs`, then a SIMD and a Henke example
   that show the encoded output (`-r2T-Er|#{eyjjUg/`, `>OwJh>}A"=r@@Y?F`);
   a link to docs.rs for the rest.  The README is a doctest
   (`#[cfg(doctest)] #[doc = include_str!("../README.md")]`), so the
   outputs it shows are checked.
6. **Which format?**, then links: the wire format, the changelog,
   migrating from 0.2 (`docs/migrating-0.2.md`).
7. **License**, then **Acknowledgements** (Joachim Henke).

### 3. Crate documentation (`rust/base91/src/lib.rs`)

The usage sections leaving the README become the crate-level
documentation (`//!`), as doctests: one-shot and streaming (both
formats), errors, the slice API, `std::io`, `no_std` and features.
`cargo test --doc` runs them (in `rust-tests`).

### 4. The benchmark leaves the published crate

New workspace member `rust/base91-bench`, `publish = false`, holding
`benches/throughput.rs` and what only it uses: the comparison crate
(`base91` 0.1, renamed `base91-dnsl48`), `criterion`, the `c-compat-tests`
feature, its `build.rs` and `cc`.  `base91-rs` drops them; the comparison
stays measured, from `base91-bench`.  The `bench`
workflow and `default.nix` call the bench in its new crate; the README's
figures keep their method.

### 5. The other READMEs

Each README is written for its own page, in the same structure and voice,
with the same figures, but not shared text: the top-level `README.md`
presents the whole project (CLI, Rust, Python, C, Go) for GitHub; the
CLI's (`rust/base91-cli/README.md`, on crates.io) presents the command,
with its examples as shell sessions showing the output.

- `rust/base91-cli/Cargo.toml` description: "base91, but faster: the
  command-line encoder and decoder, with a SIMD variant at 25× the
  speed".

## Tests

- `cargo test --doc -p base91-rs` passes: the moved examples and the
  README's.
- `scripts/check-crates`: the packaged `base91-rs` builds and tests
  without `build.rs`, `criterion` or `base91`; `cargo package --list`
  shows no `benches/`.
- crates.io API, after the release: the dependencies of `base91-rs`
  0.3.3 no longer include `base91`.
- The README renders on crates.io (checked on the published page).

## Open questions

- None.  To confirm in review: the exact description line, and the
  configurations in the table.

## References

- [spec 0011](0011-simd-kernel-performance.md) — the measurement method
- [spec 0013](0013-crates-io-publishing.md) — `check-crates`, publishing
- [spec 0014](0014-simd-bindings.md) — the bindings the README links to
- `rust/base91/benches/throughput.rs`
