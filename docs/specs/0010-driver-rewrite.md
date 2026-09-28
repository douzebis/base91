<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0010 — Driver-layer rewrite: in-place layout, strict decoding, slice API (0.3.0)

**Status:** draft
**App:** base91
**Implemented in:** —

Supersedes spec 0007 (allocation-free API), whose content is folded in
here. Implements the parts of spec 0008 that 0.2.x does not (annex A2, A4,
A9–A13b, A14). To be implemented **after 0.2.3** (specs 0004, 0005, 0006).

## Problem

The SIMD kernels of `base91-rs` are correct: the spec 0009 test suite checks
them against an independent oracle on SSE4.1, AVX2 and NEON. Everything
**around** them, the driver layer, is where the problems are:

- **Every bug found in 0.2.2** is in the driver layer: wrapping (spec 0006
  bug 3), streaming (bugs 1, 2, 4), buffer handling (spec 0005; bug 5), the
  `io` reader (bug 6).
- **Wrapping is bolted on after the fact.** The encoder encodes flat, then
  shifts the output right to open gaps for the newlines
  (`insert_wrap_newlines`). The one-shot decoder copies the whole input
  through a byte-by-byte filter to remove them; the streaming decoder splits
  each chunk on newlines. Measured on 4 MiB at `Simd256` in 0.2.2: encode
  1.46 ms unwrapped, 4.90 ms at W = 64 (+235 %); decode 1.24 ms unwrapped,
  6.82 ms at W = 64 (+450 %). Even a `memcpy` per line would cost 2.5× the
  whole decode.

  0.2.3 (spec 0006) moves whole lines instead of bytes when wrapping, and
  carries the streaming state across chunks. Measured on 1 MiB, one pinned
  vCPU: encode 325 µs unwrapped, 425 µs at W = 64 (+31 %); one-shot decode
  296 µs unwrapped, 1.69 ms at W = 64 (+470 %, unchanged); streaming decode
  302 µs unwrapped, 871 µs at W = 64. Wrapped SIMD decoding, the CLI
  default, still runs at a fraction of unwrapped speed. The format was
  designed so that this costs nothing (W a multiple of 32, spec 0008 §3.4);
  the code never uses it.
- **The kernels' validity check misses most invalid bytes.** It compares
  the computed index with 90 using a *signed* byte comparison, so every
  byte whose index wraps below zero passes: all bytes below `0x23`
  (including CR, LF and space) and most bytes from `0x80`. `'` (`0x27`,
  outside the alphabet) maps to the index of `&` and passes too. Checked
  on 0.2.3: CR, space, `'` and `0xFF` inside a block are accepted by the
  SSE4.1 and AVX2 kernels; `0x7F` is rejected. (The comments in
  `decode_block_*` claim these bytes are caught.)
- **Invalid input silently becomes garbage** (spec 0008 annex A9–A13b):
  no errors, CR LF and leading whitespace mishandled, pairs above 8191
  spilled into the next group.
- **The API has no safe way to use caller memory** (only the `unsafe`
  `*_unchecked` functions, the SIMD ones unsound in 0.2.2), and nothing
  works without `alloc`.

Keeping the kernels unaware of the layout is right; the mistake is handling
the layout by moving data afterwards instead of in the loop that drives the
kernels. This spec replaces that loop.

## Goals

- **One driver** per direction, layout-aware, used by every API (one-shot,
  streaming, slice, `Vec`, `unchecked`, `io`), writing directly into the
  caller's memory.
- **In-place wrapping, both directions** (spec 0008 §3.4): no insertion
  pass, no compaction, no copy. Target: wrapped throughput within 10 % of
  unwrapped.
- **Strict SIMD decoding** (spec 0008 §2, §3.5): classification after
  whitespace, CR LF, and the four error classes, with error detection
  costing at most 3 % on valid input.
- **Allocation-free API** (from spec 0007): a safe slice-based API for both
  formats, available without `alloc`.
- **Streaming in bounded state** (spec 0008 §3.6), including wrapped SIMD
  encoding, which 0.2.x lacks.
- No throughput regression on unwrapped streams versus 0.2.3.

## Non-goals

- **The kernels themselves** (their speed, AVX2 vs SSE4.1, spec 0002's
  5 / 3.5 GB/s targets): spec 0011. The kernel interface only changes where
  this spec says so (§5).
