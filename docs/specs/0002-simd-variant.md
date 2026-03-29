<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0002 — SIMD-accelerated basE91 variant with `--simd` flag

**Status:** draft
**App:** base91
**Implemented in:** —

## Problem

The Henke basE91 algorithm uses variable-width groups (13 or 14 bits,
data-dependent). This makes bit-group boundaries unpredictable and prevents
SIMD parallelisation. Single-threaded throughput is bounded at roughly
300–400 MB/s, far below what modern hardware can sustain, and energy
consumption per byte is correspondingly high.

A fixed-width 13-bit-per-group variant eliminates data-dependent branching.
Groups are at statically known offsets, enabling SIMD processing of 8 groups
per 128-bit register (SSE4.1) or 16 groups per 256-bit register (AVX2).
Estimated single-thread throughput: 4–7 GB/s; approximately 5× lower energy
per byte. The two variants are wire-incompatible and require an unambiguous
in-band signal for transparent decoding.

## Goals

- Define a fixed-width basE91 variant (the "SIMD variant") with:
  - A new 91-character alphabet that is contiguous in ASCII (except for the
    single exclusion of `\`) to enable arithmetic decode without a lookup
    table.
  - A single `-` prefix byte distinguishing SIMD-variant streams from Henke
    streams (hyphen is absent from the Henke alphabet).
- Add a `--simd` flag to the `base91` CLI. Without `--simd` the tool is
  byte-for-byte identical to the current implementation — no change
  whatsoever.
- Restrict `--wrap` / `-w` to multiples of 16 when `--simd` is active, so
  that line boundaries never fall inside a SIMD block.
- Implement aggressive SIMD dispatch at runtime: AVX2 > SSE4.1 > NEON >
  scalar, selected once at startup from a single binary.
- Decoder detects the variant automatically from the `-` prefix; no decoder
  flag is needed.
- No nightly Rust features (e.g. `portable_simd`) until they stabilise on
  stable.

## Non-goals

- Any change to the default (non-`--simd`) code path — it remains
  byte-for-byte Henke-compatible.
- Multi-threaded parallel encoding — deferred.
- `portable_simd` — deferred until stable.
- Changes to the Python or C API beyond exposing the new Rust entry points
  through the existing PyO3 / C-ABI layers.

## Specification

### Wire format — SIMD variant

A SIMD-variant stream begins with a single `-` byte (0x2D). The rest of
the stream is fixed-width 13-bit groups encoded two characters each, using
the SIMD alphabet defined below.

- **Encoder:** emits `-` then encodes with fixed-width 13-bit groups and
  the SIMD alphabet.
- **Decoder:** peeks at the first byte. If it is `-` (0x2D), strips it and
  decodes in fixed-width mode with the SIMD alphabet. Otherwise decodes
  in Henke mode with the Henke alphabet. Detection is a single-byte
  comparison; there is no per-block framing overhead.
- **Legacy Henke decoders** receiving SIMD-variant output will treat the
  leading `-` as an invalid character and error or corrupt — acceptable
  because the user explicitly opted in with `--simd`.

`-` is absent from the Henke alphabet (Henke excludes `'`, `-`, `\`), so
a valid Henke stream can never begin with `-`. The prefix is unambiguous.

### Alphabet — SIMD variant

The SIMD alphabet is 91 consecutive printable ASCII characters, omitting
only `\` (0x5C) at the midpoint:

```
0x23–0x5B  (57 chars):  # $ % & ' ( ) * + , - . / 0–9 : ; < = > ? @ A–Z [
0x5D–0x7E  (34 chars):  ] ^ _ ` a–z { | } ~
```

Characters in index order (index 0 = `#`, index 90 = `~`):

```
#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[]^_`abcdefghijklmnopqrstuvwxyz{|}~
```

Scalar decode uses branchless arithmetic — no table:

```rust
// index = b - 0x23 - (b > 0x5C)
// compiles to: cmp $0x5D, b; adc $-1, b; add $0xDD, b  (3 instructions)
let idx = b.wrapping_sub(0x23).wrapping_sub((b > 0x5C) as u8);
// valid iff idx < 91 (0x5C maps to 56 but is never emitted by the encoder)
```

SIMD decode maps the same two ranges without a table lookup: subtract
0x23 from every byte, then subtract 1 from bytes whose original value
was above 0x5B (compare + masked subtract — data-parallel, no scalar
branch). Invalid bytes are detected by a range check after the
subtraction.

### `--wrap` constraint with `--simd`

With `--simd`, the `-w` / `--wrap` value must be a multiple of 32.
The AVX2 kernel processes two 13-byte blocks per call, producing 32
output characters. Wrapping at a non-multiple-of-32 boundary would
split an AVX2 block across lines, preventing correct SIMD processing
of wrapped output during decoding.

If the user supplies a `--wrap` value that is not a multiple of 32
alongside `--simd`, the CLI exits with an error:

```
base91: --wrap value must be a multiple of 32 when --simd is active
```

If `--wrap` is not specified, the default (no wrapping) applies and
there is no constraint.

### SIMD block structure

The fundamental unit is a **13-byte input block → 16-character output
block** (8 groups of 13 bits, 2 characters each):

```
Input:  13 bytes  →  104 bits  →  8 × 13-bit groups
Output: 8 groups  →  16 chars  (2 chars per group)
```

13 input bytes fit in a 128-bit register (3 bytes padding to 16). 16
output characters fill a 128-bit register exactly. The final partial
block (0–12 bytes) falls through to the scalar path.

### Runtime CPU dispatch

Feature detection runs once at startup, result cached in `OnceLock<SimdLevel>`:

```
SimdLevel::Avx2   — x86_64, is_x86_feature_detected!("avx2")
SimdLevel::Sse41  — x86_64, is_x86_feature_detected!("sse4.1")
SimdLevel::Neon   — aarch64 (NEON is mandatory, always selected)
SimdLevel::Scalar — all other architectures / fallback
```

Each kernel is compiled with `#[target_feature(enable = "...")]` so the
compiler emits the correct ISA extensions regardless of the global
target CPU. The `unsafe` dispatch call is guarded by the runtime
detection result.

