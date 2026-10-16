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

### Added

- Python: the SIMD variant in `pybase91`: `simd_encode(data, wrap=0)`,
  `simd_decode` (either format, detected), streaming `SimdEncoder` and
  `SimdDecoder`, `simd_level()`, and `pybase91.Error` (a `ValueError` with
  `kind` and `offset`). The existing Henke functions are unchanged
  (spec 0014).
- C: `libbase91` is now built from base91-rs (crate `base91-capi`, not on
  crates.io): Henke's basE91 API (`struct basE91`, the five `basE91_*`
  functions), binary compatible with basE91 0.6.0, and the `base91_*` API
  for both formats, without allocation; `base91.h`, `base91.pc`, SONAME
  `libbase91.so.0`. It replaces the build of Henke's C code
  (`base91-clib`) (spec 0014).
- Python wheels for CPython 3.14.

### Changed

- The crates.io page: "base91, but faster: a SIMD variant at 25× the
  speed", a short README with the speed table, the detailed usage in the
  crate documentation on docs.rs (as doctests), `simd` and `no-std`
  keywords (spec 0015).
- The throughput benchmark moved to the unpublished `base91-bench`:
  `base91-rs` no longer has a `build.rs`, nor the `base91`, `criterion`,
  `rand` and `cc` (dev-)dependencies (spec 0015).
- `pyproject.toml` builds with the new `extension-module` feature; the
  `python` feature alone no longer enables pyo3's `extension-module`
  (the stub generator links libpython).
- The migration guide from 0.2 moved to `docs/migrating-0.2.md`.

---

## [0.3.2] - 2026-10-11

SIMD encoding 19–32 % faster, `base91` quiet on a closed pipe, and the
packaging work: the Nix build uses the nixpkgs recipes (spec 0012), and
releases publish to crates.io again, on approval (spec 0013). No API or
format change.

### Performance

- Wrapped SIMD encoding: each line's LF is written in the kernel's line
  loop instead of a second pass, 3–5 % faster on a performance core,
  2.6 % on an efficiency core (spec 0011, addendum).
- SSE4.1 and AVX2 encoding: the kernels interleave the two base-91 digits
  with one multiply-add (`vals + 165·hi`) instead of two packs and an
  unpack. 19–32 % faster than 0.3.1 on both core types; with rustc 1.98,
  whose `_mm_packus_epi16` clamps before packing, 39–42 % (spec 0012 §4).

### Fixed

- `base91` ends silently when its reader closes the pipe (`base91 big |
  head`), killed by SIGPIPE as the C tool and other Unix filters; it
  printed `base91: Broken pipe (os error 32)` and exited with status 1.

### Changed

- `base91 --man` prints the man page; it replaces `build.rs`, which wrote
  the page and the completions to Cargo's `OUT_DIR` (spec 0012).
- `nix-build -A base91` and `-A pybase91` build the nixpkgs recipes staged
  in `nixpkgs/`, from the local source; `base91.nix` and `-A base91-c` (the
  C 0.1.0 package) are gone (spec 0012).
- One version, in `rust/Cargo.toml`; pybase91 takes it from there.
- Releases publish to crates.io again, from CI: a `v*` tag runs
  `publish.yaml`, which uploads the crates and pybase91 each after the
  maintainer's approval (it replaces `pypi.yml`, which published to PyPI
  on the tag push). `base91-testkit`, the test oracles, is published too,
  so that the tests of `base91-rs` and `base91-cli` compile from their
  packages (spec 0013).

---

## [0.3.1] - 2026-09-28

Faster SIMD and legacy decoding and SIMD encoding (spec 0011). No API or
format change.

### Performance

Criterion, 1 MiB, one performance core of an Intel Core Ultra 7 165U,
against 0.3.0, strict checks included:

- SIMD decode: SSE4.1 201 → 121 µs (−40 %), AVX2 148 → 73 µs (−51 %);
  wrapped at 64 columns 169 → 71 µs; streaming 164 → 82 µs, wrapped
  196 → 94 µs.
- SIMD encode: SSE4.1 216 → 147 µs (−32 %), AVX2 120 → 77 µs (−36 %);
  wrapped at 64 columns 139 → 95 µs.
- Efficiency cores: SIMD decode about twice as fast, encode 30–40 %
  faster.
