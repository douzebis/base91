<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# basE91 Performance Analysis

Machine: Intel Core Ultra 7 165U, pinned to P-cores 0+2 (no HT sibling contention).
Compilers: rustc 1.91.1 (LLVM), clang 21.1.7.
Bench tool: Criterion (100 samples, 1 MiB deterministic input, seed 0xdeadbeef_cafebabe).
Bench numbers in §2 measured at commit e30b49e.
Bench numbers in §3 measured at commit 069742d (pshufb scatter + branch elimination + direct store).

---

## 1. Algorithm properties

basE91 is a bit-queue state machine.  State: `queue` (bit accumulator),
`nbits` (bits in queue), `val` (pending decode value).

**Encoding:** each input byte is shifted into `queue`.  When `nbits > 13`
the encoder peeks 13 bits — if the value ≤ 88 it steals 14 bits instead.
Both cases emit 2 characters via table lookup (`enctab[val % 91]`,
`enctab[val / 91]`).

**Decoding:** each input character is mapped through `dectab[256]`.  Valid
chars are consumed in pairs: `val = d1 + d2*91`.  The same 13/14-bit
decision pushes `val` back into the queue and drains 1–2 bytes.

**SIMD barrier:** the 13/14-bit steal is data-dependent, so block
boundaries are unpredictable.  SIMD is impossible on the Henke path.

---

## 2. Henke path: C vs Rust

### 2.1 Key C disassembly findings (gcc -O2)

- `queue` and `nbits` are **reloaded from memory every iteration** because
  GCC cannot prove the output buffer `o` does not alias the state struct `b`.
- Division by 91 uses a multiply-shift (`imul` with magic constant) — no
  `idiv`.
- Decode uses `cmp + adc` to select 13 vs 14 bits branchlessly.

### 2.2 Rust improvements over C

| Issue | C (gcc -O2) | Rust unchecked |
|---|---|---|
| `queue`/`nbits` aliasing | mem load/store every iter | register-resident ✓ |
| `/91` divide | multiply-shift ✓ | multiply-shift ✓ |
| 13/14 select (encode) | well-predicted branch | well-predicted branch ✓ |
| 13/14 select (decode) | `cmp+adc` ✓ | `cmp+adc` ✓ |
| Drain loop | loop + mem round-trips | two write sites, no loop ✓ |
| Output write | raw pointer, no check | raw pointer (`encode_unchecked`) ✓ |

Key fixes required to reach this state:

1. **`ENCTAB.get_unchecked()`** — without it LLVM inserted `jae` panic
   branches for every table lookup, consuming front-end bandwidth.

2. **Duplicate writes per arm** — with shared write code at the bottom of
   both 13/14-bit arms, LLVM merged them into a `cmovae`/`setae` +
   variable-count `shr cl` (3-cycle latency, dep chain through flags).
   Duplicating the writes into each arm gives LLVM immediate-count shifts
   and separate, well-predicted branches.

3. **Two-scanner decode loop** — rewriting `decode_unchecked` as two nested
   skip-loops (one for d0, one for d1) with the emit block between them
   matches GCC's layout and eliminates the `val` sentinel branch.

4. **`__restrict__` on C** — adding `__restrict__` to `src/base91.c` lets
   GCC hoist `queue`/`nbits` into registers (+57% encode, +154% decode).

5. **Clang -O3 for C** — Clang beats GCC on decode (better register
   allocation); C encode required the duplicate-writes fix here too.

### 2.3 Henke benchmark results (1 MiB random input)

| Implementation | Encode | Decode |
|---|---|---|
| Rust `encode_unchecked` | ~993 MiB/s | ~1.15 GiB/s |
| Rust `encode` (safe, `spare_capacity_mut`) | ~841 MiB/s | ~872 MiB/s |
| base91 v0.1.0 (dnsl48) | ~624 MiB/s | ~784 MiB/s |

The safe API uses `spare_capacity_mut` + `set_len` to keep `ptr`
register-resident — `Vec::push` forces LLVM to spill and reload the
pointer because `grow_one()` may reallocate.