- AVX-512 / SVE2 kernels (deferred by spec 0008 §3.4).
- SIMD support in `pybase91` (it only exposes the legacy format; a separate
  question).
- A lenient SIMD decoding mode (spec 0008 decisions: strict only).

## Decisions recorded (2026-09-27)

1. 0.2.3 ships specs 0004, 0005 and 0006 first; this rewrite is 0.3.0.
2. **Fallible SIMD functions return `Result<_, Error>`**, replacing today's
   `Option` that is always `Some` and the `bool` of `simd::Decoder::decode`.
   Breaking, and meant to be: the compiler points at every caller that must
   now handle errors. The legacy `base91::decode` stays infallible (by the
   legacy rules, junk is ignored).
3. **`*_unchecked` are deprecated** in favor of the safe slice API
   (`#[deprecated(since = "0.3.0")]`), reimplemented on the new core so they
   stay correct and zero-copy until their removal in 1.0.
4. **`simd::scalar` becomes `pub(crate)`.** Nothing outside the module uses
   it (checked: CLI, Python, benchmarks, docs); it offers nothing that
   `SimdLevel::Scalar` does not, and it is how spec 0006 bug 2 was
   reachable.
5. The AVX2 / kernel-speed question is tracked separately (spec 0011).
6. **Slice API naming: `encode_slice` / `decode_slice` / `finish_slice`**,
   as in the `base64` crate (`Engine::encode_slice`), rather than spec
   0007's `*_to_slice`.
7. **The 3 % validation budget (§4) is measured once**, with a temporary
   patch that disables the checks (reverted, result recorded here), not
   with a permanent feature.

   *Measured (2026-09-28, stage 1, 1 MiB, one pinned vCPU, two
   alternating rounds):* the checks cost far more than 3 %, and every part
   of them costs several percent:

   | Checks | SSE4.1 decode | AVX2 decode |
   |---|---|---|
   | none | 222 µs | 194 µs |
   | character range only | 238 µs | 209 µs |
   | + `'` | 251 µs | 226 µs |
   | + pairs above 8191 (all checks) | 278 µs (+25 %) | 251 µs (+29 %) |

   Wrapped (W = 64, AVX2): 276 µs with the checks, 198 µs without
   (+39 %). The cost is out of proportion to the handful of vector
   operations involved, which points to the code-generation sensitivity
   recorded in spec 0011. *Decided:* stage 1 ships with the checks as they
   are (still faster than 0.2.3 everywhere, see *Stage 1 results*); the
   3 % target moves to spec 0011.
8. **Stage 1 maps errors onto today's API**: invalid input makes
   `simd::decode` return `None` and `simd::Decoder::decode` return `false`
   (instead of garbage) until stage 2 introduces `Result`. Only 0.3.0, with
   all stages, is released.
9. **One commit per stage**, each with every check passing. Stage 1
   starts with the kernel validation fix (§5) and its tests.

## Specification

