<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0007 — Allocation-free encode/decode API

**Status:** draft
**App:** base91
**Implemented in:** —

## Problem

After spec 0004, `base91-rs` builds without `alloc`, but it offers almost
nothing there: only size hints, `simd::detect()` and the Henke
`encode_unchecked` / `decode_unchecked` raw-pointer functions (spec 0004
§2). Everything else (the SIMD variant, streaming encoders/decoders) takes
or returns a `Vec`.

People who use `no_std` usually need no-alloc as well:
- microcontrollers without a heap;
- bootloaders and firmware;
- early boot and interrupt context;
- real-time or safety code where allocation is banned.

`std` users also lack a safe way to encode into a buffer they already own
(stack array, network packet, mmap). The only option today is the `unsafe`
`*_unchecked(…, *mut u8)` functions, and the SIMD ones were unsound
(spec 0005).

## Goals

- A safe, slice-based API for every operation (Henke and SIMD, one-shot
  and streaming), available **without `alloc`**.
- Zero-copy: the slice API writes directly into the caller's buffer.
- The existing `Vec` API is unchanged in signature and behavior, and
  becomes a thin layer over the slice core.
- `simd::encode_unchecked` / `decode_unchecked` become zero-copy again
  (removing spec 0005's temporary `Vec`) and available without `alloc`.
- No throughput regression (§7).

## Non-goals

- `io` adapters without `std` (`core` has no `Read` / `Write`;
  `embedded-io` would be a new dependency).
- Changes to the Python bindings or the CLI.
- Fixing the SIMD streaming decoder bug (spec 0006). The slice-based
  streaming decoder specified here is built on spec 0006's design and must
  stay correct for arbitrary chunking.

## Dependencies

- Spec 0004 (feature layout `std` ⊃ `alloc` ⊃ core).
- Spec 0005 (tests for the unchecked functions, reused here).
- Spec 0006 (SIMD streaming decoder fix). It must land first, so §6's
  equivalence tests have a correct `Vec` streaming decoder to compare
  against. §5 below also reuses its segment-wise decoding.

## Specification

### 1 — Error type

```rust
/// Error returned by the slice-based API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// `output` is shorter than the size hint for this call.
    BufferTooSmall { needed: usize },
}
```

It implements `core::fmt::Display` and `core::error::Error` (stable since
1.81; MSRV is 1.91). The type is `#[non_exhaustive]`, so a variant for
invalid SIMD input can be added later without a breaking change.

### 2 — Public slice API (no feature required)

Every function checks `output.len()` against the matching size hint
**before writing anything**. On `Err`, `output` and the encoder/decoder
state are untouched. The check is conservative: a call can fail even if
the actual output would have fit, since the hint is an upper bound. The
rustdoc says so.

Crate root (Henke):

| Function | Size required |
|---|---|
| `encode_to_slice(input, output) -> Result<usize, Error>` | `encode_size_hint(input.len())` |
| `decode_to_slice(input, output) -> Result<usize, Error>` | `decode_size_hint(input.len())` |
| `Encoder::encode_to_slice(&mut self, input, output)` | `encode_size_hint(input.len())` |
| `Encoder::finish_to_slice(&mut self, output)` | `ENCODE_FINISH_MAX` (= 2) |
| `Decoder::decode_to_slice(&mut self, input, output)` | `decode_size_hint(input.len())` |
| `Decoder::finish_to_slice(&mut self, output)` | `DECODE_FINISH_MAX` (= 1) |

`simd` module:

| Function | Size required |
|---|---|
| `simd::encode_to_slice(input, max_level, wrap, output)` | `simd::encode_size_hint(input.len(), wrap)` |
| `simd::decode_to_slice(input, max_level, output)` | `simd::decode_size_hint(input.len())` |
| `simd::Encoder::encode_to_slice` / `finish_to_slice` | per-call hint / finish maximum |
| `simd::Decoder::decode_to_slice` / `finish_to_slice` | per-call hint / finish maximum |

The streaming `finish_to_slice` takes `&mut self`, unlike the `Vec`
`finish(self)`. That way a `BufferTooSmall` error does not consume the
state. On success it resets the encoder/decoder to its initial state.

The finish maximums are exported as `pub const`s. Their exact values for
the SIMD streaming types are set during implementation from the scalar
tail logic; they are not guessed here.

Return values and semantics (empty input, auto-detection of `-` in SIMD
decode, silent corruption on invalid SIMD characters) are identical to the
corresponding `Vec` functions.

### 3 — Internal core: write into `&mut [MaybeUninit<u8>]`

Every encoder/decoder has one core implementation:

```rust
fn …_core(state, input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize
// Precondition: out.len() >= the matching size hint.
// Postcondition: out[..n] is initialized; returns n.
```

- **`Vec` layer** (`#[cfg(feature = "alloc")]`): `reserve(hint)`, run the
  core on `spare_capacity_mut()`, then `set_len(len + n)`. That is exactly
  what `codec::Encoder::encode` does today. No `reserve` stays inside the
  cores or the kernels.
- **Slice layer**: check the hint (§2), then run the core on `output`,
  viewed as `&mut [MaybeUninit<u8>]`. This conversion goes through one
  private helper with a `// SAFETY:` comment: the cores only write
  initialized bytes. `&mut [u8]` → `&mut [MaybeUninit<u8>]` must never be
  exposed publicly.
- **Unchecked layer**: `*_unchecked(input, …, output: *mut u8)` builds the
  slice with `core::slice::from_raw_parts_mut(output.cast(), hint)`
  (sound under the documented contract) and runs the core. This replaces
  spec 0005's temporary `Vec` + copy.

Code converted: `codec::Encoder` / `Decoder`, `codec::encode_unchecked` /
`decode_unchecked`, `simd::scalar::ScalarEncoder` / `ScalarDecoder`,
`simd::encode_into` / `decode_into`, `x86::encode_loop` / `decode_loop`,
`aarch64::encode_neon` / `decode_neon`, `simd::Encoder` / `Decoder`.

### 4 — Kernel store slack

The decode block kernels produce 13 bytes but store 16 (`x86.rs:380`,
`:615`, `aarch64.rs:308`). Today this is covered by the `+ OUT` in
`reserve` (spec 0005). The slice core must not rely on space beyond the
hint:

- Run all but the last full block directly into `out`.
- If fewer than 3 bytes of `out` remain after the last block's 13, run
  that block into a 16-byte stack buffer and copy 13 bytes.
- Size hints stay unchanged (tight).

The encode kernels store exactly what they produce (16 or 32 bytes per
block; `x86.rs:187`, `:486`, `aarch64.rs:172`), so they need no bounce
buffer. Input over-read is already handled (`pad` in `encode_loop`).

### 5 — No-alloc replacements for the two internal allocations

- **`\n` stripping in `simd::decode_into`** (`simd/mod.rs:312`, collects a
  cleaned copy): already removed by spec 0006 §3, which decodes
  `\n`-separated segments through the streaming core. Nothing to do here
  beyond moving that core onto slices.
- **`insert_wrap_newlines`** (`simd/mod.rs:369`, uses `Vec::resize`): the
  final length is known up front (`simd::encode_size_hint` already counts
  the newlines), so it becomes an in-place right-to-left shift on the
  output slice, with the same algorithm as today.

### 6 — Tests

- **Equivalence:** for the spec 0005 matrix (levels × lengths `0..=1100`
  × `wrap` ∈ {0, 32, 64}), every `_to_slice` function gives the same bytes
  as its `Vec` counterpart. Canary bytes after `hint` are untouched.
- **Streaming:** every streaming type, fed in chunks of 1, 7, 13, 16, 17,
  4096 and random sizes, gives the same output as the one-shot call.
- **Too small:** with `output.len() == hint - 1`, the result is
  `Err(BufferTooSmall { needed: hint })`, `output` is unchanged (canary
  fill), and the streaming state is unchanged (a retry with a big enough
  buffer gives correct output).
- **Kernel slack:** buffers sized exactly by the hint, for every decode
  length that ends on a full block. These are the §4 cases.
- **No-alloc build:** the tests above that do not use `Vec` also run under
  `cargo test --lib --no-default-features`. The spec 0004 §7
  `wasm32v1-none` build (no `alloc` feature) checks that the slice API
  compiles without `alloc`.

### 7 — Performance

The `criterion` throughput bench (`benches/throughput.rs`) gets `_to_slice`
cases. Acceptance, versus the pre-change baseline on the same machine:

- `Vec` API and `*_unchecked`: no regression beyond noise (≤ 2 %);
- `_to_slice`: within noise of `*_unchecked`.

### 8 — Documentation and release

- README: a "Without allocation" section with a stack-buffer example.
- Rustdoc: every `*_unchecked` function points to its safe `_to_slice`
  counterpart.
- Spec 0004 §2 table: extended with the slice API.
- `CHANGELOG.md`, `Added`: the slice API and `Error`.
- Version: `0.3.0` (new public API; no breaking change).

## Open questions

1. **Deprecate the `*_unchecked` functions?** With a safe zero-copy
   `_to_slice`, their only remaining advantage is skipping one length
   comparison. Deprecating them (`#[deprecated]`, removal in 1.0) shrinks
   the unsafe surface. On the other hand, the benches and examples use
   them, and they mirror the C API style.
2. **Naming.** `encode_to_slice` / `decode_to_slice` follow the `base64`
   crate's `encode_slice` / `decode_slice` loosely. Keep `_to_slice`, or
   align exactly with `base64` (`encode_slice`)?

## Verification

```bash
cd rust/base91
cargo test                                                # pass
cargo test --lib --no-default-features                    # pass (slice API only)
cargo test --no-default-features --features alloc         # pass
cargo build --lib --no-default-features --target wasm32v1-none
cargo bench --bench throughput                            # §7 criteria
nix-build ../../default.nix -A rust-tests -A rust-nostd   # pass
```

`reuse lint` must pass.
