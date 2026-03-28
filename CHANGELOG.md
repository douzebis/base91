<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [0.2.1] - 2026-03-28

### Added

- `README.md` for `base91-rs` crate with quick-start, API overview, and
  performance comparison vs `base91` v0.1.0 (dnsl48).
- Criterion benchmark now includes `base91` v0.1.0 as a comparison target.

---

## [0.2.0] - 2026-03-28

First Rust release.  Wire-format compatible with Joachim Henke's C reference
implementation at <http://base91.sourceforge.net/>.

### Added

#### `base91` crate (crates.io)

- Pure-Rust, `no_std`-compatible implementation of the basE91 algorithm.
- Streaming API: `Encoder` / `Decoder` structs with `encode()` / `decode()` /
  `finish()` methods.
- One-shot helpers: `encode(input: &[u8]) -> Vec<u8>` and
  `decode(input: &[u8]) -> Vec<u8>`.
- Unsafe unchecked helpers: `encode_unchecked` / `decode_unchecked` for
  callers that pre-allocate output with the provided size hints.
- `std::io` adapters (feature `io`, on by default): `EncoderWriter<W>` and
  `DecoderReader<R>`.
- C-compatible ABI (feature always-on): five `basE91_*` functions with struct
  layout identical to the C reference, exposed via `#[no_mangle]` exports and
  a `cbindgen`-generated `include/base91.h`.
- PyO3 Python bindings (feature `python`, off by default): one-shot
  `encode`/`decode` functions and streaming `Encoder`/`Decoder` classes,
  with `pyo3-stub-gen` type stubs.
- Feature `c-compat-tests`: gates cross-check benchmarks against the compiled
  C reference; never enabled by default or on crates.io.

#### `base91-cli` crate (crates.io)

- `base91` binary with full flag parity with the C CLI: `-d`/`--decode`,
  `-e`, `-o`/`--output`, `-m`/`--buffer`, `-v`/`--verbose`, `-w`/`--wrap`.
- `b91enc` and `b91dec` symlink behavior: invocation name selects the default
  mode and wrap setting, matching the C reference.
- Man page `base91.1` generated at build time via `clap_mangen`.
- Shell completions for bash, zsh, fish, elvish, and PowerShell generated at
  build time via `clap_complete`.

#### `pybase91` Python package (PyPI)

- `pybase91.encode(data: bytes) -> bytes`
- `pybase91.decode(data: bytes) -> bytes`
- `pybase91.Encoder` streaming class with `update(data: bytes)` and
  `finish() -> bytes`.
- `pybase91.Decoder` streaming class with `update(data: bytes)` and
  `finish() -> bytes`.
- `pybase91.pyi` type stubs generated via `pyo3-stub-gen`.
- Build backend: `maturin` (supports `pip install .`, `maturin develop`, and
  `maturin publish`).

#### Nix

- `default.nix`: crane-based derivations for all three packages.
- `shell.nix`: development shell with Rust toolchain, cbindgen, Python,
  maturin, reuse, gh, mandoc.

#### Tests

- 18 unit tests in `base91/src/` (codec correctness, table invariants,
  streaming chunk independence, io adapters).
- 9 reference-vector tests against fixtures derived from the C `test.sh`
  (SHA-256 verified against C reference output).
- 13 CLI subprocess integration tests (round-trips, wrap flag, symlink
  invocation, `-o FILE`, `-v`).

---

## [0.1.0] - 2006

Original C implementation by Joachim Henke.
Source preserved unchanged in `src/`.

[Unreleased]: https://github.com/douzebis/base91/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/douzebis/base91/releases/tag/v0.2.1
[0.2.0]: https://github.com/douzebis/base91/releases/tag/v0.2.0
[0.1.0]: http://base91.sourceforge.net/
