<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# basE91 Performance Analysis

Machine: Intel Core Ultra 7 165U, AC power, turbo enabled.
Compilers: rustc 1.91.1 (LLVM), clang 21.1.7.
Bench tool: Criterion (100 samples, 1 MiB random input).
Measured at commit 410fde9.

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
| Rust `encode_unchecked` | ~953 MiB/s | ~1.23 GiB/s |
| Rust `encode` (safe, `spare_capacity_mut`) | ~774 MiB/s | ~1011 MiB/s |
| base91 v0.1.0 (dnsl48) | ~717 MiB/s | ~899 MiB/s |

The safe API uses `spare_capacity_mut` + `set_len` to keep `ptr`
register-resident — `Vec::push` forces LLVM to spill and reload the
pointer because `grow_one()` may reallocate.

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
| scalar fixed-width | ~1.13 GiB/s | ~1.72 GiB/s |
| simd128 (SSE4.1)   | ~4.52 GiB/s | ~5.11 GiB/s |
| simd256 (AVX2)     | ~8.24 GiB/s | ~5.44 GiB/s |
| Henke `encode_unchecked` (reference) | ~953 MiB/s | ~1.23 GiB/s |

### 3.4 Observations

**Encode: simd256 is ~1.8× simd128** — AVX2 processes two 13-byte blocks
per iteration vs one for SSE4.1, roughly halving iteration count.

**Decode: simd256 ≈ simd128** (~5.4 vs ~5.1 GiB/s, within noise).  Both
kernels perform the SIMD character unmap in hardware, then fall through to
a scalar u128 bit-pack (8 OR-shift operations per 13-byte block).  AVX2
processes 32 chars but runs two sequential bit-packs; the extra work nearly
cancels the gain from the wider unmap.  A vectorised scatter step would
push decode throughput toward encode levels.

**Scalar fixed-width outperforms Henke on both encode and decode.**
Encode: ~1.13 GiB/s vs ~953 MiB/s.  Decode: ~1.72 GiB/s vs ~1.23 GiB/s.
The Henke loop has a serial `queue` loop-carried dependency (`or`/`imul`
bottleneck visible under `perf annotate`); the scalar fixed-width path
unrolls 13-byte blocks 8 pairs deep, breaking the dependency chain.

---

## 4. Open optimisation opportunities

- **SIMD decode scatter:** replace the scalar u128 bit-pack with a SIMD
  scatter (SSE4.1 `pshufb`-based or AVX2 equivalent).  Expected to push
  decode throughput toward encode levels.
- **NEON / SVE2:** aarch64 kernels not yet benchmarked.
- **`enc_char` / `dec_char` peephole:** LLVM emits `cmp; adc $-1; lea`
  (3 instructions) instead of the optimal `cmp; sbb $0xDC` (2 instructions)
  for the alphabet gap correction.  Tracked for future inline-asm work.
