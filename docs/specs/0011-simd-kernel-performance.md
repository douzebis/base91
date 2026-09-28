<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0011 — SIMD kernel performance: validation cost, wrapping overhead, robust code generation

**Status:** implemented
**App:** base91
**Implemented in:** 2026-09-28

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
  evidence (open question 1). *Relaxed by decision 4: at most 30 %.*
- **G2 — Wrapping overhead:** wrapped (W = 64) within 10 % of unwrapped,
  encode and decode, SSE4.1 and AVX2. *Relaxed by decision 5: within 25 %
  on a performance core.*
- **G3 — Robust code generation:** one loop structure for the three SIMD
  kernels (SSE4.1, AVX2, NEON), no per-kernel special case, and each
  code path's speed insensitive to the loop around it: the three loop
  shapes of *Background* within 5 % of each other, for SSE4.1, AVX2 and
  the scalar path (NEON: not measurable, see *Non-goals*). *Relaxed by
  decision 6: one loop structure, and no spill or register shuffling in
  the compiled loops of any shape.*
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
- **The efficiency cores are warmed up** for a few seconds before their
  rounds: after the performance-core rounds, the first efficiency-core
  runs are up to 12 % slow, for every build alike.
- **The performance core's clock is not constant:** 3.70–4.11 GHz with
  the same binary, depending on the code and the data, and drifting down
  within a 20-second run (power or thermal limit; the efficiency core held
  3.68 GHz). A difference of a few percent on the performance core alone
  can be the clock, not the code; it counts when the efficiency core
  agrees or when cycle counts confirm it.
- **Hardware counters** come from the host: the VM has no virtual PMU.
  `perf stat -I 2000 -t <vCPU thread>` on the host, with the `cpu_core/…/G`
  or `cpu_atom/…/G` events of the vCPU's core type, while the VM runs
  20-second phases of each build in turn.

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
protocol of §1; if no formulation reaches 3 %, open question 1 decides
(decided: decision 4).

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

4. **G1 relaxed to the measured cost** (open question 1, decided with
   the stage 3 results): exact checks take four vector operations per
   block with this alphabet, +17 to +27 % of the new kernels' decode
   throughput. Spec 0008 §3.5's SHOULD becomes "at most 30 %" (the
   highest measured cost, 27 %, rounded up for run-to-run noise). The gap
   is not a trigger of the assembly gate (§6): assembly cannot remove the
   four operations.
5. **G2 relaxed on performance cores:** wrapped within 25 % of unwrapped
   there (measured: encode +20 to +23 %, decode +10 to +15 %); 10 % still
   holds on efficiency cores (+6 to +12 %, SSE4.1 decode at 12 %
   included as measured). The per-line costs (the inner loop's restart,
   the terminator) did not grow; the kernels became 30–60 % faster, so
   they weigh more. Wrapped streams are still faster than ever (AVX2
   encode, W = 64: 66.1 → 38.3 µs).
6. **G3 judged on the compiled code, not on the 5 % spread.** With the
   stage 3 kernels, no loop shape makes the compiler spill or rotate
   registers (the failure G3 was written against); the shapes still
   differ by more than 5 %, but through the loop's own work: a line
   counter per block in the flat shape (about 5 instructions against a
   kernel of about 18), a call per line in the out-of-line shape (a
   64-column line is 4 blocks). G3 is met with one structure, nested
   loops, for every kernel, and clean compiled loops.

## Open questions

None.

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

### Stage 3 — Validation and decode kernels, first part (2026-09-28)

Quick harness (§1), five alternating rounds, fastest round; each change
measured on its own against the tree before it, on both core types.
(Absolute times drift between runs, by up to 15 % when the host is busy;
only comparisons within a run count. The A/B script checks a reference
first and refuses to run when the machine is slowed down.)

