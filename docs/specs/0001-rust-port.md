<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0001 — Rust port of basE91: CLI, libraries, and nixpkgs packaging

**Status:** implemented
**App:** base91
**Implemented in:** 2026-03-28

## Problem

The existing basE91 implementation is a C project with a hand-written
Makefile, no machine-readable API, and no package on crates.io.
Consumers who want to embed basE91 in Rust projects must either call the
C code via FFI or reimplement the algorithm themselves.  The nixpkgs
package ships only the compiled binary; there is no Rust crate that
nixpkgs or crates.io users can depend on.

Additionally, the current CLI tool (`base91`) lacks:
- shell auto-completions (bash, zsh, fish)
- a man page generated from structured metadata (the existing `.1` is
  hand-maintained and drifts)
- a stable symlink convention discoverable by tooling

## Goals

- Pure-Rust reimplementation of the basE91 algorithm, byte-for-byte
  compatible with the reference C implementation.
- Three publishable libraries from a single implementation:
  - `base91` — Rust crate on crates.io
  - `pybase91` — PyO3 Python extension (publishable to PyPI)
  - `libbase91.h` / `libbase91.so` — C-compatible shared/static library
- CLI binary `base91` (with symlinks `b91enc` and `b91dec`) with full
  feature parity with the C CLI, generated man pages, and shell
  auto-completions.
- Nix derivation using crane for reproducible builds, suitable for
  submission to nixpkgs as a drop-in replacement for the existing
  `base91` package.
- Performance at least on par with the C reference (verified by
  benchmark).
- All existing C test-suite semantics covered by Rust tests.

## Non-goals

- Changing the basE91 alphabet or wire format — full compatibility with
  the reference implementation is required.
- Async API — the algorithm is CPU-bound streaming; no async wrapper
  is planned.
- WASM/wasm-bindgen target — deferred.
- Python packaging to PyPI — the PyO3 wheel is built and tested but
  publishing is out of scope for this spec.
- Windows binary distribution — the Nix build targets Unix; Windows
  support may be added later.
- A new encoding variant (e.g. URL-safe alphabet) — out of scope.

## Specification

### 1. Repository layout

The Rust port lives under `rust/` at the repo root; the original C
source under `src/` is preserved unchanged.

```
base91/
  src/                        # original C source (unchanged)
  rust/
    Cargo.toml                # workspace root
    base91/                   # Rust library + C FFI + PyO3 bindings
      Cargo.toml
      build.rs                # generates C header via cbindgen
      cbindgen.toml           # cbindgen configuration
      src/
        lib.rs                # public Rust API + no_std core
        codec.rs              # encoding/decoding algorithm
        c_api.rs              # #[no_mangle] C-compatible exports
        python.rs             # PyO3 module (feature-gated)
      include/
        base91.h              # generated C header (committed)
    base91-cli/               # thin CLI shell
      Cargo.toml
      build.rs                # embeds man page, triggers completion gen
      src/
        main.rs               # clap entry point + completion dispatch
        cli/
          encode.rs
          decode.rs
  docs/
    specs/
      0001-rust-port.md       # this file
  REUSE.toml
  default.nix                 # crane-based Nix derivation
  shell.nix
```

### 2. Algorithm correctness

The Rust implementation must be a faithful translation of
`src/base91.c`.  Specifically:

- Encoding table: `enctab[91]` — the 91-character printable ASCII
  alphabet defined in the C source, in the same order.
- Decoding table: `dectab[256]` — the 256-entry reverse-lookup table,
  identical byte-for-byte.
- Encoding loop: bit-queue accumulation, 13-vs-14-bit branch on
  `val > 88`, two output bytes per cycle.
- Flush: `encode_end` emits 1 or 2 bytes from remaining queue bits.
- Decoding: pair-wise character consumption; non-alphabet bytes are
  silently ignored; `decode_end` emits at most 1 byte.
- State: `queue: u32`, `nbits: u32`, `val: i32` (−1 = no pending
  value).  These map directly from the C `basE91` struct.

**Compatibility test:** a round-trip property test
(`encode(decode(x)) == x` and `decode(encode(x)) == x`) is run against
all 256 single-byte inputs and against the four test vectors from the
original `test.sh` (reproduced as binary fixtures in
`rust/base91/tests/fixtures/`).