A Cargo feature `force-scalar` bypasses detection for benchmarking:

```toml
[features]
force-scalar = []
```

#### SSE4.1 kernel (x86_64)

Processes **one block** (13 bytes → 16 chars) per iteration using 128-bit
XMM registers.

Encode:
1. `_mm_loadu_si128` — load 13 input bytes (reads 16, top 3 ignored).
2. `_mm_shuffle_epi8` (SSSE3) with precomputed masks — gather the right
   byte pairs into 16-bit lanes for each of the 8 groups.
3. `_mm_srlv_epi16` — per-lane variable right-shift to align each 13-bit
   group to the low bits.
4. `_mm_and_si128` with constant `0x1FFF` — mask off high bits.
5. `_mm_mulhi_epu16(v, K)` + multiply-back to split `v` into `hi` and
   `lo` (replaces division by 91; K is the precomputed magic constant).
6. Character mapping: `_mm_add_epi8` to offset by 0x23, then
   `_mm_adds_epu8` + compare to add 1 for bytes above 0x5B (close the
   `\` gap). No table lookup.
7. `_mm_storeu_si128` — store 16 output bytes.

Decode reverses the pipeline: character unmap, reconstruct `v = lo + 91*hi`
via `_mm_mullo_epi16`, then pack 8 × 13-bit values back into 13 bytes via
inverse shuffle masks (OR-merging adjacent group contributions that share
a byte boundary — this scatter step is the most complex part of the
decode pipeline).

#### AVX2 kernel (x86_64)

Processes **two blocks** (26 bytes → 32 chars) per iteration using 256-bit
YMM registers — roughly 1.8–1.9× SSE4.1 throughput (not quite 2× due to
cross-lane boundary handling in `_mm256_shuffle_epi8`, which operates
independently on each 128-bit lane).

The pipeline mirrors SSE4.1 with `_mm256_*` equivalents:
`_mm256_shuffle_epi8`, `_mm256_srlv_epi16`, `_mm256_mulhi_epu16`, etc.
The two 13-byte blocks are loaded into the low and high 128-bit halves of
the YMM register independently; there is no cross-lane bit extraction.

#### NEON kernel (aarch64)

Processes **one block** (13 bytes → 16 chars) per iteration using 128-bit
Neon registers. NEON is mandatory on aarch64 — no runtime detection needed.

Key instructions:
- `vld1q_u8` — load 16 bytes (13 used).
- `vqtbl1q_u8` — table-based byte shuffle (NEON equivalent of
  `_mm_shuffle_epi8`); used for bit-group extraction.
- `vshrq_n_u16` / `vshlq_n_u16` — shift for bit alignment.
- `vandq_u16` with constant `0x1FFF` — mask.
- `vmulq_u16` / `vmlaq_u16` — multiply for `hi`/`lo` split and
  reconstruction.
- Character map: `vaddq_u8` + `vcgtq_u8` + `vsubq_u8` for gap correction.
- `vst1q_u8` — store 16 output bytes.

### Crate layout additions

```
rust/base91/src/
  simd/
    mod.rs        # SimdLevel enum, detection, dispatch, public API
    scalar.rs     # scalar fixed-width path (always compiled)
    x86.rs        # SSE4.1 + AVX2 kernels (cfg target_arch = "x86_64")
    aarch64.rs    # NEON + SVE2 kernels (cfg target_arch = "aarch64")
```

The existing `codec.rs` Henke path is untouched.

### CLI changes

- Add `--simd` boolean flag (no short form).
  - Encoding only; ignored when `--decode` / `-d` is active.
  - Documented in `--help` with a note that output is not compatible with
    legacy Henke decoders.
- Validate `--wrap` is a multiple of 32 when `--simd` is set; exit with
  error otherwise.
- The CLI passes `--wrap` directly to the encoder; no post-hoc second pass.
- Man page `base91.1` updated with a `--simd` entry and a COMPATIBILITY
  section explaining the two-format design.

### Henke API (`base91-rs`, unchanged)

```rust
// One-shot
pub fn encode(input: &[u8]) -> Vec<u8>;
pub fn decode(input: &[u8]) -> Vec<u8>;
pub unsafe fn encode_unchecked(input: &[u8], output: *mut u8) -> usize;
pub unsafe fn decode_unchecked(input: &[u8], output: *mut u8) -> usize;
pub fn encode_size_hint(input_len: usize) -> usize;
pub fn decode_size_hint(input_len: usize) -> usize;

// Streaming
pub struct Encoder;
pub struct Decoder;

// std::io adapters (feature = "io")
pub mod io {
    pub struct EncoderWriter;
    pub struct DecoderReader;
}
```

### SIMD API (`base91::simd`, new)

```rust
/// Arch-agnostic SIMD width selector.
/// Acts as a maximum-level hint: the dispatcher uses the best available
/// kernel up to this width. Simd256 on a machine without AVX2/SVE2
/// falls back to Simd128; Simd128 on a machine without SSE4.1/NEON
/// falls back to Scalar.
///
/// Mapping to arch-specific kernels:
///   Scalar  → scalar fixed-width path (all architectures)
///   Simd128 → SSE4.1 (x86_64) / NEON (aarch64)
///   Simd256 → AVX2  (x86_64) / SVE2-256 (aarch64)
pub enum SimdLevel { Scalar, Simd128, Simd256 }

impl Default for SimdLevel {
    fn default() -> Self { SimdLevel::Simd256 }
}

/// Detect the best SimdLevel available on the current CPU.
pub fn detect() -> SimdLevel;

/// Upper bound on SIMD-encoded output length.
/// `wrap=0` means no line wrapping. Accounts for newline bytes when wrap>0.
/// Must be used to size buffers before any unchecked encode call.
pub fn encode_size_hint(input_len: usize, wrap: usize) -> usize;

/// Upper bound on decoded output length.
pub fn decode_size_hint(encoded_len: usize) -> usize;

/// Encode `input` to a new Vec. Output begins with `-` then SIMD-alphabet
/// characters. `wrap=0` means no line wrapping; otherwise a `\n` is
/// inserted after every `wrap` output characters (must be a multiple of 32).
pub fn encode(input: &[u8], max_level: SimdLevel, wrap: usize) -> Vec<u8>;

/// Decode a SIMD-variant stream (leading `-`) to binary bytes.
/// Returns None if input does not start with `-`.
pub fn decode(input: &[u8], max_level: SimdLevel) -> Option<Vec<u8>>;

/// Encode into a caller-provided buffer without bounds checking.
/// Returns the number of bytes written.
///
/// # Safety
/// `output` must point to at least `encode_size_hint(input.len(), wrap)`
/// writable bytes. Writing beyond that is undefined behaviour.
pub unsafe fn encode_unchecked(
    input: &[u8],
    max_level: SimdLevel,
    wrap: usize,
    output: *mut u8,
) -> usize;

/// Decode into a caller-provided buffer without bounds checking.
/// Returns the number of bytes written, or usize::MAX if input does not
/// start with `-`.
///
/// # Safety
/// `output` must point to at least `decode_size_hint(input.len())`
/// writable bytes. Writing beyond that is undefined behaviour.
pub unsafe fn decode_unchecked(
    input: &[u8],
    max_level: SimdLevel,
    output: *mut u8,
) -> usize;

/// Streaming SIMD encoder. Wrap is built in — no second pass.
pub struct Encoder;
impl Encoder {
    pub fn new(max_level: SimdLevel, wrap: usize) -> Self;
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>);
    pub fn finish(self, output: &mut Vec<u8>);
}

