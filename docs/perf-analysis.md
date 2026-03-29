<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# basE91 Performance Analysis

Machine: Intel Core Ultra 7 165U, CPU-pinned VCPUs at 2688 MHz (E-cores).
Compilers: rustc 1.91.1 (LLVM), clang 21.1.7.
Bench tool: Criterion (100 samples, 1 MiB deterministic input, seed 0xdeadbeef_cafebabe).
Measured at commit ad07897.

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
| Rust `encode_unchecked` | ~719 MiB/s | ~720 MiB/s |
| Rust `encode` (safe, `spare_capacity_mut`) | ~519 MiB/s | ~572 MiB/s |
| base91 v0.1.0 (dnsl48) | ~409 MiB/s | ~527 MiB/s |

The safe API uses `spare_capacity_mut` + `set_len` to keep `ptr`
register-resident — `Vec::push` forces LLVM to spill and reload the
pointer because `grow_one()` may reallocate.

### 2.4 Profiling: Henke hotspots (`perf record -e cpu-clock`)

**Encode** — 99% of samples in `encode_unchecked`.  Top instructions:

| % | Instruction | Role |
|---|---|---|
| 5.7 | `movzbl (%rdi,%r10),%r9d` | load input byte |
| 5.3 | `or %r11d,%r9d` | accumulate byte into queue |
| 5.3 | `cmp $0x5,%ecx` / `jbe` | check nbits ≥ 6 |
| 5.2 | `shl %cl,%r9d` | variable shift into queue |
| 4.8 | `mov %r9d,%r11d` | save queue snapshot |
| 3.5 | `imul $0xb41,%r11d,%ebx` | multiply-shift divide by 91 |
| 3.1 | `shr $0x12,%ebx` | complete the divide |

Samples spread across the whole loop body with no single dominant instruction.
The `shl %cl` (variable-count shift, 3-cycle latency) and `or` form the
loop-carried queue dependency; the `imul`/`shr` divide-by-91 runs in parallel.

**Decode** — 99% in `decode_unchecked`.  Samples similarly spread;
no instruction above 3.2%.  The `shl %cl,%r10d` (variable shift) and
`or %r9d,%r10d` (~2.6–2.9% each) form the same serial queue dependency.
The two-scanner loop eliminates the `val` sentinel branch, leaving the
shift chain as the primary constraint.

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
| scalar fixed-width | ~767 MiB/s | ~1.04 GiB/s |
| simd128 (SSE4.1)   | ~2.91 GiB/s | ~2.61 GiB/s |
| simd256 (AVX2)     | ~2.99 GiB/s | ~3.42 GiB/s |
| Henke `encode_unchecked` (reference) | ~719 MiB/s | ~720 MiB/s |

### 3.4 Observations

**Encode: simd256 ≈ simd128** (~2.99 vs ~2.91 GiB/s, within noise) on
pinned 2688 MHz E-cores.  These cores likely throttle AVX2 throughput —
the wide 256-bit path gains nothing over SSE4.1 at this frequency.
Profiling (§3.5) shows the dominant cost is `vpmullw`/`vpaddw` (25% combined)
for the divide-by-91, not the memory bandwidth or shuffle pipeline.

**Decode: simd256 > simd128** (~3.42 vs ~2.61 GiB/s).  The AVX2 path
processes 32 chars per iteration before the scalar bit-pack, so two
bit-packs amortise the wider SIMD unmap better at this clock speed.

**Scalar fixed-width outperforms Henke on both encode and decode.**
Encode: ~767 MiB/s vs ~719 MiB/s.  Decode: ~1.04 GiB/s vs ~720 MiB/s.
See §2.4 for the Henke bottleneck analysis.

### 3.5 Profiling: SIMD hotspots (`perf record -e cpu-clock`)

**simd128 encode** — 94% of samples in `encode_sse41`.  Samples spread
across the pipeline: `add %rbx` (loop counter, 8.8%), `paddb` (3.7%),
`movdqa` (2.9%), `movdqu` store (4.1%), `psubb` (2.9%), `pblendw` (2.5%).
No single dominant bottleneck — pipeline is reasonably balanced.

**simd128 decode** — 96% in `decode_sse41`.  Single dominant hotspot:
`shr $0xc,%r8d` at **22.4%**, followed by `pextrw $0x3,%xmm5,%ebx` at
**11.2%**.  Both are in the scalar u128 bit-pack reconstructing 13 bytes
from 8 pextrw/shift/or operations.  The bit-pack is the clear bottleneck.

**simd256 encode** — 95% in `encode_avx2`.  Two instructions tie at
**12.6%** each: `vpmullw` and `vpaddw` — the paired multiply-add that
implements divide-by-91 in SIMD.  These have latency ≥4 cycles and form
a dependency chain.  `add %rbx` loop counter at 7.4%, `vpunpcklbw` at
4.3%, `vpcmpgtb`/`vpsubb` gap correction at 4.1%/3.8%.

**simd256 decode** — 96% in `decode_avx2`.  `test %r8d,%r8d` (invalid
character check) at **6.3%**, `vpcmpgtb` unmap at 2.8%, then a cluster
of `vpextrw`/shift/`or` for two bit-packs (3–5% each).  Compared to the
SSE4.1 case the bottleneck is more distributed across the two bit-packs.

---

## 4. Open optimisation opportunities

- **SIMD decode scatter:** profiling confirms the scalar u128 bit-pack
  is the dominant cost in both `decode_sse41` (`shr` at 22.4%, single
  hotspot) and `decode_avx2` (`vpextrw`/shift/`or` cluster).  Replacing
  it with a `pshufb`-based scatter should push decode throughput toward
  encode levels.
- **AVX2 encode divide-by-91:** `vpmullw`/`vpaddw` tie at 12.6% each —
  the multiply-add dependency chain for div-91.  An alternative with fewer
  dependent ops (e.g. `vpmulhuw` alone or a reciprocal multiply) may help.
- **NEON / SVE2:** aarch64 kernels not yet implemented or benchmarked.
- **`enc_char` / `dec_char` peephole:** LLVM emits `cmp; adc $-1; lea`
  (3 instructions) instead of the optimal `cmp; sbb $0xDC` (2 instructions)
  for the alphabet gap correction.  Tracked for future inline-asm work.