A cross-implementation fuzz test drives the Rust encoder with random
inputs and compares output to the C reference compiled as a test helper
(`tests/c_compat/`).  This test is gated behind feature
`c-compat-tests` and requires a C compiler.

### 3. Rust library API (`base91` crate)

#### 3.1 Streaming (stateful) API

Mirrors the C struct-based API for callers that feed data in chunks.

```rust
/// Stateful encoder. Feed input in chunks; call `finish()` to flush.
pub struct Encoder {
    queue: u32,
    nbits: u32,
}

impl Encoder {
    pub fn new() -> Self;

    /// Encode `input` bytes, appending encoded output to `output`.
    /// Returns the number of bytes written.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) -> usize;

    /// Flush remaining bits. Returns 0, 1, or 2 trailing bytes.
    pub fn finish(self, output: &mut Vec<u8>) -> usize;
}

/// Stateful decoder. Feed encoded text in chunks; call `finish()` to flush.
pub struct Decoder {
    queue: u32,
    nbits: u32,
    val: i32,
}

impl Decoder {
    pub fn new() -> Self;

    /// Decode `input` bytes (non-alphabet bytes silently skipped),
    /// appending binary output to `output`.
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) -> usize;

    /// Flush any remaining partial value. Returns 0 or 1 byte.
    pub fn finish(self, output: &mut Vec<u8>) -> usize;
}
```

Both types implement `Default`.

#### 3.2 One-shot helpers

For callers with the entire input in memory:

```rust
/// Encode all of `input` to a new `Vec<u8>`.
pub fn encode(input: &[u8]) -> Vec<u8>;

/// Decode all of `input` to a new `Vec<u8>`.
/// Non-alphabet bytes are silently skipped (matching C behavior).
pub fn decode(input: &[u8]) -> Vec<u8>;
```

#### 3.3 `std::io` adapters (feature `io`, default-on)

```rust
/// Wraps a `Write` sink; data written to `EncoderWriter` is encoded
/// and forwarded to the inner writer.
pub struct EncoderWriter<W: Write> { /* private */ }

impl<W: Write> EncoderWriter<W> {
    pub fn new(inner: W) -> Self;
    /// Flush encoder state and return the inner writer.
    pub fn finish(self) -> io::Result<W>;
}

impl<W: Write> Write for EncoderWriter<W> { … }

/// Wraps a `Read` source; data read from `DecoderReader` is decoded
/// from the inner reader.
pub struct DecoderReader<R: Read> { /* private */ }

impl<R: Read> DecoderReader<R> {
    pub fn new(inner: R) -> Self;
    pub fn into_inner(self) -> R;
}

impl<R: Read> Read for DecoderReader<R> { … }
```

#### 3.4 Crate feature flags

| Feature | Default | Purpose |
|---|---|---|
| `io` | yes | `EncoderWriter` / `DecoderReader` (requires `std`) |
| `std` | yes | Pulls in `std`; disable for `no_std` environments |
| `python` | no | PyO3 Python module |
| `c-compat-tests` | no | Cross-checks against compiled C reference |

With `default-features = false` the crate compiles under `no_std`
(algorithm core only, no I/O adapters).

#### 3.5 Performance targets

- Encoding throughput ≥ 1 GiB/s on a modern x86-64 core (single
  thread, in-memory buffers).
- Decoding throughput ≥ 1 GiB/s under the same conditions.
- Measured with `criterion` benchmarks in `base91/benches/`.
- The Rust one-shot `encode`/`decode` paths must not be slower than
  the C reference at `-O2` on equivalent hardware.

See `docs/perf-analysis.md` for the full disassembly study and
reasoning.  Summary of findings and implementation directives:

**Algorithm constraints:**
- Serial bit-queue state machine; loop-carried dependency on `queue`
  and `nbits` makes SIMD impossible.
- Each output pair encodes 13 or 14 bits (data-dependent), so fixed
  block sizes (e.g. 13 bytes → 16 chars) do not hold in general.
- The realistic goal is to **marginally beat the C reference at -O2**,
  not a dramatic speedup.