| # | Change | SSE4.1 decode | AVX2 decode |
|---|---|---|---|
| 1 | Running maximums of the indices (`'` forced to 0xFF) and of the pair values, compared once per batch, instead of subtract-and-OR per block | validation cost 23 → 17 % (P), 25 → 14 % (E), unwrapped | 14 → 11 % (P), 26 → 22 % (E) |
| 2 | `pmaddubsw` for the pair values (1 instruction instead of 6) | −19 to −25 % | — |
| 3 | same, AVX2 | — | −13 to −18 % |
| 4 | Packing with `pmaddwd` (value pairs into 26-bit fields), a per-lane shift (0, 2, 4, 6 bits; a multiply on SSE4.1) and two byte shuffles, instead of 7 shifts, 6 blends and 4 shuffles | −18 to −32 % | — |
| 5 | same, AVX2 (`vpsllvd`) | — | −31 to −49 % |
| 6 | SSE4.1: one validity test per batch, nested loop inlined (was: an out-of-line call and a test per line) | −1.5 to −16 % | — |
| 7 | Terminators checked after the kernel call, branch-free, no counting loop before it | −2 to −5 % (W = 64) | −2 to −5 % (W = 64) |

Tried and dropped: a flat loop for SSE4.1 with the new kernel (+7 to
+19 %); line batches of 64 blocks instead of 256 (+4 to +9 %).

Change 5's size: the former AVX2 packing issued about 8 byte shuffles per
32 characters, all on the one shuffle port of a performance core; the new
loop issues 2–3, about 20 instructions per unit instead of about 40.

Result against the stage 2 commit (`6f198ef`):

| Decode | performance core | efficiency core |
|---|---|---|
| SSE4.1, unwrapped / W = 64 | 88.9 → 50.1 µs (−44 %) / 99.2 → 54.8 µs (−45 %) | 132.6 → 66.3 µs (−50 %) / 143.0 → 74.2 µs (−48 %) |
| AVX2, unwrapped / W = 64 | 69.5 → 29.4 µs (−58 %) / 74.6 → 31.1 µs (−58 %) | 118.1 → 64.4 µs (−46 %) / 126.9 → 68.9 µs (−46 %) |

Encoding and the scalar paths are unchanged (not touched yet).

- G2 on decode: wrapping now costs +6 to +12 % (SSE4.1 on the efficiency
  core still just over 10 %).
- G1: validation still costs 8–34 % of the (much faster) decoder; its
  absolute cost went down (AVX2, performance core: 4.7 µs per 512 KiB,
  from 6.7 µs).

Against 0.2.2 (`07ce3ea`, the published state), one-shot `Vec` API,
512 KiB: SIMD decode with validation is 2.1–2.9 times faster unwrapped and
9–14 times faster wrapped than 0.2.2 without it. The legacy codec
regressed: encode +12 % (performance core), decode +37 % (efficiency core)
— see the next section.

### Stage 3 — Legacy decoder (2026-09-28)

**Cause of the efficiency-core regression: code placement, not code.**
The legacy decoding loop has the same source in 0.2.2 and 0.3.0; only its
surroundings changed (the slice core of spec 0006). Host counters,
efficiency core, 512 KiB of random data, `Vec` API:

| Build | µs/run | IPC | branch misses |
|---|---|---|---|
| 0.2.2 | 1063 | 3.94 | 0.16 % |
| 0.3.0 and stage 3 part 1 | 1469 | 3.04 | 0.19 % |
| same, loops aligned to 64 bytes (`-C llvm-args=-align-loops=64`) | 3294 | 1.51 | ≤ 0.19 % |

The clock is constant (3.68 GHz) and the branch misses negligible (0 on
all-zero data); the alignment padding adds only 6–19 % more instructions.
The loop is front-end bound: the efficiency core has no micro-op cache,
and a loop of about two short blocks per input byte, with two taken
branches, decodes at a speed set by where those blocks fall. Moving the
loop out of line and restoring 0.2.2's exact loop shape changed nothing; a
branch-free drain helped the efficiency core but was 25 % slower on the
performance core.

**Fix: one pair per iteration.** While no character is pending, the
decoder takes two input bytes at a time (`[b0, b1, tail @ ..]`), looks
both up, and decodes the pair in one straight block when both are in the
alphabet: one taken branch per pair instead of about two per byte. A
junk byte, a pending character or the last byte fall back to the former
byte-at-a-time path, so behavior is unchanged (the matrix and legacy
tests pass as they are).

Host counters, 512 KiB, `Vec` API, cycles per run from the measured clock:

