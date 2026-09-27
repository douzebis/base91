<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0005 — Fix memory corruption in `simd::encode_unchecked` / `decode_unchecked`

**Status:** implemented
**App:** base91
**Implemented in:** 2026-09-27

## Problem

`simd::encode_unchecked` and `simd::decode_unchecked` (`simd/mod.rs:224`,
`:246`) are `unsafe fn`s whose documented contract is: "`output` must point
to at least `encode_size_hint(input.len(), wrap)` (resp.
`decode_size_hint(input.len())`) writable bytes". Callers who honor that
contract still get undefined behavior, observed as heap corruption and
wrong output.

### Root cause

Both functions wrap the caller's buffer in a `Vec`:

```rust
let mut out = Vec::from_raw_parts(output, 0, encode_size_hint(input.len(), wrap));
…
core::mem::forget(out);
```

1. **Contract violation in itself.** `Vec::from_raw_parts` requires a
   pointer obtained from the global allocator with exactly that capacity.
   A caller's buffer (stack array, sub-slice, `Vec` of a different
   capacity, C memory) never satisfies this.
2. **Reallocation of caller memory.** The internal code calls
   `Vec::reserve` with more than the capacity the size hint provides.
   `Vec` then `realloc`s the caller's pointer: the caller's allocation is
   freed or moved, the output lands in a new buffer that is then leaked,
   and the caller's buffer is left partly unwritten.

| Location | Reserve | Exceeds the hint capacity when |
|---|---|---|
| `simd/x86.rs:679` (`encode_loop`) | `out_needed + OUT` | always, for inputs with ≥ 1 full block (e.g. AVX2, n = 26k: needs 32k+32, has 32k+2) |
| `simd/aarch64.rs:321` (`encode_neon`) | `full_blocks*16 + 16` | same pattern (not tested; no aarch64 machine) |
| `simd/scalar.rs:221` (`ScalarDecoder::decode`) | `input.len()` | `input.len() > ⌊7·len/8⌋ + 1`, i.e. almost always — **on every level, including `Scalar`** |
| `simd/x86.rs:712` (`decode_loop`) | `out_needed + OUT` | full blocks present |
| `simd/aarch64.rs:346` (`decode_neon`) | `full_blocks*13 + 13` | same pattern |

What the caller sees depends on the allocator. When `realloc` can grow in
place, nothing visible happens. When it moves the block, the result is a
use-after-free or a double free.

### Observed (x86_64, AVX2 CPU, glibc, buffers sized exactly by the hint)

| Level | Input length | Result |
|---|---|---|
| `Simd128`, `Simd256` | 26, 27, 52 | encode aborts: `free(): double free detected in tcache 2` |
| all, including `Scalar` | 32, 100 | `decode_unchecked` output wrong (no crash) |
| all | 0 | `encode_unchecked` writes `-` (1 byte); `simd::encode` returns an empty result |

### Exposure

- Only `base91-rs` **0.2.2** (the SIMD module first shipped in 0.2.2; 0.2.0
  and 0.2.1 have no `simd` module). crates.io: 0.2.0–0.2.2 are published,
  none yanked, 201 downloads in total (as of 2026-09-27).
- Only Rust callers of `simd::encode_unchecked` / `simd::decode_unchecked`.
  Nothing inside the repo calls them: not the CLI, not `pybase91`, not
  `io`. `pybase91` 0.2.2 on PyPI is not affected.
- The Henke `encode_unchecked` / `decode_unchecked` at the crate root are
  pointer-based (`codec.rs:267`, `:329`) and not affected.

## Goals

- No undefined behavior in `simd::encode_unchecked` /
  `simd::decode_unchecked` for any input, when the caller honors the
  documented contract.
- Their output is byte-for-byte identical to `simd::encode` /
  `simd::decode` for the same arguments, including empty input.
- Tests that would have caught this.
- A public notice for 0.2.2 users.

## Non-goals

- Zero-copy output. This spec trades speed for obvious correctness; spec
  0010 restores zero-copy with a slice-based core.
- Changing the signatures or the contract of the two functions.
- Miri in CI (needs a nightly toolchain, which the pinned nixpkgs does not
  provide). The test-strategy spec (0009) revisits this.
- Validating `wrap` in the library. Spec 0008 settles the rule (multiple of
  32) and lists the validation as annex item A2.

## Specification

### 1 — Reimplement on top of the `Vec` API, then copy

