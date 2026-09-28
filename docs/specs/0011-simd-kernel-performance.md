<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0011 — SIMD kernel performance: validation cost, wrapping overhead, robust code generation

**Status:** draft
**App:** base91
**Implemented in:** —

Follows spec 0010 (driver rewrite, 0.3.0), which left two performance
targets unmet and found the kernels' speed fragile under the compiler.
Kernel-internal work only: no API or format change.

## Problem

Measured on a performance core with the 0.3.0 driver (see *Measurement
protocol*, §1), strict checks included:

1. **Validation is expensive.** Spec 0008 §3.5 says error detection SHOULD
   cost at most 3 % of decode throughput. It costs +22 % (SSE4.1), +16.5 %
   (AVX2) and +29.5 % (AVX2, wrapped at 64 columns), measured against a
   build with the checks removed (spec 0010, decision 7). Every part of the
   check (the unsigned range test, the `'` test, the 8191 test) costs
   several percent, out of proportion to the handful of vector operations
   involved.
2. **Wrapping is not free.** Spec 0010 §4 targets wrapped throughput within
   10 % of unwrapped. On a performance core, 1 MiB, AVX2: decode +14 %
   (168–170 µs against 147–149 µs), encode +16 % (138–140 µs against
   119–120 µs).
3. **The kernels' speed depends on the code around them.** The loop that
   drives a block kernel changes how the compiler allocates registers: the
   same construct was the fastest for one kernel and twice as slow for the
   other (see *Background*). The driver works around it with per-kernel
   special cases chosen by measurement (a flat loop for AVX2, an
   out-of-line call per line for SSE4.1), tuned to today's compiler.
   `perf_sanity` (spec 0010, stage 3) detects large regressions of this
   kind, but nothing prevents them.

## Goals

Every goal applies on both core types: performance and efficiency cores
(§1).

- **G1 — Validation cost:** at most 3 % of decode throughput (spec 0008
  §3.5), SSE4.1 and AVX2, unwrapped and wrapped. If
  the best formulation cannot reach it, the lowest cost reached, with the
  evidence (open question 1).
- **G2 — Wrapping overhead:** wrapped (W = 64) within 10 % of unwrapped,
  encode and decode, SSE4.1 and AVX2.
- **G3 — Robust code generation:** one loop structure for the three SIMD
  kernels (SSE4.1, AVX2, NEON), no per-kernel special case, and each
  code path's speed insensitive to the loop around it: the three loop
  shapes of *Background* within 5 % of each other, for SSE4.1, AVX2 and
  the scalar path (NEON: not measurable, see *Non-goals*).
- **G4 — No regression:** every figure of spec 0010's performance-core
  table (*Stage 1 results*) at least as fast, within noise (2 %).

## Non-goals

- AVX-512 or SVE2 kernels (spec 0008 §3.4 defers them).
- **NEON speed.** No aarch64 hardware is available, and qemu timings are
  meaningless. NEON receives the same changes as the x86 kernels, for
  symmetry (decision 2); its correctness stays tested under qemu.
- The driver's API and the wire format (specs 0008, 0010).
- Hand-written assembly: a decision gate at the end of this spec (§6)
  says whether a follow-up spec is needed.

## Background

**Resolved questions of the embryo** (2026-09-28). The embryo asked whether
AVX2 is worth keeping and whether spec 0002's targets (encode ≥ 5 GB/s,
decode ≥ 3.5 GB/s) are reachable. Its evidence came from unpinned runs on a
hybrid laptop chip in a VM, mostly on efficiency cores, which run 256-bit
AVX2 as two 128-bit halves. Re-measured on a performance core (0.3.0,
`perf_sanity`, 512 KiB, fastest of 51 runs, three runs):

| Level | encode (MB/s) | decode (MB/s) |
|---|---|---|
| Scalar | 3045–3106 | 2292–2328 |
| Simd128 (SSE4.1) | 5123–5232 | 5795–5904 |
| Simd256 (AVX2) | 9293–9880 | 7747–7763 |

AVX2 is ×1.81–1.89 faster than SSE4.1 on encode, ×1.31–1.34 on decode,
and the targets are met at this cache-resident size. Both questions are
closed. (The 0.2.2 measurement that raised them: AVX2 ×0.87–1.06 of SSE4.1,
encode about 2.4 GB/s; unpinned.)

**Code paths.** Three SIMD kernels, each an encode and a decode block
function written with intrinsics: SSE4.1 and AVX2 (x86-64), NEON
(aarch64). A fourth path, the scalar level, is plain Rust, for CPUs with
none of them; it is not a SIMD kernel, but the compiler's code generation
affects it too.