| Build | efficiency core: µs (cycles), IPC | performance core: µs (cycles), IPC |
|---|---|---|
| 0.2.2 | 1066 (3.92 M), 3.94 | 632 (2.46 M), 6.22 |
| stage 3 part 1 | 1492 (5.49 M), 3.04 | 658 (2.45 M), 6.68 |
| one pair per iteration | 863 (3.18 M), 3.90 | 502 (1.91 M), 6.47 |

- About 12.4 M instructions per run, against 15.4 M (0.2.2) and 16.7 M;
  the same counts on both core types. Cycles −19 % (efficiency core) and
  −22 % (performance core) against 0.2.2.
- The performance-core "regression" of 0.3.0 was the clock: the same
  cycle count as 0.2.2 (2.45 M against 2.46 M), at 3.72 GHz instead of
  about 3.9.
- Branch misses 0.17–0.26 % on random data (0.12–0.16 % before), 0 on
  zeros.
- **Placement no longer matters:** loops aligned to 32 or 64 bytes give
  the same speed (quick harness, efficiency core 831.2 / 832.2 / 831.3 µs,
  performance core 445.8 / 451.4 / 445.0 µs); the same flag made the
  former loop 2.2 times slower.

Two variants were measured and dropped: a single junk test
(`d0.max(d1) == 91`, one `cmova` and one branch) was +6.7 % on the
performance core and even on the efficiency core; indexing (`input[i]`,
`input[i + 1]`) instead of the slice pattern left a redundant bounds check
in the loop, 3–5 % slower on both core types.

The legacy encoder is unchanged. Against 0.2.2 (`Vec` API, fastest of five
rounds) it is +5 % on the performance core (534 against 506 µs) and even on
the efficiency core (923 against 924 µs), not the +12 % measured before;
the difference is within the clock variation above, not confirmed by
cycle counts.

### Stage 3 — Encoders (2026-09-28)

The encoders' unpacking mirrors the decoders' packing (stage 3, changes 4
and 5): one byte shuffle gathers bytes 3k..3k+3 into 32-bit lane k, a
per-lane left shift by 6 − 2k (a multiply on SSE4.1, `vpsllvd` on AVX2)
aligns the four 26-bit fields on bits 6..31, and two shifts, a blend and
an AND split them into the eight 13-bit values. It replaces 2 shuffles,
7 shifts, 6 blends, 2 ANDs and a pack.

The assembly showed a second cost, older than this change: LLVM fuses the
division's multiply-high (`_mm_mulhi_epu16`) and the 16-bit shift after
it into a 32-bit multiply of widened values: 7 instructions instead of 2,
and on AVX2 two cross-lane ones (`vextracti128`, `vpermq`). Scaling the
values so that their top bits carry data does not prevent it (LLVM then
uses `pmulld`). A 32-bit shift followed by a mask does: 3 instructions
(`pmulhw`, `psrld`, `pand`). The loop is now 22 instructions per block
(SSE4.1) and 20 per unit of two blocks (AVX2).

Quick harness (§1), five alternating rounds, fastest round, each change
against the tree before it:

| # | Change | performance core, W = 0 / 64 | efficiency core, W = 0 / 64 |
|---|---|---|---|
| 1 | SSE4.1 unpacking | −17 / −14 % | −20 / −20 % |
| 2 | SSE4.1 division unfused | −21 / −16 % | −20 / −18 % |
| 3 | AVX2 unpacking | −17 / −16 % | −15 / −14 % |
| 4 | AVX2 division unfused | −27 / −29 % | −28 / −27 % |

Result against the legacy-decoder commit, same harness:

| Encode | performance core | efficiency core |
|---|---|---|
| SSE4.1, W = 0 / 64 | 102.4 → 69.8 µs (−32 %) / 114.3 → 80.2 µs (−30 %) | 161.2 → 103.9 µs (−36 %) / 168.5 → 110.6 µs (−34 %) |
| AVX2, W = 0 / 64 | 58.9 → 35.6 µs (−40 %) / 66.1 → 39.9 µs (−40 %) | 158.5 → 97.0 µs (−39 %) / 164.4 → 103.8 µs (−37 %) |

Decoding, unchanged, measured as a control: within 0.2 % on the
efficiency core; on the performance core +5.6 % once, −0.2 % when re-run
(the clock variation of §1). NEON is not changed yet.