**Technique 1 — Register-hoist state (primary win):**
GCC cannot prove that the output buffer does not alias the `basE91`
struct, so it reloads `queue` and `nbits` from memory on every loop
iteration.  Rust's ownership system gives LLVM that proof for free.
The implementation must exploit this by keeping `queue`, `nbits`, and
`val` in local variables for the duration of the hot loop, writing
back to `&mut self` only on exit:
```rust
let mut queue = self.queue;
let mut nbits = self.nbits;
// ... hot loop ...
self.queue = queue;
self.nbits = nbits;
```
This is the single most important optimization and is essentially free.

**Technique 2 — Single division for /91 (confirm with assembly):**
GCC already emits a multiply-shift sequence for `/ 91`.  LLVM will
too.  Write it as one division + multiply-subtract so the intent is
clear and a single multiply-shift is emitted:
```rust
let q = val / 91;
let r = val - q * 91;
// ENCTAB[r as usize], ENCTAB[q as usize]
```
**Verify in the disassembly** that LLVM emits `imul` + `shr`, not
`idiv`.

**Technique 3 — Branchless 13/14 selection (decode):**
GCC emits a beautiful `cmp + adc` sequence for the 13/14-bit branch:
```asm
cmp  edx, 0x59   ; sets carry if val&0x1fff < 89 (i.e. ≤ 88)
adc  ecx, 0xd    ; nbits += 13 + carry  (14 if ≤88, 13 if >88)
```
Write the Rust decode path so LLVM can emit the same.  The natural
expression is:
```rust
nbits += if val & 0x1fff > 88 { 13 } else { 14 };
```
**Verify in the disassembly** that LLVM emits `cmp + adc` (or `cmov`),
not a branch.

**Technique 4 — Unroll decode drain loop:**
GCC still emits a loop for the drain.  Replace it with a fixed
two-emit sequence (trip count is at most 2):
```rust
// After queue |= val << nbits; nbits += 13 or 14;
// nbits is now 13–27; always at least one full byte:
out.push(queue as u8); queue >>= 8; nbits -= 8;
if nbits >= 8 {
    out.push(queue as u8); queue >>= 8; nbits -= 8;
}
```

**What not to do:**
- No large lookup tables (16–32 KB): evicted by context switches;
  data-dependent indexing defeats hardware prefetch.
- No SIMD: the loop-carried state dependency chain cannot be
  vectorized.

### 4. C library (`libbase91`)

#### 4.1 ABI compatibility

The C library is a **drop-in replacement** for Joachim Henke's
`base91.h` / `base91.c`.  The struct layout and all five function
signatures are identical; existing C consumers relink without any source
changes.

`cbindgen` generates `include/base91.h` from `c_api.rs`:

```c
/* State struct — identical layout to the C reference:
   unsigned long queue; unsigned int nbits; int val; */
typedef struct basE91 basE91;

void   basE91_init      (basE91 *b);
size_t basE91_encode    (basE91 *b, const void *i, size_t l, void *o);
size_t basE91_encode_end(basE91 *b, void *o);
size_t basE91_decode    (basE91 *b, const void *i, size_t l, void *o);
size_t basE91_decode_end(basE91 *b, void *o);
```

The struct is exposed (not opaque) so that callers can stack-allocate it
exactly as they do with the C reference.

#### 4.2 Header installation

`cbindgen` writes `include/base91.h` during `build.rs`.  The Nix
derivation installs it to `$out/include/base91.h` alongside
`$out/lib/libbase91.{a,so}`.

### 5. Python bindings (`pybase91` / PyO3)

The Python module is compiled from `base91/src/python.rs` when the
`python` feature is enabled.  It is built as a separate cdylib target
named `pybase91` (to avoid colliding with the `base91` Rust crate name
on import).

#### 5.1 Python API

```python
import pybase91

# One-shot (accepts bytes or bytearray):
encoded: bytes = pybase91.encode(b"Hello, world!")
decoded: bytes = pybase91.decode(encoded)

# Streaming encoder:
enc = pybase91.Encoder()
enc.update(b"chunk1")
enc.update(b"chunk2")
result: bytes = enc.finish()   # flushes remaining bits; enc is consumed

# Streaming decoder:
dec = pybase91.Decoder()
dec.update(encoded_chunk)
result: bytes = dec.finish()   # flushes partial value; dec is consumed
```