### 1 — Error type

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Output buffer shorter than the size required for this call.
    BufferTooSmall { needed: usize },
    /// Encoder line width is neither 0 nor a multiple of 32 (spec 0008 §3.4).
    InvalidWidth { width: usize },
    /// Spec 0008 §3.5 error classes. `offset` is the position in the whole
    /// input (or stream, when streaming) of the first offending byte.
    InvalidCharacter { offset: usize },
    InvalidLayout { offset: usize },
    InvalidLength,
    InvalidValue { offset: usize },
}
```

It implements `Display` and `core::error::Error` (MSRV 1.91). Offsets are
always reported, at no cost on valid input: the fast path only notes that
*some* block failed; a scalar re-scan of the failing region then finds the
first offending byte and its class. The classes and offsets are the ones the
spec 0009 oracle computes (`base91_testkit::oracle::DecodeError`), so tests
compare them exactly.

### 2 — Public API (0.3.0)

**SIMD module:**

| Function | Change |
|---|---|
| `simd::encode(input, level, wrap) -> Result<Vec<u8>, Error>` | was `Vec<u8>`; `InvalidWidth` |
| `simd::decode(input, level) -> Result<Vec<u8>, Error>` | was `Option<Vec<u8>>`; strict |
| `simd::encode_slice(input, level, wrap, out: &mut [u8]) -> Result<usize, Error>` | new, no `alloc` |
| `simd::decode_slice(input, level, out: &mut [u8]) -> Result<usize, Error>` | new, no `alloc` |
| `simd::encoded_len(n, wrap) -> Result<usize, Error>` | new: E(n, W), exact (spec 0008 §3.4) |
| `simd::encode_size_hint`, `simd::decode_size_hint` | kept (upper bounds) |
| `simd::Encoder::new(level, wrap) -> Result<Self, Error>` | was `new(level)`; **gains wrapping** |
| `simd::Encoder::{encode, encode_slice, finish, finish_slice}` | slice variants new |
| `simd::Decoder::decode(&mut self, input, &mut Vec) -> Result<(), Error>` | was `bool`; strict |
| `simd::Decoder::{decode_slice, finish, finish_slice}` | `finish` → `Result`: the last line and the payload length are only validated at the end |
| `simd::encode_unchecked`, `simd::decode_unchecked` | deprecated; core-based, zero-copy |
| `simd::scalar` | `pub(crate)` |

**Crate root (legacy):**

| Function | Change |
|---|---|
| `encode`, `decode`, `Encoder`, `Decoder` | unchanged |
| `encode_slice`, `decode_slice`, `Encoder::encode_slice` / `finish_slice`, `Decoder::decode_slice` / `finish_slice` | new, no `alloc`; only `BufferTooSmall` |
| `encode_unchecked`, `decode_unchecked` | deprecated; core-based |

**`io`:** `DecoderReader` returns `io::Error` of kind `InvalidData`, wrapping
`Error`, for an invalid SIMD stream. Legacy streams never fail.

**Slice-API rules** (from spec 0007):
- Each call checks `out.len()` against the required size **before writing
  anything**. On error, `out` and the encoder/decoder state are untouched.
- Streaming `finish_slice` takes `&mut self`, so that `BufferTooSmall`
  does not consume the state. On success it resets the state.
- The required sizes are the size hints (upper bounds), except
  `encode_slice`, which needs exactly E(n, W).

**Without `alloc`** (spec 0004 feature layout): the whole slice API, the
streaming types' slice methods, `Error`, the length functions, and
`simd::detect()`.

### 3 — Core: one driver per direction, writing into `&mut [MaybeUninit<u8>]`

From spec 0007 §3, kept: every API layer is a thin wrapper around one core
per direction.
- **`Vec` layer:** `reserve`, run the core on `spare_capacity_mut()`, then
  `set_len`.
- **Slice layer:** check the size, then run the core. `&mut [u8]` becomes
  `&mut [MaybeUninit<u8>]` in one private helper; the cores only write
  initialized bytes.
- **Unchecked layer:** builds the slice from the raw pointer and the
  documented size.

No `reserve`, no `Vec` and no allocation below the API layer.

**Encode driver** (one-shot and streaming share it):
- Output positions are closed-form: `-`, LF if W > 0, then line k starts at
  `2 + k·(W + 1)`. Each 16-character block (13 input bytes) is written by a
  kernel **directly at its final offset**; after W/16 blocks, the driver
  writes one LF.
- AVX2 processes two blocks (a 32-character unit) per call; W is a multiple
  of 32, so a unit never spans a line end. (A streaming call may leave a
  line at an odd block; the driver then emits one 16-character block before
  resuming 32-character units.)
- The tail (fewer than 13 bytes) goes through the scalar path, then the
  final LF (spec 0008 §3.4).
- Streaming state: up to 12 pending bits, the column, the prefix flag
  (written lazily, with the first non-empty input).

**Decode driver:**
- **Classification** (spec 0008 §2): skip whitespace; `-` → SIMD;
  otherwise the legacy codec, unchanged.
- **Layout**: decided by the header, never by scanning the payload. If
  the byte after `-` starts a terminator, the stream is wrapped; otherwise
  it is unwrapped, and the driver never looks for a terminator again: a
  stray one is an invalid block, caught by the kernels' validity check
  (§5), not by a search. For a wrapped stream, the terminator (LF or CR LF) and W (the length of the first
  line) are fixed from then on. Line k's content and its terminator are at
  known positions, so the driver checks **one terminator per line**, and
  runs the kernels on each line's blocks **in place**, writing output at
  its final offset (13 bytes per 16 characters). There is no copy and no
  scan for newlines beyond the first line.
- **Validation:** each kernel call reports whether its block was valid
  (§5); the driver ANDs the results and checks them once per line (or per
  call). The payload length and the last line (1 … W characters, then only
  whitespace) are checked at the end. On any failure, a scalar re-scan of
  the failing region produces the precise `Error` (§1).
- **Streaming** (spec 0008 §3.6): state is the classification, the header
  and terminator progress (a first chunk of just `-` leaves the layout
  undecided until the next byte), W once known, the column, and a
  **16-byte carry buffer** for a block split across calls (no heap). Whole blocks inside a
  chunk are decoded in place; only a split block goes through the carry
  buffer. Chunking-invariant by construction: the same bytes and the same
  error (class and offset) for any split.

**Kernel store slack** (spec 0007 §4): the decode kernels store 16 bytes
for 13; every block but the last is followed by the next block's output,
which overwrites the slack. The last block goes through a 16-byte stack
buffer when fewer than 3 bytes of space remain. Size hints stay tight.

### 4 — Performance

**Baseline first.** Before the rewrite, measure on 0.2.3 what the
newline search costs on unwrapped input: one-shot decode with and without
its `contains(&b'\n')` scan (a temporary patch, reverted), same pinned
CPU. The rewrite removes that scan (§3); the measurement says how much of
the unwrapped speedup to expect from it.

*Measured (2026-09-27, 0.2.3, 1 MiB input, one pinned vCPU, two
alternating rounds):*

| Level | With the scan | Without | Speedup |
|---|---|---|---|
| AVX2 | 297 µs | 200 µs | 1.48× |
| SSE4.1 | 327 µs | 233 µs | 1.40× |
| Scalar | 912 µs | 811 µs | 1.12× |

The scan costs a constant ~97 µs per MiB (about 13 GB/s): `core`'s
`memchr`, behind `<[u8]>::contains`, works a word at a time, not with
SIMD. Removing it is worth about a third of the AVX2 decode time on its
own. The unwrapped target of this spec is therefore not just "no
regression" but this speedup.

Measured with `tests/perf_sanity.rs` (`rust-perf`) and `criterion`, against
0.2.3 on the same machine:
- **Unwrapped:** no regression beyond noise (≤ 2 %), encode and decode.
- **Wrapped (W = 64) vs unwrapped:** within 10 %, encode and decode.
  `perf_sanity`'s `MIN_WRAPPED_OVER_UNWRAPPED` is then re-calibrated (70 %
  of the new measurement).
- **Strict validation on valid input:** at most 3 % slower than decoding
  without it (spec 0008 §3.5). Measured by comparing the strict decoder with
  a build that skips the checks.
- **Slice API:** within noise of the `Vec` API.

### 5 — Kernel interface changes

The kernels keep their block-level contract, with two changes to the
validity they return:

- **Correct character validation.** Replace the signed comparison with
  an unsigned range check on the index (e.g. `min_epu8` / `max_epu8`, or a
  saturating subtraction), and reject `0x27` explicitly. Every byte outside
  the alphabet must fail its block, CR, LF and space included: the
  unwrapped fast path relies on it to catch stray terminators (§3).
- **Reject pairs worth more than 8191** (spec 0008 *invalid value*): one
  extra vector comparison per block. Today's kernels do not check it.

Both fold into one validity mask per block. The early return after each
block becomes an accumulated mask, checked by the driver once per line or
per call (§3). Other kernel work (e.g. the per-block call through a
function pointer, which spec 0011 lists as a performance suspect) belongs to
spec 0011.

### 6 — CLI

- `base91 -d`: an invalid SIMD stream exits with status 1 and a message
  naming the error class and offset, e.g.
  `base91: invalid character at offset 1234`.
- `base91 --simd`: encodes with the streaming encoder (wrapping included)
  instead of reading all input into memory first. The encoded output is
  unchanged.
- Help text and man page: invalid widths, error exit status.

### 7 — Tests (spec 0009)

- Delete `Deviation::SimdStrictDecoding` from `pending`. Every comparison it
  gated switches on: the matrix, invalid-input, property, fuzz (including
  error class and offset against the oracle), and CLI tests. `rust-fuzz`
  drops its `-Coverflow-checks=off`.
- Write the `strict_decoder_*` regression tests (spec 0009 §12, one per
  annex item A9–A13b) against the new error API.
- Add the slice API, the new `simd::Encoder` wrapping, and `io` error
  reporting to the matrix, properties and fuzz targets; add buffer-too-small
  cases (state and output untouched).
- Delete the spec 0006 bug-2 regression test from `tests/regressions.rs`
  (`simd::scalar` is no longer public); the unit tests in `simd/scalar.rs`
  (`decoder_chunk_size_1`, `decoder_chunk_independence`) already cover it.
- Kernel unit tests: every byte value outside the alphabet, at every
  position of a block, fails that block, at every level; pairs worth
  8192 … 8280 fail too.
- Raise `perf_sanity`'s wrapped thresholds (§4).
- The no-`alloc` build (spec 0004 §7, `wasm32v1-none`) must include the
  slice API.

### 8 — Documentation and release

- README: the error API, the slice API (a stack-buffer example), migration
  notes from 0.2.x.
- Rustdoc: every deprecated function points to its replacement.
- Spec 0002: its wire-format and API sections marked superseded by specs
  0008 and 0010.
- `CHANGELOG.md` for 0.3.0, `Changed` (breaking):
  - `simd::encode` → `Result`;
  - `simd::decode` → `Result`;
  - `simd::Decoder::decode` / `finish` → `Result`;
  - `simd::Encoder::new(level, wrap)`;
  - `simd::scalar` private;
  - `io::DecoderReader` reports invalid SIMD streams;
  - strict SIMD decoding.

  `Added`: `Error`, the slice API, `simd::encoded_len`, wrapped streaming
  SIMD encoding. `Deprecated`: `*_unchecked`.
- Version 0.3.0 for `base91-rs`, `base91-cli` and `pybase91` (the latter
  unchanged in behavior).

## Stage 1 results (2026-09-28)

Against 0.2.3, 1 MiB, one pinned vCPU, with the strict checks:

| | 0.2.3 | Stage 1 |
|---|---|---|
| decode, unwrapped: scalar / SSE4.1 / AVX2 | 905 / 323 / 294 µs | 853 / 273 / 245 µs |
| decode, W = 64 (AVX2) | 1 686 µs | 274 µs (+12 % over unwrapped) |
| encode, unwrapped: scalar / SSE4.1 / AVX2 | 1 029 / 331 / 324 µs | 809 / 335 / 329 µs |
| encode, W = 64 (AVX2) | 482 µs | 357 µs (+8 % over unwrapped) |
| streaming decode, unwrapped / W = 64 | 302 / 869 µs | 264 / ≈ 340 µs |

- Unwrapped encode is within 2 % of 0.2.3 (SSE4.1 +1.2 %, AVX2 +1.5 %).
- Wrapped decode misses the 10 % target by 2 points; the gap is
  kernel-loop work and moves to spec 0011, with the validation budget
  (decision 7).
- The loop around each kernel is chosen per kernel by measurement (a flat
  loop for AVX2, one out-of-line call per line for SSE4.1): see spec 0011,
  *Code-generation sensitivity*.

Two changes to the plan below:
- **`Error` moved into stage 1.** The decoder core reports errors with
  offsets from stage 1 on; making the type public at once (it is purely
  additive, and §1 fixes its variants) avoids an internal type with fields
  nothing reads yet. The `Result` signatures stay in stage 2.
- **Spec 0008 §3.5 now says which error is reported** when a stream has
  several (the first one, reading left to right, with a defined offset per
  class), and the spec 0009 oracle implements it. Without it, "the
  classes and offsets are the ones the oracle computes" (§1) would have
  made the oracle's check order normative, and that order cannot be
  implemented by a streaming decoder.

## Stages

1. **Core and drivers** behind the current public API, so that the existing
   and pending tests exercise them from the start.
2. **Public API layers**: `Error`, slice API, `Result` signatures,
   deprecations, `io`, CLI.
3. **Tests switched on** (§7) and performance re-calibrated (§4).
4. **Docs, CHANGELOG, release 0.3.0.**

## Open questions

None.

## Verification

```bash
cd rust
cargo test --workspace --features base91-rs/reference-tests,base91-cli/reference-tests,base91-fuzz/reference
cargo test -p base91-rs --lib --no-default-features                  # slice API without alloc
cargo test --release -p base91-rs --test perf_sanity -- --ignored   # §4 criteria
nix-build ../default.nix -A rust-tests -A rust-tests-full -A rust-tests-aarch64 \
  -A rust-nostd -A rust-perf -A rust-clippy -A rust-fmt
nix-build ../default.nix -A rust-fuzz --arg fuzzSeconds 600
```

`pending::PENDING` is empty for SIMD deviations. `reuse lint` must pass.