**Loop shapes.** A block kernel processes one block (16 characters ↔ 13
bytes; 32 ↔ 26 for AVX2). The driver hands it batches of several lines of
several blocks, and some loop calls it on each block, skipping the
terminator at each line end. A *loop shape* is how that loop is written;
the three measured compute exactly the same thing:

```rust
// (a) flat: one loop over every block of the batch
for _ in 0..lines * blocks_per_line {
    bad |= kernel(src, dst);
    src += 16; dst += 13;
    if end_of_line { src += terminator_len; }
}

// (b) out of line: one call per line to a function kept out of line
//     (#[inline(never)]), itself a plain loop over the line's blocks
for l in 0..lines { ok &= decode_line(src_of(l), dst_of(l), blocks_per_line); }

// (c) nested: the loop of (b), inlined
for l in 0..lines { for i in 0..blocks_per_line { bad |= kernel(block(l, i), out(l, i)); } }
```

**Code-generation sensitivity** (found in spec 0010, stage 1, on an
**efficiency core**; to be re-measured on a performance core, §2):

| Loop around the block kernel | SSE4.1 decode | AVX2 decode |
|---|---|---|
| one flat loop over the batch, skipping terminators | 575 µs | 245 µs |
| a plain loop per line, out of line (`#[inline(never)]`) | 273 µs | 253 µs |
| nested loops (lines × blocks), inlined | 267 µs | 305 µs |

`perf` shows the cause on the slow AVX2 variant: the kernel's dozen
constants stay live across iterations, and the compiler rotates them
through registers on every block (a chain of `vmovdqa ymmA, ymmB` among
the hottest instructions), plus a reload. The scalar decoder showed the
same class of problem: a block function returning its bytes through memory
(40 % of the time), then a validation loop auto-vectorized into about
1 000 SSE2 instructions per block; handling one pair at a time fixed both.

Intrinsics choose instructions; register allocation, scheduling and
inlining stay the compiler's. Three ways to deal with it, from least to
most control: (1) shape the kernels so the allocator has no reason to
misbehave; (2) hand-written assembly (`core::arch::asm!`), immune to the
compiler but maintained by hand; (3) detect regressions (`perf_sanity`,
done in spec 0010). This spec does (1), with a gate for (2) (§6).

## Specification

### 1 — Measurement protocol

Every number in this spec, and every decision, comes from measurements
made this way:

- On **both core types** of the dev machine (an Intel Core Ultra 7 165U):
  - a **performance core**: `taskset -c 8` (host CPU 0), with vCPU 9, its
    hyperthread sibling, idle;
  - an **efficiency core**: `taskset -c 4` (host CPU 8), with vCPUs 5–7,
    the rest of its four-core cluster (they share an L2 cache), idle.
  Every number says which core it comes from.
- **Nothing else runs**: every binary is built before measuring, and no
  build, test or Nix job runs until the measurement ends.
- **A/B comparisons alternate** over three rounds; results are reported
  as lowest–highest, not as a single figure.
- **While iterating, rounds of seconds, not minutes:**
  `examples/quick_bench.rs` (the `perf_sanity` method: 512 KiB, the
  fastest of 101 runs, no allocation in the timed region, only the paths
  under study; a full table in under a second), five alternating rounds.
  `criterion` (`benches/throughput.rs`, 1 MiB) only to confirm a result;
  `perf_sanity` for ratios; `perf record` / `perf annotate` for where the
  time goes; `llvm-mca` on the kernel loops for port pressure, when the
  profile alone does not explain a difference.
- A variant under study (a validation formulation, a loop shape) is a
  temporary patch in a separate worktree, built before measuring, then
  discarded; only the chosen variant is committed.

### 2 — Baseline

Before any change, on a performance core:

- the loop-shape table of *Background*, for SSE4.1 and AVX2 decode and
  encode, the three shapes each;
- the validation cost split into its parts (range check, `'` test, 8191
  test), unwrapped and wrapped;
- the wrapping overhead, with a profile of where the extra time goes
  (terminator checks, per-line loop, kernel restarts).

Recorded here, under *Results*.

### 3 — Kernel shape

The kernels are restructured so that the loop around them stops
mattering (G3):

- **Constants loaded once per batch.** Each kernel's constants (shuffle
  masks, multipliers, bounds) are built into a struct at the start of a
  batch function and passed by reference to the block function, instead
  of being materialized inside it. Constants used once per block and
  cheap to reload from memory may stay memory operands, to relieve
  register pressure.