`update()` returns `None`; partial output is buffered internally and
returned only on `finish()`.  This keeps the API simple: callers
accumulate chunks then collect the result in one call.

#### 5.2 Error handling

`encode` / `decode` / `Encoder.update` / `Decoder.update` never raise.
`finish()` never raises.  Non-alphabet bytes are silently discarded by
the decoder (same as the C reference).

#### 5.3 Type stubs (.pyi generation)

Type stubs are generated automatically via `pyo3-stub-gen` (same
pattern as `prototools`).

**Cargo.toml dependencies** (in the `base91` crate when `python`
feature is active):

```toml
pyo3             = { version = "0.26", features = ["extension-module"] }
pyo3-stub-gen    = "0.16"
pyo3-stub-gen-derive = "0.16"
```

**Annotations** — every exported item is decorated with the matching
derive macro alongside its PyO3 attribute:

```rust
use pyo3_stub_gen::derive::{gen_stub_pyclass, gen_stub_pymethods, gen_stub_pyfunction};

#[gen_stub_pyclass]
#[pyclass]
pub struct Encoder { … }

#[gen_stub_pymethods]
#[pymethods]
impl Encoder { … }

#[gen_stub_pyfunction]
#[pyfunction]
fn encode(data: &[u8]) -> Vec<u8> { … }
```

**`stub_info()` function** — exported from `python.rs` so the
post-build binary can link it:

```rust
pub fn stub_info() -> pyo3_stub_gen::Result<pyo3_stub_gen::StubInfo> {
    pyo3_stub_gen::StubInfo::from_project_root(
        "pybase91".to_string(),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    )
}
```

**Post-build binary** (`rust/base91/src/bin/post_build.rs`):

```rust
use base91::stub_info;

fn main() -> pyo3_stub_gen::Result<()> {
    stub_info()?.generate()?;
    Ok(())
}
```

Running `cargo run --release --bin post_build` (after building the
cdylib) writes `pybase91.pyi` into the crate directory.

The generated `pybase91.pyi` is committed to the repository and
installed alongside the `.so` by the Nix derivation.

#### 5.4 Build

**Build tool: maturin.**  `pyproject.toml` declares `maturin` as the
build backend.  This makes the same source tree work for both:

- `pip install .` / `maturin develop` — local development
- `maturin publish` / `maturin-action` GitHub Actions — PyPI wheel
  publishing (`manylinux`, macOS, Windows)
- `nix build .#pybase91` — reproducible Nix build via
  `buildPythonPackage` with `maturinBuildHook`

The `python` feature is not included in the default feature set of the
`base91` crate on crates.io; it is only activated by maturin via
`[tool.maturin] features = ["python"]`.

### 6. CLI (`base91-cli` crate)

#### 6.1 Commands and flags

The CLI binary is installed as `base91`.  Symlinks `b91enc` and `b91dec`
are created at install time (via the Nix derivation and `Makefile`-like
install target).  When invoked as `b91enc`, encode mode is the default;
as `b91dec`, decode mode is the default — matching the C behavior.

Flags (full parity with the C CLI):

| Flag | Long form | Default | Description |
|---|---|---|---|
| `-d` | `--decode` | off | Decode instead of encode |
| `-e` | `--encode` | on | Encode (explicit override of `-d`) |
| `-o FILE` | `--output=FILE` | stdout | Write output to FILE |
| `-v` | `--verbose` | off | Print stats to stderr; repeat for extra verbosity |
| `-w COLS` | `--wrap=COLS` | 76 (enc), 0 (b91enc) | Wrap encoded lines at COLS chars; 0 = no wrap |
| `-m SIZE` | — | 64K | Buffer size (suffixes: b, K, M) |
| — | `--help` | — | Print help and exit |
| — | `--version` | — | Print version and exit |

Positional argument: optional `FILE` (default stdin).  `-` means stdin.

#### 6.2 Man page

Generated at build time from the clap model via `clap_mangen`.  The
resulting `base91.1` is written to `man/man1/base91.1` and installed to
`$out/share/man/man1/`.

