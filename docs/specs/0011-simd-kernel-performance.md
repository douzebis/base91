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
   - ~~the per-block call through a function pointer in
     `x86::encode_loop` / `decode_loop`~~: gone with the spec 0010 driver,
     which calls batch functions (`encode_lines_*`, `decode_lines_*`)
     whose block kernels are inlined;
   - the padding copy for the last block of a batch (now in
     `driver::encode_blocks`);
   - cross-lane shuffles or port pressure specific to the AVX2 kernels;
   - memory bandwidth, e.g. whether 4 MiB fits the machine's caches.
3. **Whether the targets are realistic.** Are spec 0002's 5 / 3.5 GB/s
   reachable with this format, or should they be revised, with evidence?
4. **Is AVX2 worth keeping** if it cannot beat SSE4.1 meaningfully? (It
   adds code, tests and a dispatch path.)

## Code-generation sensitivity (found in spec 0010, stage 1, 2026-09-28)

**Fact.** How fast a kernel loop runs depends heavily on the shape of the
Rust code around it, because that shape steers the compiler's register
allocation and inlining. Measured while writing the spec 0010 driver
(1 MiB, one pinned vCPU, rustc from the pinned nixpkgs):

| Loop around the block kernel | SSE4.1 decode | AVX2 decode |
|---|---|---|
| one flat loop over the batch, skipping terminators | 575 µs | 245 µs |
| a plain loop per line, out of line (`#[inline(never)]`) | 273 µs | 253 µs |
| nested loops (lines × blocks), inlined | 267 µs | 305 µs |

The same source construct is the fastest for one kernel and twice as slow
for the other; the ranking of the others differs too. `perf` shows the cause on the slow AVX2 variant: the
kernel's dozen constants stay live across iterations, and the compiler
rotates them through registers on every block (a chain of
`vmovdqa ymmA, ymmB` among the hottest instructions), plus a reload.

The scalar decoder showed the same class of problem twice: a block
function that was not inlined returned its bytes through memory (40 % of
the time), and once inlined, its per-character validation loop over an
array was auto-vectorized into SSE2 of about 1 000 instructions per
block. Handling one pair at a time, straight into the output, fixed both
(853 µs, against 905 µs in 0.2.3).

**What the driver does today** (per kernel, chosen by measurement, and
commented in the code): a flat loop for AVX2, one out-of-line call per
line for SSE4.1, pair-at-a-time for the scalar decoder. This is tuned to
today's compiler; another compiler version may reorder the ranking.

**The risk is speed only.** The output and the errors never depend on it;
the tests check them at every level.

**Exposure.** Our builds pin nixpkgs, so the compiler changes only with a
deliberate bump. Users building from crates.io use their own compiler.
This sensitivity is common to all SIMD code written with intrinsics, in
Rust or C.

**Register usage cannot be forced with intrinsics.** Intrinsics choose
instructions; register allocation, scheduling and inlining stay the
compiler's. Options, from least to most control:

1. **Shape the kernels so the allocator has no reason to misbehave**
   (influences, does not guarantee): fewer live constants (derive some
   from others, read rarely used ones from memory, which costs a load
   that is almost free), constants loaded once per batch and passed
   explicitly, a loop unrolled by two blocks, the same loop shape for
   every kernel. Usually stable across compiler versions, but not
   promised.
2. **Hand-written assembly** for the hot loops (`core::arch::asm!`,
   stable Rust): the only way to force registers and instructions, and
   immune to compiler versions. Used for this reason by ring and dav1d.
   Costs: three loops maintained by hand (SSE4.1, AVX2, NEON; about 60 to
   100 instructions each, the driver staying in Rust), harder review,
   fully `unsafe`, no optimization across the boundary. The existing
   tests (oracle matrix, kernel validation tests, fuzzing, NEON under
   qemu) would cover it as they cover the intrinsics.
3. **Detect instead of prevent:** `tests/perf_sanity.rs` ratios in CI
   (`rust-perf`), machine-independent, that fail on this class of
   regression: wrapped against unwrapped decode, AVX2 against SSE4.1
   decode (AVX2 must not be slower), SIMD against scalar decode.

**Recommendation.** Option 3 in spec 0010 (stage 3 re-calibrates
`perf_sanity` anyway), option 1 here, and option 2 kept in reserve: only
if the CI guard trips on a nixpkgs bump despite option 1. Assembly buys
certainty at a real maintenance cost; so far the sensitivity has been
seen on loops that option 1 restructures anyway. Once option 1 is done,
the per-kernel special cases above should go.

**Targets left over from spec 0010** (both kernel-loop work):
- Wrapped decoding within 10 % of unwrapped: stage 1 measures +12 %
  (274 µs against 245 µs, AVX2, W = 64).
- Strict validation costing at most 3 % of decode throughput (spec 0008
  §3.5): stage 1 measures +25 % (SSE4.1) and +29 % (AVX2), every part of
  the check costing several percent (spec 0010, decision 7). Candidate
  formulations, to evaluate with the kernels reshaped (option 1 above):
  running maxima instead of subtract-and-OR (one operation per block for
  the range check and for the 8191 check, tested once per batch); the 8191
  check on the bytes, before the multiply (a pair exceeds 8191 exactly
  when its high index is 90 and its low index is at least 2); a
  lookup-table classification of the alphabet (two byte shuffles), as
  SIMD base64 decoders do.

## Dependencies and timing

- After spec 0010: the driver rewrite changes the loops around the
  kernels, so profiling before it would measure code about to disappear.
- Uses the spec 0009 tooling: `rust-perf` for relative numbers (its
  `Simd256 / Simd128` thresholds, currently 0.61 / 0.76, are to be raised
  when this is resolved), `criterion` for absolute ones.

## Non-goals (for now)

- AVX-512 or SVE2 kernels (spec 0008 §3.4 defers them).
- Anything in the driver layer (spec 0010).