### 2.4 Profiling: Henke hotspots (`perf record -e cpu-clock`)

**Encode** — 99% of samples in `encode_unchecked`.  Top instructions:

| % | Instruction | Role |
|---|---|---|
| 10.7 | `imul $0xb41,%r11d,%ebx` | multiply-shift divide by 91 |
| 10.5 | `or %r11d,%r9d` | accumulate byte into queue |
| 10.2 | `shr $0x12,%ebx` | complete the divide |
| 7.6 | `mov %r11b,(%rdx,%rax)` | write lo output char |
| 7.5 | `jmp cdab` | jump to table lookup |
| 6.7 | `movzbl (%rbx,%r8),%r11d` | table lookup hi char |
| 5.0 | `cmp $0x5,%ecx` / `jbe` | check nbits ≥ 6 |
| 4.6 | `shl %cl,%r9d` | variable shift into queue |

The `imul`/`shr` divide-by-91 (21% combined) and the `or` queue
accumulation (10.5%) are the dominant costs.  The `shl %cl` variable-count
shift has 3-cycle latency and forms the loop-carried queue dependency
(`or → shl → or`), but the `imul` running in parallel accounts for similar
sample weight.

**Decode** — 99% in `decode_unchecked`.  Top: `or %r9d,%r10d` (7.8%,
accumulate val into queue), `adc $0xffffffff,%r11d` (5.1%, branchless
13/14-bit select), `mov %r10b,(%rdx,%rax)` (4.7%, write output byte),
`lea 0x1(%rax),%rbx` (4.6%), `lea 0x6(%r11),%ecx` (3.4%).  The `shl %cl`
variable shift (3.5%) and `or` form the same serial queue dependency as
in encode.

**Scalar fixed-width encode is faster than Henke** despite more arithmetic
per output byte.  `perf annotate` shows samples spread across all 8 unrolled
`ingest` sites (≤2% each) with no dominant hotspot — the unrolled 13-byte
blocks break the serial `queue` dependency by giving the CPU independent
work to schedule between each group emission.

---

## 3. SIMD fixed-width variant

### 3.1 Design