- **Unrolled by two blocks**, with independent dependency chains.
- **The same loop structure** for SSE4.1, AVX2 and NEON; the driver's
  per-kernel special cases (`decode_line_sse41` out of line, the flat
  AVX2 loop) are replaced by that single structure.

G3 is checked by re-measuring the three loop shapes of §2 with the new
kernels: within 5 % of each other, for every kernel.

### 4 — Validation formulations

With the kernels reshaped (§3), candidate formulations are measured
against the current one and the unchecked build:

- **Running maxima:** `max_epu8` of the indices and `max_epu16` of the
  pair values accumulated per batch, compared once per batch against 90
  and 8191 (one operation per block for each check, instead of
  subtract-and-OR).
- **The 8191 test on bytes**, before the multiply: a pair exceeds 8191
  exactly when its high index is 90 and its low index is at least 2. It
  removes the dependency on the multiply.
- **Lookup-table classification** of the alphabet (two byte shuffles
  and an AND), as SIMD base64 decoders do, covering the range test and
  `'` at once.
- Combinations of the above.

The cheapest formulation that keeps every kernel validation test passing
(spec 0010 §7) is kept, per kernel if they differ. G1 is checked with the
protocol of §1; if no formulation reaches 3 %, open question 1 decides.

### 5 — Wrapping overhead

Driven by the §2 profile. Candidates, to measure:

- terminator checks for a whole batch of lines ahead of the kernel call
  (they are already one comparison per line; the question is the branch
  and the loop around them);
- one kernel call per batch of lines with the line stride inside the
  reshaped loop of §3 (no restart per line);
- on the encode side, writing the LF of each line inside the kernel loop
  rather than in a second loop over the lines.

G2 is checked with the protocol of §1.

### 6 — Assembly gate

At the end of this spec, if G1 or G3 is not met, or if a nixpkgs bump
trips `perf_sanity` on this kind of regression, a follow-up spec proposes
hand-written assembly for the hot loops (Background, option 2), with its
maintenance cost. Otherwise, assembly is not pursued.

### 7 — Tests

- The kernel validation tests (every byte outside the alphabet at every
  position; pairs 8192–8280), the driver-against-oracle tests, the matrix,
  fuzzing and NEON under qemu must pass unchanged: this spec changes no
  behavior.
- `perf_sanity` re-calibrated at the end (spec 0009, decision 6), its
  thresholds raised where the new ratios allow.

### 8 — Release

`base91-rs` 0.3.1: internal changes only. CHANGELOG `Performance` entry
with the measured gains. Not published before the disclosure hold is
lifted (see the 0.2.3 and 0.3.0 releases).

## Stages

1. **Baseline** (§2), on both core types, recorded under *Results*.
2. **Wrapping** (§5): the targeted fixes the baseline points to (the
   per-line LF loop of the encoder, the byte-by-byte re-read of the first
   batch), with the G2 check.
3. **Validation** (§4), with the G1 check. The baseline shows it is also
   most of the decode wrapping overhead.
4. **Kernel shape** (§3), with the G3 check.
5. **Gate and release** (§6, §7, §8).

(Reordered after the baseline, decision 3: the biggest and simplest levers
first.)

One commit per stage, each with every check passing.

## Decisions recorded (2026-09-28)

1. **G3's criterion is accepted:** the three loop shapes within 5 % of
   each other, for every measurable code path, the scalar one included.
2. **NEON is reshaped** like the x86 kernels, so the three SIMD kernels
   keep the same structure, although its speed cannot be measured here.
3. **Efficiency cores are in scope**, measured on one core of a cluster
   with the other three idle (§1); the stages are reordered after the
   baseline: wrapping fixes and validation before the reshaping.

## Open questions

1. **If 3 % is out of reach with intrinsics:** accept the lowest cost
   reached and relax spec 0008 §3.5's SHOULD to it, with the evidence; or
   make it a trigger of the assembly gate (§6)? *To be discussed with the
   stage 3 results: it depends on the size of the gap.*

## Results

### Stage 1 — Baseline (2026-09-28)

#### Performance core

1 MiB, 0.3.0 (`36d0202`), three alternating rounds
(lowest–highest), protocol of §1.

**Loop shapes** (§2). Each shape applied to every kernel and to the scalar
path, in a separate tree; all trees pass the library and matrix tests.