```rust
pub unsafe fn encode_unchecked(
    input: &[u8],
    max_level: SimdLevel,
    wrap: usize,
    output: *mut u8,
) -> usize {
    let encoded = encode(input, max_level, wrap);
    debug_assert!(encoded.len() <= encode_size_hint(input.len(), wrap));
    // SAFETY: the caller guarantees `output` has at least
    // `encode_size_hint(input.len(), wrap)` writable bytes, and the size
    // hint is an upper bound on `encode`'s output (§2).
    core::ptr::copy_nonoverlapping(encoded.as_ptr(), output, encoded.len());
    encoded.len()
}

pub unsafe fn decode_unchecked(input: &[u8], max_level: SimdLevel, output: *mut u8) -> usize {
    let decoded = decode(input, max_level).unwrap_or_default();
    debug_assert!(decoded.len() <= decode_size_hint(input.len()));
    // SAFETY: as above, with `decode_size_hint`.
    core::ptr::copy_nonoverlapping(decoded.as_ptr(), output, decoded.len());
    decoded.len()
}
```

- `Vec::from_raw_parts` and `mem::forget` go away entirely.
- Empty input: `encode` already returns an empty `Vec`, so
  `encode_unchecked` writes nothing and returns 0. This is a behavior
  change (it used to write `-`), and it is the consistent choice: the SIMD
  decoder treats an empty stream and `-` alike.
- Invalid SIMD input: `decode` "always succeeds" (invalid bytes silently
  corrupt the output). `decode_unchecked` keeps exactly that behavior.
- Performance cost: one allocation and one copy per call. It is accepted
  until spec 0010.

### 2 — Size-hint audit

The hints no longer size a `Vec`'s capacity; they now bound the **output**.
Check (in tests, §3) that for every level, wrap and length in the test
matrix:

- `simd::encode(…).len() <= simd::encode_size_hint(len, wrap)`;
- `simd::decode(enc).len() <= simd::decode_size_hint(enc.len())`.

If a hint turns out too small, fix the hint (never the check).

### 3 — Tests

New integration test file `rust/base91/tests/unchecked.rs`:

- **Matrix:**
  - levels: `Scalar`, `Simd128`, `Simd256`;
  - input lengths: `0..=1100`, which covers every block boundary (13, 16,
    26, 32) several times over;
  - `wrap`: `{0, 32, 64}`.
- **Encode:**
  - allocate `hint + 64` bytes, filled with `0xA5`;
  - call `encode_unchecked` on the first `hint` bytes;
  - assert the output equals `simd::encode`;
  - assert the 64 canary bytes are untouched;
  - assert the §2 bound.
- **Decode:** same pattern, taking as input:
  - each encoding from the encode step (wrapped and unwrapped);
  - a Henke encoding of the same data (fallback path).
- **Crate root:** the same matrix for the Henke `encode_unchecked` /
  `decode_unchecked`, to lock in that they stay correct.
- **Regression:** named tests for the exact failing cases in *Observed*
  (lengths 26, 27, 52 encode; 32, 100 decode; 0).

These run in the existing `rust-tests` Nix attribute. They also run under
`--no-default-features --features alloc` (spec 0004 §7), which exercises
the compile-time detection path.

### 4 — Documentation

- Rustdoc of both functions: say that the output is identical to
  `simd::encode` / `simd::decode`, that empty input writes nothing, and
  that they currently allocate (to be removed by spec 0010, which also deprecates them).
- `CHANGELOG.md`, new `Security` section under 0.2.3: describe the bug, the
  affected version (0.2.2), the affected functions, and the recommended
  action (upgrade to 0.2.3).

### 5 — Release and disclosure

- Ship in `base91-rs` 0.2.3 together with spec 0004.
- Yank `base91-rs` 0.2.2 on crates.io once 0.2.3 is published.
- Do not yank `pybase91` 0.2.2 on PyPI (not affected).
- Publish a GitHub security advisory on `douzebis/base91`, and file a
  RustSec advisory. Whether it is filed as a memory-corruption advisory or
  as `informational = "unsound"` is decided with the RustSec maintainers.
  The relevant fact is that the bug is reachable while honoring the
  documented contract.

## Open questions

1. **Disclosure scope.** Yanking 0.2.2 and filing RustSec is standard
   practice for a soundness bug in a published crate, but with 201 total
   downloads you may prefer only a GitHub advisory plus the CHANGELOG
   entry. Your call.

   *Decided (2026-09-27):* no disclosure of any kind until every pending
   spec is implemented and tested. The scope (yank, GitHub advisory,
   RustSec) is decided then. Until that point nothing that describes the
   bug is pushed to the public repository.

## Verification

```bash
cd rust/base91
cargo test --test unchecked                                           # pass
cargo test --test unchecked --no-default-features --features alloc    # pass
cargo test                                                            # pass
nix-build ../../default.nix -A rust-tests                             # pass
```

Before the fix, the regression tests must fail (or abort). Check this
once by running them against the 0.2.2 code.

`reuse lint` must pass.
