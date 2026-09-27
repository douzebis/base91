<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0011 — SIMD kernel performance investigation (embryo)

**Status:** embryo (placeholder to track an open question; not ready for
implementation)
**App:** base91
**Implemented in:** —

## Why this exists

Measurements made while implementing spec 0009 show that the SIMD kernels
fall short of their own targets, and that AVX2 barely improves on SSE4.1.
This is a separate question from the driver-layer rewrite (spec 0010), so
it is kept apart. This embryo records the facts so far and what to
investigate; it becomes a real spec once someone takes it up.

## Facts so far

Dev machine, 2026-09-27, x86_64 with AVX2, 4 MiB of random data, unwrapped
SIMD streams (`tests/perf_sanity.rs`; median of 7 runs, three runs):

| Level | encode (MB/s) | decode (MB/s) |
|---|---|---|
| Scalar | ~1 020 | ~1 110 |
| Simd128 (SSE4.1) | ~2 390 | ~3 100 |
| Simd256 (AVX2) | 2 090–2 520 | ~3 370 |

- **AVX2 vs SSE4.1:** ×0.87–1.06 on encode (×1.45 once, in the Nix
  sandbox), ×1.07–1.10 on decode. Twice the vector width buys almost
  nothing.
- **Targets of spec 0002** (single thread, AVX2): encode ≥ 5 GB/s, decode
  ≥ 3.5 GB/s, with 4–7 GB/s estimated. Measured: about 2.4 GB/s encode
  (half the target), about 3.4 GB/s decode (just under).
- The kernels themselves are correct: the spec 0009 matrix checks them
  against the oracle on x86_64 (SSE4.1, AVX2) and on aarch64 (NEON, under
  qemu).
- Not measured yet: aarch64 NEON on real hardware (qemu timings are
  meaningless).

## Questions to investigate

1. **Where the time goes.** Profile the kernels (e.g. `samply`, already in
   the dev shell; `perf`; `llvm-mca` on the kernel loops) rather than
   guess.
2. **Candidate causes, to confirm or rule out:**
   - the per-block call through a function pointer in `x86::encode_loop` /
     `decode_loop` (`block_fn: unsafe fn(…)`). Functions with
     `#[target_feature]` may not be inlined into callers without it, so
     each block may pay a real call;
   - the padding copy in `encode_loop` for the last blocks;
   - cross-lane shuffles or port pressure specific to the AVX2 kernels;
   - memory bandwidth, e.g. whether 4 MiB fits the machine's caches.
3. **Whether the targets are realistic.** Are spec 0002's 5 / 3.5 GB/s
   reachable with this format, or should they be revised, with evidence?
4. **Is AVX2 worth keeping** if it cannot beat SSE4.1 meaningfully? (It
   adds code, tests and a dispatch path.)

## Dependencies and timing

- After spec 0010: the driver rewrite changes the loops around the
  kernels, so profiling before it would measure code about to disappear.
- Uses the spec 0009 tooling: `rust-perf` for relative numbers (its
  `Simd256 / Simd128` thresholds, currently 0.61 / 0.76, are to be raised
  when this is resolved), `criterion` for absolute ones.

## Non-goals (for now)

- AVX-512 or SVE2 kernels (spec 0008 §3.4 defers them).
- Anything in the driver layer (spec 0010).