- Legacy decode 24 % faster on a performance core and 36 % on an
  efficiency core (one pair of characters per loop iteration), faster
  than 0.2.x on both; legacy encode and the scalar SIMD level unchanged.
- NEON kernels rewritten on the same arithmetic (speed not measured).

### Changed

- The wire-format specification (spec 0008 §3.5) now asks error detection
  to cost at most 30 % of decode throughput, not 3 %: exact checks take
  four vector operations per 16 characters with this alphabet, now about
  a third of the decoder's work (+17 to +27 % measured).

---

## [0.3.0] - 2026-09-28

The driver-layer rewrite (spec 0010), with strict SIMD
decoding, an error type and an allocation-free API. Breaking. See
`docs/migrating-0.2.md`.

### Changed (breaking)

- `simd::encode` returns `Result<Vec<u8>, Error>`: a line width other than
  0 or a multiple of 32 is `Error::InvalidWidth` (it used to be accepted).
- `simd::decode` returns `Result<Vec<u8>, Error>` instead of an `Option`
  that was always `Some`.
- SIMD decoding is strict (spec 0008 §3.5): a byte outside the alphabet, a
  broken line layout, an invalid payload length or a pair worth more than
  8191 is an error, reported with its class and offset, instead of
  decoding to garbage. The legacy (Henke) format is unchanged: junk bytes
  are ignored, as the C reference does.
- `simd::Decoder::decode` and `finish` return `Result<(), Error>` (it
  returned `bool` and `()`); a decoder that failed keeps reporting its
  error.
- `simd::Encoder::new(level, wrap)` takes a line width and returns
  `Result`: the streaming encoder now wraps lines.
- `io::DecoderReader` reports an invalid SIMD stream as an `io::Error` of
  kind `InvalidData`, wrapping the `Error`.
- `base91 -d` reports an invalid SIMD stream with the error and its offset,
  and exits with status 1.
- Leading whitespace before `-` is skipped when detecting the format (spec
  0008 §2); CR LF line endings are accepted in SIMD streams.

### Added

- `Error`: the error type of the fallible functions.
- The slice API, available without an allocator: `encode_slice`,
  `decode_slice` (legacy and `simd`), and `encode_slice` / `decode_slice` /
  `finish_slice` on the streaming encoders and decoders. Sizes are checked
  before anything is written.
- `simd::encoded_len`: the exact encoded length.
- Without the `alloc` feature: the whole slice API, `Error`, the length
  functions and `simd::detect()`.
- `base91 --simd` encodes in streaming mode, in bounded memory, instead of
  reading all its input first.

### Deprecated

- `encode_unchecked` / `decode_unchecked` (crate root and `simd`): use the
  slice API. They remain until 1.0; the SIMD ones no longer copy through a
  temporary buffer. `simd::decode_unchecked` returns 0 for an invalid
  stream.

### Removed

- The `simd::scalar` module (its encoder and decoder); the SIMD level
  `SimdLevel::Scalar` covers it.

### Performance

Performance core of an Intel Core Ultra 7 165U, 1 MiB, against 0.2.3,
strict checks included: SIMD decode 9–15 % faster (AVX2 −14.5 %), wrapped
SIMD decode 5.4 times faster, scalar SIMD encode 40 % faster, streaming
wrapped decode 3.7 times faster; the legacy codec is unchanged (−2 %).

---

## [0.2.3] - 2026-09-27

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
- `pybase91` ships type stubs (`__init__.pyi`, `py.typed`), so type
  checkers see its API.

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

[Unreleased]: https://github.com/douzebis/base91/compare/v0.3.2...HEAD
[0.3.2]: https://github.com/douzebis/base91/releases/tag/v0.3.2
[0.3.1]: https://github.com/douzebis/base91/releases/tag/v0.3.1
[0.3.0]: https://github.com/douzebis/base91/releases/tag/v0.3.0
[0.2.3]: https://github.com/douzebis/base91/releases/tag/v0.2.3
[0.2.2]: https://github.com/douzebis/base91/releases/tag/v0.2.2
[0.2.1]: https://github.com/douzebis/base91/releases/tag/v0.2.1
[0.2.0]: https://github.com/douzebis/base91/releases/tag/v0.2.0
[0.1.0]: http://base91.sourceforge.net/