The SIMD variant uses fixed-width 13-bit groups (8 groups per 13-byte
block → 16 output chars) and a contiguous 91-char alphabet (0x23–0x5B,
0x5D–0x7E, omitting `\`).  The leading `-` byte distinguishes SIMD
streams from Henke streams.

The scalar path uses the same block structure as the SIMD kernels: 16-byte
decode blocks with an optional `\n` skip at each boundary, and 13-byte
encode blocks unrolled 8 pairs deep with `spare_capacity_mut` output.

`dec_char` uses branchless arithmetic: `b.wrapping_sub(0x23).wrapping_sub((b > 0x5C) as u8)` —
three instructions (`cmp $0x5D; adc $-1; lea -35`).

**Input validity:** all three decode paths (scalar, SSE4.1, AVX2) assume
well-formed input.  Non-alphabet bytes (other than an optional `\n` at each
16-char block boundary) silently corrupt the decoded output rather than
returning an error.  Callers are responsible for ensuring the input stream
contains only SIMD-alphabet characters.

### 3.2 SIMD kernels (x86_64)

**SSE4.1** (13 bytes → 16 chars per call, 128-bit XMM):
`pshufb` gathers bit-group bytes into 32-bit lanes; four `psrld` +
`pblendw` blends select the right shift per group; `pmulhuw` + `pmullw`
divide by 91; `pshufb` + `punpcklbw` interleave lo/hi indices;
`paddb` + `pcmpgtb` + `psubb` apply the alphabet gap correction.

Decode reverses this: character unmap via `pcmpgtb` + `paddb`, validation
via `pcmpgtb`, then `pshufb` to separate lo/hi, `pmullw` to reconstruct
`val = lo + hi*91`, then a scalar u128 bit-pack to reconstruct 13 bytes.

**AVX2** (26 bytes → 32 chars per call, 256-bit YMM):
Same pipeline as SSE4.1 using `_mm256_*` equivalents, applied to two
independent 13-byte blocks in the low and high 128-bit lanes respectively.
No cross-lane interaction.

### 3.3 SIMD benchmark results (1 MiB random input)

Decode throughput is measured on encoded bytes (encoded ≈ 1.23× input).

| Path | Encode | Decode |
|---|---|---|
| scalar fixed-width | ~1.52 GiB/s | ~2.31 GiB/s |
| simd128 (SSE4.1)   | ~4.40 GiB/s | ~6.25 GiB/s |
| simd256 (AVX2)     | ~7.68 GiB/s | ~8.57 GiB/s |
| Henke `encode_unchecked` (reference) | ~993 MiB/s | ~1.15 GiB/s |

### 3.4 Observations

**Encode: simd256 is ~1.75× simd128** (~7.68 vs ~4.40 GiB/s) on P-cores
with full-width AVX2 execution.  Each AVX2 iteration processes two 13-byte
blocks, roughly halving iteration count vs SSE4.1.

**Decode: pshufb scatter replaces scalar bit-pack.**  Before: simd128
~4.78 GiB/s, simd256 ~5.12 GiB/s (bit-pack dominated at 29.4% for SSE4.1).
After: simd128 ~6.25 GiB/s (+31%), simd256 ~8.57 GiB/s (+68%).
simd256 is now ~1.37× simd128 on decode (was ~1.07×), matching encode scaling.

**Scalar fixed-width outperforms Henke on both encode and decode.**
Encode: ~1.52 GiB/s vs ~993 MiB/s (+53%).  Decode: ~2.31 GiB/s vs ~1.15 GiB/s (+101%).
Both gains come from eliminating fixed-pattern branches per block when nbits=0:
encode removes 13 `if nbits >= 13` checks; decode removes 8 `if nbits >= 8`
second-drain checks.  In both cases the emit positions are compile-time
constants at block entry, so the generic macro is split into
`_no_emit!`/`_emit!` (encode) and `_single!`/`_double!` (decode) variants
used at their hardcoded positions.
See §2.4 for the Henke bottleneck analysis.

### 3.5 Profiling: SIMD hotspots (`perf record -e cpu-clock`)

**simd128 encode** — 94% in `encode_sse41`.  Top: `paddb` (19.2%),
`psubb` (9.6%), `movdqa` (8.7%), `packusdw` (7.3%), `packuswb` (7.9%),
`psrld` (2.6%).  The `paddb`/`psubb` pair implements the alphabet gap
correction and dominates.  No single-instruction bottleneck; pipeline is
reasonably balanced across the encode stages.

**simd128 decode** — scalar bit-pack bottleneck eliminated by pshufb scatter
(+31% throughput).  Profiling not yet re-run after the scatter rewrite.

**simd256 encode** — 95% in `encode_avx2`.  Top: `vpaddb` (20.8%),
`vpunpcklbw` (10.6%), `vpaddw` (11.1%), `vpackuswb` (7.2%+),
`vinserti128` (5.7%), `vpmullw` (4.3%).  The `vpaddb`/`vpaddw` pair
(alphabet correction + div-91 add) dominates.  `vpmullw` itself is only
4.3% — the multiply is not the bottleneck; the dependent `vpaddw` absorbs
the latency cost.

**simd256 decode** — scalar bit-pack bottleneck eliminated by pshufb scatter
(+68% throughput).  Profiling not yet re-run after the scatter rewrite.

---

## 4. Open optimisation opportunities

- **AVX2 encode alphabet correction:** `vpaddb` at 20.8% is the top
  hotspot — the `+0x23` base offset add after the gap correction.  Together
  with `vpunpcklbw`/`vpackuswb` (interleave/pack) this sequence accounts
  for ~40% of samples.  Restructuring the output layout to avoid the
  interleave step could reduce this cost.
- **NEON / SVE2:** aarch64 kernels not yet implemented or benchmarked.
- **`enc_char` / `dec_char` peephole:** LLVM emits `cmp; adc $-1; lea`
  (3 instructions) instead of the optimal `cmp; sbb $0xDC` (2 instructions)
  for the alphabet gap correction.  Tracked for future inline-asm work.