| Bench | (a) flat | (b) out of line | (c) nested | spread |
|---|---|---|---|---|
| encode, scalar | 492–500 µs | 386–394 µs | 376–381 µs | 31 % |
| encode, SSE4.1 | 218–220 µs | 213–214 µs | 212–213 µs | 3.1 % |
| encode, AVX2 | 123–124 µs | 119–120 µs | 120 µs | 3.0 % |
| decode, scalar | 555–558 µs | 495–496 µs | 496–497 µs | 12.5 % |
| decode, SSE4.1 | 210–211 µs | 200–201 µs | 199–201 µs | 5.3 % |
| decode, AVX2 | 149–150 µs | 146–147 µs | 154 µs | 5.0 % |
| encode W = 64, scalar | 509–519 µs | 466–467 µs | 455–456 µs | 13.1 % |
| encode W = 64, SSE4.1 | 236–239 µs | 246–247 µs | 242 µs | 3.8 % |
| encode W = 64, AVX2 | 142–143 µs | 147–148 µs | 137–138 µs | 7.1 % |
| decode W = 64, scalar | 570–573 µs | 540–541 µs | 538–540 µs | 6.0 % |
| decode W = 64, SSE4.1 | 225–227 µs | 227 µs | 221 µs | 2.6 % |
| decode W = 64, AVX2 | 165–167 µs | 177–178 µs | 175 µs | 6.9 % |

On a performance core, the SIMD kernels' sensitivity is 2.6–7.1 %, not the
factor of two seen on the efficiency core; the scalar path is the most
sensitive (up to 31 %, the flat loop being the worst). No shape is the
fastest everywhere.

**Validation cost** (§2), decode, in trees with parts of the check removed.
On the scalar path the range and `'` checks are one table lookup, so
"range only" equals "without 8191" there.

| Decode | full checks | without 8191 | range only | no checks | cost |
|---|---|---|---|---|---|
| scalar | 497–505 µs | 478–482 µs | 478–480 µs | 367–372 µs | +35.5 % |
| SSE4.1 | 203 µs | 186–187 µs | 176–178 µs | 166 µs | +22.5 % |
| AVX2 | 150–151 µs | 141 µs | 134 µs | 130–131 µs | +15.6 % |
| scalar, W = 64 | 539–540 µs | 540–542 µs | 540–550 µs | 373–375 µs | +44.2 % |
| SSE4.1, W = 64 | 228 µs | 213 µs | 205–206 µs | 168–169 µs | +35.5 % |
| AVX2, W = 64 | 165–167 µs | 157–158 µs | 150–151 µs | 132 µs | +25.9 % |

Per part, unwrapped: AVX2 range +2.7 %, `'` +5.3 %, 8191 +6.9 %; SSE4.1
+6.7 %, +5.4 %, +9 %; scalar table check +30 %, 8191 +4.5 %.

**Wrapping** (§2). Without the checks, wrapped decoding costs almost
nothing over unwrapped (SSE4.1 168 against 166 µs, AVX2 132 against
130 µs): G2's decode overhead appears with the validation. Profiles
(`perf`, share of cycles):

| | kernel | driver | byte-at-a-time path |
|---|---|---|---|
| AVX2 decode, unwrapped | 95.6 % | 2.5 % | — |
| AVX2 decode, W = 64 | 87.2 % | 10.1 % | 1.9 % |
| SSE4.1 decode, W = 64 | 91.4 % | 5.9 % | 1.9 % |
| AVX2 encode, unwrapped | 99.5 % | — | — |
| AVX2 encode, W = 64 | 85.4 % | 14.6 % | — |

- Decode: the kernel takes the same absolute time wrapped or not; the
  extra is the driver's per-line work (terminator checks, the loop that
  counts the lines of a batch), which overlaps with the kernel when it
  does not validate, and the first line: W is unknown until the first
  terminator, so the first batch (up to 1 024 characters) fails on it and
  is re-read byte by byte, once per stream (1.9 % at 1 MiB, more for
  smaller streams).
- Encode: the driver's loop that writes one LF per line after the kernel
  call, bounds-checked (14.6 %).

#### Efficiency core

`taskset -c 4`, vCPUs 5–7 idle, same trees and method. The spread between
rounds is mostly under 1 %.