The generated man page supersedes the hand-written `src/base91.1`.

#### 6.3 Shell completions

Generated at build time for bash, zsh, fish, elvish, and PowerShell via
`clap_complete`.  Completion files are written to
`completions/<shell>/` and installed to the appropriate
`$out/share/<shell-completion-dir>/` paths by the Nix derivation.

Shell completion dispatch: when the `BASE91_COMPLETE=<shell>` environment
variable is set, the binary prints completions for the named shell and
exits (same pattern as `clap_complete::CompleteEnv`).

#### 6.4 Verbose output

`-v` (once): prints `encoding <file> ...` or `decoding <file> ...` to
stderr before processing; on completion prints the ratio (e.g.
`114.23%`) for encoding or `done` for decoding.

`-vv` (twice): additionally prints the resolved buffer sizes in bytes.

#### 6.5 Buffer sizing

The CLI pre-allocates a single buffer of `SIZE` bytes split between
input and output using the same overlapping-buffer arithmetic as the C
CLI:

- Encode: `ibuf_size = (SIZE - 2) * 16 / 29`
- Decode: `ibuf_size = (SIZE - 1) * 8 / 15`

Minimum `SIZE` is 4 for encoding, 3 for decoding; the CLI exits with an
error message if the value is too small.

### 7. Nix derivation

#### 7.1 Crane build

`default.nix` uses crane (pinned via `flake-compat` or direct fetchTarball)
to build the Rust workspace.  Exported outputs:

| Output | Description |
|---|---|
| `default` (= `base91`) | CLI binary + man page + completions |
| `libbase91` | C shared and static libraries + header |
| `pybase91` | Python extension `.so` |
| `rust-tests` | `cargo test` derivation (tier-1 unit tests) |
| `rust-clippy` | `cargo clippy -- -D warnings` |
| `rust-fmt` | `cargo fmt --check` |
| `dev-shell` | Development shell |

#### 7.2 Dev shell

The dev-shell provides:
- Rust toolchain (stable, via `rustup` or `pkgs.rustup`)
- `cbindgen` (for regenerating the C header)
- `python3` + `maturin` (for building/testing the PyO3 wheel locally)
- `reuse` (REUSE/SPDX compliance tool)
- `cargo` build on entry; `rust/target/release` on PATH
- `NIXSHELL_REPO = toString ./.;` export for hook compatibility

#### 7.3 nixpkgs compatibility

The derivation is structured to be submittable as a nixpkgs PR that
updates the existing `pkgs/tools/misc/base91/` package:

- `pname = "base91"`, `version = "0.2.0"` (first Rust release, minor
  bump from the C `0.1.0` Nix package version).
- `buildInputs` / `nativeBuildInputs` follow nixpkgs conventions.
- `meta.license` includes both `lib.licenses.bsd3` (original C
  algorithm, Joachim Henke) and `lib.licenses.mit` (Rust port,
  Frederic Ruget).
- `meta.maintainers` lists the new maintainer.

### 8. Licensing and REUSE compliance

The Rust code is a clean rewrite; it carries a single copyright:

| Path | Copyright | License |
|---|---|---|
| `src/*` | 2006 Joachim Henke | BSD-3-Clause (unchanged) |
| `rust/**` | 2026 Frederic Ruget | MIT |
| All other new files | 2026 Frederic Ruget | MIT |

Joachim Henke is credited in `README.md` as the inventor of the basE91
algorithm and author of the reference implementation; his name does not
appear in Rust source file headers.

`REUSE.toml` is updated to reflect the new paths.  `reuse lint` must
pass 100% before any commit that touches headers.

### 9. Testing strategy

#### 9.1 Unit tests (always run, no system deps)

- Property tests (`proptest` or `quickcheck`): `decode(encode(x)) == x`
  for random byte slices of length 0–65536.
- Table correctness: `enctab` and `dectab` are inverses over the valid
  alphabet.
- Edge cases: empty input, single byte, exact multiples of 13/14 bits.
- Streaming vs. one-shot consistency: identical output regardless of
  chunk boundaries.