### Stage 3 — Validation folded into the arithmetic (2026-09-28)

Validation cost with the stage 3 kernels, quick harness, unwrapped (the
build with the checks removed, against the full checks):

| Decode | performance core | efficiency core |
|---|---|---|
| SSE4.1 | 37.4 → 45.7 µs (+22 %) | 55.1 → 66.2 µs (+20 %) |
| AVX2 | 26.0 → 30.5 µs (+17 %) | 50.9 → 64.5 µs (+27 %) |

Its absolute cost went down (SSE4.1, performance core: about 8 µs per
512 KiB, against about 18 µs in the baseline, 37 µs per MiB), its share
up: the four validation operations (`'` compare, OR, two running
maximums) stayed while the rest of the SSE4.1 kernel went from about 30
to about 11 operations. Removing only one of the two maximums gives inconsistent
figures (on the efficiency core, AVX2 without the value check is 5 %
*slower*): only the total is reliable. The wrapped figures of builds
without the index check are not measurable: the first-line probe relies on
it to find the first terminator.

**Folding the checks into the arithmetic** (one running maximum catching
every error through the pair value) is not possible with this alphabet:

- An invalid *low* character has weight 1 in `lo + 91 hi`: with a small
  high index, the pair value stays small. Seeing it needs a second
  weighted sum (`91 lo + hi`), which costs what the maximum it replaces
  costs. Signed indices do not help either (a negative low index and a
  positive high one still sum to a small positive value).
- `'`: removing a one-character gap with one threshold maps it onto a
  neighbor's index, whatever the formula; separating it takes a
  non-monotonic operation (an equality test) and a merge. `psignb` fails
  by a half-integer offset, `pabsb` reverses the order of the low range,
  and a byte shuffle repeats every 16 values.

Four operations per block is therefore the floor for exact checks; at
about 4–5 % per operation in these issue-bound kernels, 3 % (G1) is out of
reach. One variant was measured and dropped: OR instead of the maximum for
the pair values (exact: a valid value never has bits 13–15 set, and values
stay below 23 461): no change on the efficiency core, −3.3 to +2.0 % on
the performance core. Open question 1 was decided with these figures
(decision 4).

### Stage 3 — NEON (2026-09-28)

The NEON kernels follow the new x86 arithmetic, with NEON's own
instructions where they save work:

- decode: pair values as `x − 165 (x >> 8)` on the 16-bit lanes of the
  indices (one shift and one multiply-subtract; NEON has no `pmaddubsw`);
  fields joined by shift-and-insert (`vsliq_n_u32`, 13 bits); a variable
  shift by 0, 2, 4, 6; two table lookups and an OR;
- encode: one table lookup gathers the bytes, a variable shift by 3, 1,
  −1, −3 aligns every field on bits 3..28, a 16-bit variable shift and an
  AND split them; the division by a doubling multiply-high
  (`vqdmulhq_n_s16`, 2881) and a shift by 3; the character pairs as
  `v + 165 hi` = `lo + 256 hi`, without a zip.

The generated code (aarch64 test build) has 10 vector instructions per
encode block and 11 plus the 4 of validation per decode block, with no
widening. Speed not measurable here (*Non-goals*); the aarch64 library
tests pass under qemu, and the aarch64 `no_std` build succeeds.

### Stage 4 — Kernel shape (2026-09-28)

**Loop shapes with the stage 3 kernels** (quick harness, fastest of five
alternating rounds; each shape applied to all six batch loops; percentages
against the committed tree, whose loops were nested except AVX2 decode):

| | flat, P / E | out of line, P / E | nested, P / E |
|---|---|---|---|
| encode scalar, W = 0 / 64 | +32 / +15 %, +11 / +9 % | −1 / −4 %, −4 / −1 % | +1 / −2 %, 0 / −3 % |
| encode SSE4.1, W = 0 / 64 | +8 / +3 %, +1 / −1 % | −1 / +6 %, 0 / +1 % | −1 / +4 %, +1 / 0 % |
| encode AVX2, W = 0 / 64 | +4 / +10 %, −2 / −1 % | 0 / +12 %, 0 / +12 % | −6 / −2 %, 0 / 0 % |
| decode scalar, W = 0 / 64 | +17 / +9 %, +19 / +17 % | +4 / +5 %, 0 / +2 % | +4 / +2 %, 0 / 0 % |
| decode SSE4.1, W = 0 / 64 | +15 / +4 %, +19 / +10 % | +4 / +13 %, +1 / +23 % | −2 / −4 %, 0 / 0 % |
| decode AVX2, W = 0 / 64 | +4 / 0 %, 0 / 0 % | −10 / +41 %, +1 / +26 % | −9 / −3 %, −1 / +1 % |