/// Streaming SIMD decoder. Auto-detects Henke vs SIMD from first byte.
pub struct Decoder;
impl Decoder {
    pub fn new(max_level: SimdLevel) -> Self;
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) -> bool;
    pub fn finish(self, output: &mut Vec<u8>);
}
```

### Benchmarks

Extend `benches/throughput.rs` with:

- `encode_simd_{scalar,simd128,simd256}` — per-level throughput.
- `decode_simd_{scalar,simd128,simd256}` — decode throughput.
- Compared against existing Henke `encode`/`decode`.

Target acceptance criteria (single thread, x86_64 AVX2):
- Encode: ≥ 5 GB/s
- Decode: ≥ 3.5 GB/s

### Compatibility matrix

| Producer | Consumer | Result |
|---|---|---|
| Henke encoder | Henke decoder | correct (unchanged) |
| Henke encoder | new decoder | correct (`-` absent → Henke path) |
| `--simd` encoder | new decoder | correct (`-` detected → SIMD path) |
| `--simd` encoder | legacy Henke decoder | error / corrupt (expected; user opted in) |

## Open questions

- Should `simd::Encoder` become the default in a future major version, with
  `--henke` for legacy compatibility? Deferred.
- SVE2 kernel not yet implemented; `Simd256` on aarch64 currently falls back
  to `Simd128` (NEON).
- LLVM missing peephole: `cmp` + `adc $-1` + `add $0x24` in `enc_char` and
  `cmp` + `adc $-1` + `lea` in `dec_char` could each be two instructions
  (`cmp` + `sbb`) when inlined. Tracked for future inline-asm optimization.