- Buffer-size sweep: split input at every possible boundary; assert same
  final output.

#### 9.2 Reference-vector tests

The four binary test files from `src/test/test.sh` (reproduced as
`rust/base91/tests/fixtures/`) are encoded and decoded; output is
compared byte-for-byte against pre-computed expected values (the
checksums from `test.sh`).

#### 9.3 C cross-check (feature `c-compat-tests`)

A test compiles `src/base91.c` via `cc` crate in `build.rs` and links
it into the test binary.  Random inputs are encoded by both the C and
Rust implementations; outputs are compared byte-for-byte.

#### 9.4 CLI subprocess tests

Integration tests in `base91-cli/tests/` invoke the compiled binary
as a subprocess (similar to yb's pattern in `0009-cli-subprocess-tests`):
- round-trip encode/decode of arbitrary binary data via pipes
- `-w` / `--wrap` produces correctly-wrapped lines
- symlink invocation (`b91enc`, `b91dec`) selects the correct default mode
- `-o FILE` writes output to the named file
- `-v` writes statistics to stderr

#### 9.5 Benchmarks

`criterion` benchmarks in `rust/base91/benches/throughput.rs`:
- `encode_1mib` / `decode_1mib`: 1 MiB random data, one-shot
- `encode_streaming_64k` / `decode_streaming_64k`: 1 MiB data fed in
  64 KiB chunks via the streaming API
- `encode_small` / `decode_small`: 64-byte inputs (latency-sensitive path)

Benchmarks are excluded from `cargo test` and run only with
`cargo bench`.

### 10. Versioning and publishing

#### 10.1 crates.io

- Library crate name: `base91.rs` (`base91` is taken by an abandoned crate).
- CLI crate name: `base91-cli` (available).
- Both start at `0.2.0`, version-locked until API stabilises at 1.0.
- Published via `cargo publish`.
- `--version` output matches the crate version.
- `CHANGELOG.md` maintained in Keep a Changelog format.

#### 10.2 PyPI

- Package name: `pybase91` (available).
- Build tool: `maturin` — already the build backend in `pyproject.toml`.
- GitHub Actions workflow using `maturin-action` builds wheels for all
  target triples and uploads on tag push (follow-up spec).

#### 10.3 nixpkgs

Three separate packages (each a first-class linkable artifact):

| nixpkgs attribute | content | PR strategy |
|---|---|---|
| `base91` | CLI binaries + man(1) + completions | update existing package |
| `libbase91` | `libbase91.{so,a}` + `base91.h` + man(3) | new package |
| `python3Packages.pybase91` | PyO3 extension + `.pyi` stubs | new package |

The existing `pkgs/by-name/ba/base91/package.nix` already points to
`douzebis/base91`; it is updated from the C/Make build to
`rustPlatform.buildRustPackage` targeting `base91-cli`.

## Open questions

- Should `base91-cli` and `base91` share the same version number, or
  version independently?  Current proposal: lock them together until
  the API stabilizes at 1.0.
- `EncoderWriter` / `DecoderReader` use an 8 KiB internal buffer;
  not user-configurable at v0.2.  Resolved.
- `cbindgen` vs hand-written C header: cbindgen is the plan, but the
  generated output must be reviewed for ABI stability on each release.
- GitHub Actions workflow for maturin-based PyPI publishing — deferred
  to a follow-up spec, but the build backend is already maturin.
- Should `codec.rs` be `no_std` by default?  Answer: yes, the core
  algorithm has no `std` dependency; gate only the I/O adapters.

## References

- Original C source: `src/base91.c`, `src/cli.c`
- Original man page: `src/base91.1`
- Upstream: http://base91.sourceforge.net/
- yb crate-structure spec (structural model): `../yb/docs/specs/0002-crate-structure.md`
- yb CLI spec (CLI patterns): `../yb/docs/specs/0007-cli-improvements.md`
- cbindgen: https://github.com/mozilla/cbindgen
- PyO3: https://pyo3.rs/
- clap_mangen: https://docs.rs/clap_mangen/
- clap_complete: https://docs.rs/clap_complete/
- criterion: https://bheisler.github.io/criterion.rs/book/
- REUSE spec: https://reuse.software/spec/
