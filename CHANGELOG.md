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

### Security

- `simd::encode_unchecked` and `simd::decode_unchecked` (`base91-rs`
  0.2.2 only) caused undefined behavior even when the caller honored the
  documented buffer size: they wrapped the caller's buffer in a `Vec`,
  which could reallocate it. Observed effects: double free (encode, inputs
  of 26 bytes or more on SSE4.1/AVX2), wrong output (decode, every SIMD
  level). They now encode or decode into a temporary `Vec` and copy the
  result. Only direct Rust callers of these two functions were affected;
  the CLI, `pybase91` and the `io` adapters never called them. Upgrade to
  0.2.3.
- `simd::scalar::ScalarDecoder::decode` (`base91-rs` 0.2.2 only), a safe
  function, wrote one byte past its reservation when a character was
  pending from the previous call: heap corruption from safe code, observed
  as an abort with 1-character chunks. Reachable only by using
  `ScalarDecoder` directly. It now reserves one more byte. Upgrade to
  0.2.3.

### Fixed

- `base91 -d` and `io::DecoderReader` could silently corrupt SIMD streams
  of about 16 kB and more (`base91-rs`, `base91-cli` 0.2.2):
  `simd::Decoder` lost its state at every chunk boundary. It now carries
  its state across chunks and gives exactly the one-shot result for any
  chunking. Streaming decode throughput, 8 KiB chunks, AVX2 (measured on
  an Intel Core Ultra 7 165U VM, one pinned vCPU): 3.98 GiB/s unwrapped
  (the one-shot speed), 1.40 GiB/s wrapped at 64 columns.
- `base91 --simd` wrote corrupt output for inputs whose size is a multiple
  of 52 bytes (default wrap), or generally whenever the payload filled its
  last line (`simd::encode` with `wrap > 0`, 0.2.2): the first payload
  character was duplicated before the first newline. **Repair** such a
  file by deleting the single character between `-` and the first newline.
  Users who encoded or decoded SIMD streams with 0.2.2 should re-check
  their outputs.
- `simd::Encoder` turned an empty stream into `-`; it now gives the empty
  stream, like `simd::encode`.
- The streaming decoder (Rust `Decoder`, Python `pybase91.Decoder`) could
  panic when fed very small chunks (0.2.0–0.2.2, crates.io and PyPI).
- `base91 -d` and `io::DecoderReader` could stop early and silently drop
  the rest of the input when it arrived in pieces, e.g. from a pipe
  (0.2.0–0.2.2): a read that decoded to nothing was reported as end of
  file.
- `simd::encode_unchecked` on empty input now writes nothing and returns
  0, like `simd::encode` (it used to write `-`).
- `base91-rs` now builds under `no_std` (issue #1): `std` items used
  without a feature gate were replaced by their `core` / `alloc`
  equivalents or gated, and SIMD detection falls back to compile-time
  target features without `std`.
- `default-features = false` no longer fails on hosted targets: the crate
  no longer declares a `cdylib` crate-type (maturin builds the Python
  extension's cdylib itself).

### Added

- `alloc` feature (implied by `std`): the `Vec`-based API under `no_std`.
  Without it, the crate exposes the size hints, the Henke `*_unchecked`
  functions and `simd::detect()`.

### Changed

- Wrapped SIMD output now ends with a newline (spec 0008 §3.4). Every
  0.2.x decoder accepts it.
- Wrapped SIMD encoding is about 3× faster (the newlines are inserted a
  line at a time).

### Removed

- The unused `c_api.rs` and `cbindgen.toml` leftovers.

---

## [0.2.2] - 2026-03-30

### Changed

- `simd::decode`, `simd::decode_unchecked`, and `io::DecoderReader` now
  auto-detect the stream format: a leading `-` byte selects the SIMD path;
  any other first byte falls back to the Henke codec.  Plain `base91 -d`
  (and the library's `DecoderReader`) now decode both SIMD and Henke streams
  transparently — no `--simd` flag needed at decode time.

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

[Unreleased]: https://github.com/douzebis/base91/compare/v0.2.2...HEAD
[0.2.2]: https://github.com/douzebis/base91/releases/tag/v0.2.2
[0.2.1]: https://github.com/douzebis/base91/releases/tag/v0.2.1
[0.2.0]: https://github.com/douzebis/base91/releases/tag/v0.2.0
[0.1.0]: http://base91.sourceforge.net/