| Bench | (a) flat | (b) out of line | (c) nested | spread |
|---|---|---|---|---|
| encode, scalar | 808–823 µs | 771–785 µs | 807–808 µs | 5.2 % |
| encode, SSE4.1 | 332 µs | 334–335 µs | 333–334 µs | 0.7 % |
| encode, AVX2 | 329 µs | 328–329 µs | 327–328 µs | 0.4 % |
| decode, scalar | 990–992 µs | 837–844 µs | 835–836 µs | 18.6 % |
| decode, SSE4.1 | 269 µs | 272–274 µs | 267 µs | 2.3 % |
| decode, AVX2 | 243–246 µs | 252–254 µs | 303–305 µs | 24.0 % |
| encode W = 64, scalar | 850–872 µs | 802–811 µs | 807–812 µs | 7.1 % |
| encode W = 64, SSE4.1 | 363–364 µs | 364–367 µs | 366–367 µs | 0.8 % |
| encode W = 64, AVX2 | 359–360 µs | 381–382 µs | 357 µs | 7.0 % |
| decode W = 64, scalar | 1019–1021 µs | 895 µs | 881–885 µs | 15.6 % |
| decode W = 64, SSE4.1 | 297–298 µs | 307–308 µs | 297 µs | 3.6 % |
| decode W = 64, AVX2 | 274 µs | 302–304 µs | 334–335 µs | 22.1 % |

On an efficiency core, AVX2 decoding is the most sensitive (24 %, the
nested loop the slowest), then the scalar decoder. The "twice as slow"
SSE4.1 case of spec 0010 does not reproduce with the cluster idle.

| Decode | full checks | without 8191 | range only | no checks | cost |
|---|---|---|---|---|---|
| scalar | 836–839 µs | 716–718 µs | 717–719 µs | 574–577 µs | +45.5 % |
| SSE4.1 | 272–273 µs | 250 µs | 236 µs | 217 µs | +25.7 % |
| AVX2 | 245 µs | 222–223 µs | 207–208 µs | 192–193 µs | +27.3 % |
| scalar, W = 64 | 880–887 µs | 767–768 µs | 767–768 µs | 582–583 µs | +51.4 % |
| SSE4.1, W = 64 | 306–308 µs | 291–292 µs | 270–271 µs | 220–222 µs | +39.1 % |
| AVX2, W = 64 | 273–276 µs | 244–245 µs | 234–235 µs | 195–197 µs | +40.2 % |

Without the checks, wrapping again costs about 2 %. Profiles, W = 64:
AVX2 decode 85.2 % kernel, 12.0 % driver, 1.6 % byte-at-a-time path;
SSE4.1 decode 86.7 % kernel loops, 10.6 % driver; AVX2 encode 91.2 %
kernel, 8.6 % driver.

### Stage 2 — Wrapping (2026-09-28)

Changes: encoder batches capped at 64 blocks, their LFs written through a
strided iterator (one bounds check per batch); the first line of a wrapped
stream decoded in probe batches of 2 blocks, doubling on success; batches
of whole lines up to 256 blocks (`LINE_BATCH`).

A strided `chunks().take_while()` scan to count the lines of a batch was
tried and dropped: it made wrapped decoding 7–9 % slower on both core
types (isolated by variants, performance core: 244–248 µs against
229–230 µs with the former counting loop, SSE4.1, W = 64).

Quick harness (§1), five alternating rounds, fastest round; wrapping
overhead = wrapped (W = 64) over unwrapped:

| | performance core, before → after | efficiency core, before → after |
|---|---|---|
| encode SSE4.1 | 10.6 → 9.0 % | 8.8 → 4.3 % |
| encode AVX2 | 10.9 → 14.1 % | 7.6 → 3.7 % |
| decode SSE4.1 | 18.7 → 11.5 % | 14.6 → 8.0 % |
| decode AVX2 | 11.6 → 8.5 % | 13.8 → 7.6 % |

Unwrapped speed unchanged (within the ±2 % noise of the performance core).
G2 is met on the efficiency core. On the performance core, AVX2 encode
(14.1 %) and SSE4.1 decode (11.5 %) remain: the driver's share of wrapped
AVX2 encoding is down to about 5 % (from 14.6 %), the rest is inside the
kernel loop (two AVX2 units per 64-character line), which is stage 4's
subject; the decode overhead is linked to validation (stage 3).

## Verification

```bash
cd rust
cargo test --workspace --features base91-rs/reference-tests,base91-cli/reference-tests,base91-fuzz/reference
taskset -c 8 cargo bench -p base91-rs --bench throughput      # nothing else running
taskset -c 8 cargo test --release -p base91-rs --test perf_sanity -- --ignored --nocapture
nix-build ../default.nix -A rust-tests -A rust-tests-full -A rust-tests-aarch64 \
  -A rust-nostd -A rust-clippy -A rust-fmt
```

`reuse lint` must pass.