In every shape the SIMD loops have no spill and no register-to-register
vector move beyond the copies two-operand SSE needs. Nested loops are the
fastest or even everywhere: AVX2 decode, the last flat loop, is now
nested too (−9 / −3 % on the performance core, ±1 % on the efficiency
core), and all kernels share one structure (decision 6).

**Unrolled by two** (§3): SSE4.1 decode +5.4 % unwrapped on the
performance core, +5.6 / +7.1 % on the efficiency core (no micro-op cache:
a longer loop body costs more there than the loop overhead it saves).
Dropped, and not tried on the other kernels for the same reason.

**Constants built once per batch** (§3): each x86 kernel takes a struct of
its constants, built by its batch function. The compiled batch functions
are instruction for instruction the same as without it (LLVM already
hoisted every constant into a register); measured: within 0.5 % on the
efficiency core, −3.9 to +8.7 % on the performance core for the same
machine code (its clock variation, §1). Kept: it states the intent in the
source instead of relying on the compiler. NEON keeps its constants
inline (not measurable here).

### Stage 5 — Gate and release (2026-09-28)

**Goals, final:**

- G1 (decision 4, at most 30 %): met, +17 to +27 %.
- G2 (decision 5, at most 25 % on performance cores, 10 % on efficiency
  cores): met on performance cores (encode +20 to +23 %, decode +10 to
  +15 %); on efficiency cores met except SSE4.1 decode, at +12 %.
- G3 (decision 6): met: nested loops for every kernel, no spill or
  register shuffling in any shape.
- G4: met. Criterion, 1 MiB, performance core, against the 0.3.0 figures
  of spec 0010 (*Stage 1 results*):

| | 0.3.0 | 0.3.1 |
|---|---|---|
| decode, unwrapped: scalar / SSE4.1 / AVX2 | 489–492 / 201 / 147–149 µs | 508 / 121 / 73 µs |
| decode, W = 64 (AVX2) | 168–170 µs | 71 µs |
| encode, unwrapped: scalar / SSE4.1 / AVX2 | 367–372 / 215–218 / 119–120 µs | 389 / 147 / 77 µs |
| encode, W = 64 (AVX2) | 138–140 µs | 95 µs |
| streaming decode, unwrapped / W = 64 | 164–165 / 196–197 µs | 82 / 94 µs |
| legacy encode / decode (`Vec` API) | 1057–1066 / 1223–1242 µs | 1129 / 1041 µs |

  The scalar paths and the legacy encoder, which this spec does not
  change, read 3–6 % slower in this single run; the quick harness,
  alternating 0.3.0 and 0.3.1 builds on both core types, puts them within
  1.6 % (scalar encode +0.4 / +0.6 %, scalar decode −1.6 / +0.2 %, legacy
  encode +0.1 / 0.0 %; legacy decode −24 / −36 %): the performance core's
  clock (§1). On the efficiency core, criterion: SIMD decode SSE4.1 /
  AVX2 138 / 136 µs (0.3.0: 273 / 245), encode 215 / 202 µs (335 / 329).

**Assembly gate** (§6): not triggered. G1 and G3 are met as decided (4,
6), and assembly could not remove what remains: the four validation
operations per block, and the per-line work of wrapped streams.

**Tests** (§7): the workspace tests with the reference features, the Nix
checks (x86_64 and aarch64 under qemu, `no_std` on both, clippy, fmt)
and 61 seconds of fuzzing per target pass. `perf_sanity` re-calibrated
at 70 % of the lowest ratios of six performance-core runs:
Simd128/Scalar 1.75 / 3.18 (was 1.15 / 1.76), wrapped/unwrapped 0.49 /
0.59 (was 0.59 / 0.58: lower for encode, decision 5), Simd256/Simd128
still capped at 0.90.

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
