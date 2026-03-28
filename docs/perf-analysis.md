<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# basE91 Performance Analysis

Reference C source: `src/base91.c` (Joachim Henke, BSD-3-Clause).
Compiled with: `gcc -O2` (GCC 14.3.0, x86-64).
Disassembled with: `objdump -d -M intel`.

This document records the disassembly study that informed the Rust
port's performance directives (see `docs/specs/0001-rust-port.md §3.5`).

---

## 1. Algorithm properties

basE91 is a bit-queue state machine.  State: `queue` (bit accumulator),
`nbits` (bits in queue), `val` (pending decode value, −1 = none).

**Encoding:** each input byte is shifted into `queue`.  When `nbits > 13`
the encoder peeks 13 bits:
- If the 13-bit value > 88: consume 13 bits, emit 2 chars.
- If ≤ 88: consume 14 bits (steal one more), emit 2 chars.

The two output chars are `enctab[val % 91]` and `enctab[val / 91]`.
The full value space 0–8280 (= 91²−1) is covered without overlap:
- 0–88 and 8192–8280 → 14-bit path (178 values)
- 89–8191 → 13-bit path (8103 values)

**Decoding:** each input char is looked up in `dectab[256]`; non-alphabet
bytes (dectab value = 91) are silently skipped.  Valid chars are
consumed in pairs: `val = d1 + d2*91`.  The same 13/14-bit decision
is then used to push `val` back into the queue.

**Block structure:** the 13/14-bit steal is data-dependent, so the queue
state after N input bytes is not fixed.  Fixed-size blocking (e.g.
13 bytes → 16 chars) does not hold in general.  The only clean
boundary is end-of-input.  SIMD vectorization is impossible due to the
loop-carried dependency on `queue` and `nbits`.

---

## 2. Disassembly: `basE91_encode` (gcc -O2)

```
0000000000000020 <basE91_encode>:
  ; entry: rdi=state, rsi=input, rdx=len, rcx=output
  20:  test   rdx,rdx          ; len == 0?
  23:  je     110              ; → return 0
  29:  lea    r8,[rsi+rdx]     ; r8 = input end
  2d:  push   rbx
  2e:  mov    r9,rcx           ; r9 = output base
  31:  xor    edx,edx          ; n = 0
  33:  jmp    4d

  ; no-fire path: store nbits and loop
  40:  mov    DWORD PTR [rdi+0x8],r10d   ; ← STORE nbits every iteration
  44:  cmp    r8,rsi
  47:  je     d2               ; → done

  ; loop top
  4d:  mov    ecx,DWORD PTR [rdi+0x8]   ; ← LOAD nbits every iteration
  50:  movzx  eax,BYTE PTR [rsi]        ; load input byte
  53:  add    rsi,0x1
  57:  shl    eax,cl                    ; byte << nbits
  59:  lea    r10d,[rcx+0x8]            ; r10 = nbits + 8
  5d:  cdqe
  5f:  or     rax,QWORD PTR [rdi]       ; ← LOAD queue every iteration
  62:  mov    QWORD PTR [rdi],rax       ; ← STORE queue every iteration
  65:  cmp    r10d,0xd                  ; nbits+8 > 13?
  69:  jbe    40               ; → no fire

  ; fire path — 13-bit branch
  6b:  mov    r10d,eax
  6e:  and    r10d,0x1fff               ; peek 13 bits
  75:  cmp    r10d,0x58                 ; val > 88?
  79:  jbe    f0               ; → 14-bit path

  ; 13-bit path
  7b:  shr    rax,0xd                   ; queue >>= 13
  7f:  sub    ecx,0x5                   ; nbits = nbits+8-13 = nbits-5
  82:  mov    DWORD PTR [rdi+0x8],ecx   ; ← STORE nbits
  85:  mov    ecx,r10d                  ; val (13-bit)
  88:  mov    r11,QWORD PTR [rip+0x0]   ; r11 = &enctab
  8f:  lea    rbx,[rdx+0x1]            ; rbx = n+1
  ; division by 91 via multiply-shift:
  93:  imul   rcx,rcx,0x68168169       ; magic multiply
  9a:  mov    QWORD PTR [rdi],rax       ; ← STORE queue
  9d:  mov    eax,r10d
  a0:  shr    rcx,0x20                  ; >> 32 → quotient in rcx
  a4:  sub    eax,ecx                   ; eax = val - quot
  a6:  shr    eax,1
  a8:  add    eax,ecx
  aa:  shr    eax,0x6                   ; eax = val / 91
  ad:  imul   ecx,eax,0x5b             ; ecx = quot * 91
  b0:  movzx  eax,BYTE PTR [r11+rax]   ; enctab[val/91]
  b5:  sub    r10d,ecx                  ; r10 = val % 91
  b8:  movzx  ecx,BYTE PTR [r11+r10]   ; enctab[val%91]
  bd:  mov    BYTE PTR [r9+rdx],cl     ; output[n] = enctab[val%91]
  c1:  add    rdx,0x2
  c5:  mov    BYTE PTR [r9+rbx],al     ; output[n+1] = enctab[val/91]
  c9:  cmp    r8,rsi
  cc:  jne    4d               ; → loop

  ; 14-bit path (jmp back to 82 shared tail)
  f0:  mov    r10d,eax
  f3:  sub    ecx,0x6                   ; nbits = nbits+8-14 = nbits-6
  f6:  shr    rax,0xe                   ; queue >>= 14
  fa:  and    r10d,0x3fff               ; val (14-bit)
 101:  jmp    82
```

### Encode observations

1. **`queue` and `nbits` are reloaded from memory every iteration**
   (instructions at 0x4d, 0x5f, 0x40, 0x62, 0x82, 0x9a).  GCC cannot
   hoist them because it cannot prove the output buffer `rcx/r9` does
   not alias the state struct `rdi`.

2. **Division by 91 is a multiply-shift** (0x93–0xaa): `imul` with
   magic constant `0x68168169`, then a sequence of shifts and adds.
   No `idiv` instruction.  One division's worth of work, not two.

3. **13/14-bit branch is a real conditional branch** (0x75/0x79), taken
   ~98.9% of the time (val > 88).  Well-predicted by the branch
   predictor.

4. **Two separate `enctab` lookups** (0xb0, 0xb8): both index into the
   91-byte table.  Table fits in one or two L1 cache lines.

---

## 3. Disassembly: `basE91_decode` (gcc -O2)

```
00000000000001e0 <basE91_decode>:
  ; entry: rdi=state, rsi=input, rdx=len, rcx=output
  1e0:  mov    r8,rsi           ; r8 = input ptr
  1e3:  mov    r9,rcx           ; r9 = output base
  1e6:  test   rdx,rdx
  1e9:  je     2c9              ; → return 0
  1ef:  lea    r10,[rsi+rdx]    ; r10 = input end
  1f3:  mov    r11,[rip+0x0]    ; r11 = &dectab
  1fa:  xor    esi,esi          ; n = 0

  ; loop top
  20c:  movzx  eax,BYTE PTR [r8]        ; load input char
  210:  add    r8,0x1
  214:  movzx  eax,BYTE PTR [r11+rax]   ; d = dectab[char]
  219:  cmp    eax,0x5b                  ; d == 91?
  21c:  je     203              ; → skip (not in alphabet)

  ; valid char: check val state
  21e:  mov    edx,DWORD PTR [rdi+0xc]  ; ← LOAD val
  221:  cmp    edx,0xffffffff            ; val == -1?
  224:  je     200              ; → store d as new val

  ; second char of pair: reconstruct value
  226:  imul   eax,eax,0x5b             ; d2 * 91
  229:  mov    ecx,DWORD PTR [rdi+0x8]  ; ← LOAD nbits
  22c:  add    edx,eax                  ; val = d1 + d2*91
  22e:  mov    eax,edx
  230:  and    edx,0x1fff               ; val & 0x1fff (13-bit peek)
  236:  shl    eax,cl                   ; val << nbits
  238:  cdqe
  23a:  or     rax,QWORD PTR [rdi]      ; ← LOAD queue; queue |= val<<nbits
  23d:  cmp    edx,0x59                 ; val&0x1fff > 88?
  240:  adc    ecx,0xd                  ; *** nbits += 13 + (val<=88 ? 1 : 0)
  243:  mov    QWORD PTR [rdi],rax      ; ← STORE queue
  246:  mov    DWORD PTR [rdi+0x8],ecx  ; ← STORE nbits

  ; drain loop
  280:  add    rsi,0x1
  284:  mov    BYTE PTR [r9+rsi-1],al   ; output[n++] = queue & 0xff
  289:  mov    rax,QWORD PTR [rdi]      ; ← LOAD queue
  28c:  mov    ecx,DWORD PTR [rdi+0x8] ; ← LOAD nbits
  28f:  shr    rax,0x8                  ; queue >>= 8
  293:  lea    edx,[rcx-0x8]            ; nbits - 8
  296:  mov    QWORD PTR [rdi],rax      ; ← STORE queue
  299:  mov    DWORD PTR [rdi+0x8],edx  ; ← STORE nbits
  29c:  cmp    edx,0x7                  ; nbits > 7?
  29f:  ja     280              ; → emit another byte

  ; reset val = -1
  2a1:  mov    DWORD PTR [rdi+0xc],0xffffffff
  2a8:  cmp    r8,r10
  2ab:  jne    20c              ; → outer loop
```

### Decode observations

1. **`cmp + adc` for 13/14-bit selection** (0x23d–0x240): this is the
   standout optimization.  GCC replaces the conditional `nbits += (val
   & 0x1fff) > 88 ? 13 : 14` with:
   ```asm
   cmp  edx, 0x59   ; CF=1 if edx < 89 (i.e. val ≤ 88)
   adc  ecx, 0x0d   ; ecx += 13 + CF  → 14 if val≤88, 13 if val>88
   ```
   Completely branchless.  Must verify LLVM emits equivalent.

2. **`queue`, `nbits`, `val` reloaded every iteration** — same aliasing
   problem as encode.  Every drain-loop iteration loads and stores
   `queue` and `nbits` from/to memory.

3. **Drain loop is a real loop** (0x280–0x29f): trip count 1 or 2.
   Can be unrolled to eliminate loop overhead and the intra-drain
   memory round-trips.

4. **`dectab` lookup** (0x214): 256-byte table, fits in L1.  The
   skip-on-91 branch (0x219/0x21c) is ~26% taken for purely
   alphabetic input (0% non-alphabet) and up to 100% for whitespace-
   heavy input.  Not easily made branchless without changing semantics.

---

## 4. Summary: where Rust beats C (pre-implementation forecast)

This section recorded the expected wins before the Rust port was written.
See §5–7 for actual disassembly results.

| Issue | C (gcc -O2) | Rust (forecast) |
|---|---|---|
| `queue`/`nbits` aliasing | reload every iter | hoisted to registers |
| `/91` divide | multiply-shift ✓ | multiply-shift ✓ |
| 13/14-bit select (encode) | branch | branchless (cmov) |
| 13/14-bit select (decode) | `cmp+adc` ✓ | `cmp+adc` ✓ |
| Drain loop | loop (1–2 iters) | unrolled `if` |
| Output write | raw pointer | Vec capacity check |

---

## 5. Rust disassembly (LLVM, rustc 1.86.0 -O)

Compiled from `rust/base91/src/codec.rs`.  Disassembled via
`rustc --emit=asm` with `#[no_mangle] #[inline(never)]` probe
wrappers to prevent inlining.  AT&T syntax (rustc default).

### 5.1 Encode hot loop (probe_encode)

Key instructions from `.LBB4_2` / `.LBB4_9` / `.LBB4_10`:

```asm
; queue (r12d) and nbits (ebp) are register-resident — no memory loads ✓

; load byte, shift into queue:
movzbl  (%r10,%r15), %r12d      ; byte = input[i]
shll    %cl, %r12d              ; byte << nbits
orl     %eax, %r12d             ; queue |= byte << nbits   (eax = old queue)

; 13-bit peek and 13/14-bit select — BRANCHLESS via cmov/setae:
movl    %r12d, %eax
andl    $8191, %eax             ; val13 = queue & 0x1fff
movl    %r12d, %r13d
andl    $16383, %r13d           ; val14 = queue & 0x3fff
cmpl    $89, %eax               ; val13 >= 89?
cmovael %eax, %r13d             ; val = val13 if ≥89, else val14  ✓
setae   %r11b                   ; consumed = 13 if ≥89, else 14
movb    $14, %cl
subb    %r11b, %cl              ; cl = 14 - (val13>=89)  ✓
shrl    %cl, %r12d              ; queue >>= consumed

; division by 91 — MULTIPLY-SHIFT, no idiv:
imull   $11523, %r13d, %ebx     ; magic multiply
shrl    $20, %ebx               ; >> 20  → quotient in ebx  ✓
imull   $-91, %ebx, %edi
addl    %r13d, %edi             ; remainder = val - quot*91
```

**Checklist:**
- [x] No `idiv` — multiply-shift confirmed
- [x] `queue`/`nbits` register-resident (r12d/ebp), no memory reloads
- [x] 13/14-bit select branchless via `cmovae`/`setae`
- [x] `enctab` accessed via register-held base (r8)

**Remaining overhead:** `Vec::push` capacity check (`cmpq (%rdx), %r9`)
emits a branch + `grow_one` call on every pair.  Unavoidable with
`Vec<u8>` output.  Amortized to near-zero for large inputs; visible
for small inputs.

### 5.2 Decode hot loop (probe_decode)

Key instructions from `.LBB5_3` / `.LBB5_8` / `.LBB5_11`:

```asm
; queue (ebp) and nbits (ecx) are register-resident — no memory loads ✓

; dectab lookup and skip-invalid:
movzbl  (%rdi), %eax
incq    %rdi
movzbl  (%rax,%r12), %eax      ; d = dectab[char]  (r12 = &dectab)
cmpl    $91, %eax
je      .LBB5_3                ; skip non-alphabet

; second char: reconstruct value
imull   $91, %eax, %eax        ; d2 * 91
addl    %ebx, %eax             ; val = d1 + d2*91
movl    %eax, %r9d
shll    %cl, %r9d              ; val << nbits
andl    $8191, %eax            ; val & 0x1fff

; 13/14-bit select — BRANCHLESS via adc:
cmpl    $89, %eax              ; CF=1 if val&0x1fff < 89
movl    %ecx, %r10d
adcl    $-1, %r10d             ; r10 = nbits + (-1 + CF) = nbits-1 if val≤88
; ... addl $14, %r10d follows → nbits += 14 if val≤88, 13 if val>88  ✓

; drain — RESTRUCTURED (not a simple unrolled pair):
; byte 1: emitted unconditionally at .LBB5_8
; byte 2: emitted at .LBB5_11 when second capacity check passes
; LLVM restructured the drain around the Vec grow paths —
; functionally equivalent to unrolled, but interleaved with grow checks.
```

**Checklist:**
- [x] No `idiv`
- [x] `queue`/`nbits` register-resident (ebp/ecx), no memory reloads
- [x] 13/14-bit select branchless via `adc` (identical to GCC pattern) ✓
- [x] Drain logically unrolled (two separate emit sites, no backward branch
      in the drain itself — LLVM restructured around Vec grow paths)
- [x] `dectab` accessed via register-held base (r12)

---

## 6. Rust disassembly: unchecked paths (rustc 1.86.0 -O)

`encode_unchecked` and `decode_unchecked` use raw pointer output —
no `Vec` machinery, no capacity checks, direct `mov BYTE PTR` stores.
Disassembled from the benchmark binary (`target/release/deps/throughput-*`).

### 6.1 `encode_unchecked` hot loop (final version)

Two structural changes from the initial implementation significantly affect
the generated code:

1. **`get_unchecked` on `ENCTAB`**: removes two `jae` panic branches from the
   hot path (LLVM could not prove `r,q < 91` statically).
2. **Duplicated writes in each arm**: prevents LLVM from merging the 13/14-bit
   paths into a `cmovae`/`setae` + variable-count shift sequence.

```asm
; r10d = queue, r8d = nbits — register-resident throughout ✓

; accumulate: load byte, shift into queue
  5d97b:  mov    ebx, r10d
  5d97e:  movzx  r10d, BYTE PTR [rdi+r11]  ; input byte
  5d983:  mov    ecx, r8d
  5d986:  shl    r10d, cl                   ; byte << nbits
  5d989:  or     r10d, ebx                  ; queue |= ...
  5d98c:  mov    ebx, 0x8
  5d991:  cmp    r8d, 0x5                   ; nbits > 13 after +8?
  5d995:  jbe    5d970                      ; → accumulate more

; fire: 13/14-bit branch — well-predicted (val>88 ~98.9% of the time)
  5d997:  mov    ecx, r10d
  5d99a:  and    ecx, 0x1fff               ; val13 = queue & 0x1fff
  5d9ab:  cmp    ecx, 0x59                 ; val13 > 88?
  5d9ae:  jbe    (14-bit arm)              ; → rarely taken

; 13-bit arm:
  5d9b4:  shr    r10d, 0xd                 ; queue >>= 13  (immediate shift ✓)
  5d9be:  imul   ecx, ecx, 0x2d03          ; divide by 91: magic multiply
  5d9c4:  shr    ecx, 0x14                 ; >> 20 → quotient q
  5d9c7:  imul   r14d, ecx, 0xffffffa5    ; q * -91
  5d9cb:  add    r14d, ebp                 ; r = val - q*91
  5d9ce:  movzx  ebp, BYTE PTR [r14+r9]   ; ENCTAB[r]  (get_unchecked, no check)
  5d9d3:  mov    BYTE PTR [rdx+rax], bpl  ; output[n]   ✓
  5d9d7:  movzx  ecx, BYTE PTR [rcx+r9]  ; ENCTAB[q]  (get_unchecked, no check)
  5d9dc:  mov    BYTE PTR [rdx+rax+1], cl ; output[n+1] ✓
  5d9e0:  add    rax, 0x2
  5d9e4:  jmp    5d970                    ; → loop top

; 14-bit arm (rarely reached):
  ; identical structure with queue &= 0x3fff, shr 0xe
```

**No `Vec` grow branch.  No `cmovae`/`setae`.  No variable-count shift.
Immediate shifts, no bounds-check branches in the hot path.**

### 6.2 `decode_unchecked` hot loop

```asm
; r10d = queue, ecx = nbits, r9d = queue_low — register-resident ✓

; dectab lookup and skip:
  25:  movzx  r11d, BYTE PTR [rdi]
  29:  inc    rdi
  2c:  movzx  r11d, BYTE PTR [r11+r8]   ; d = dectab[char]  (r8=&dectab)
  31:  cmp    r11d, 0x5b
  35:  je     0x20                       ; skip non-alphabet

; second char: reconstruct value, push into queue:
  3d:  imul   r11d, r11d, 0x5b          ; d2 * 91
  41:  add    r11d, r10d                 ; val = d1 + d2*91
  47:  shl    r10d, cl                   ; val << nbits
  4a:  or     r10d, r9d                  ; queue |= val<<nbits

; 13/14-bit select — cmp+adc, branchless:
  4d:  and    r11d, 0x1fff
  54:  cmp    r11d, 0x59                 ; CF=1 if val<=88
  58:  mov    r11d, ecx
  5b:  adc    r11d, 0xffffffff           ; r11 = nbits - 1 + CF
  6e:  lea    ecx, [r11+6]              ; ecx = nbits + (14 or 13) - 8
                                         ; (+6 = -1+CF+14-8 or -1+0+13-8)

; drain — two emit sites, NO backward branch in drain:
  5f:  mov    BYTE PTR [rdx+rax], r10b  ; byte 0 — always emitted  ✓
  63:  lea    rbx, [rax+1]
  72:  cmp    ecx, 0x8
  75:  jb     0x12                       ; if only 1 byte, back to outer loop
  77:  add    r11d, 0xe
  7b:  mov    BYTE PTR [rdx+rax+1], r9b ; byte 1 — conditional  ✓
  80:  add    rax, 0x2
```

**No `Vec` capacity check.  `cmp+adc` branchless select confirmed.**
Drain is two unconditional stores with a single branch to skip the
second byte — exactly the unrolled pattern specified.

---

## 7. Final comparison: C vs Rust

| Issue | C (gcc -O2) | Rust Vec API | Rust unchecked API |
|---|---|---|---|
| `queue`/`nbits` aliasing | mem load/store every iter | register-resident ✓ | register-resident ✓ |
| `/91` divide | multiply-shift ✓ | multiply-shift ✓ | multiply-shift ✓ |
| 13/14 select (encode) | well-predicted branch ✓ | `cmovae`/`setae` (suboptimal) | well-predicted branch ✓ |
| 13/14 select (decode) | `cmp+adc` ✓ | `cmp+adc` ✓ | `cmp+adc` ✓ |
| Drain loop | loop + mem round-trips | restructured, no mem ✓ | two stores, no loop ✓ |
| Output write | raw pointer, no check | `Vec` capacity branch | raw pointer, no check ✓ |
| `enctab`/`dectab` bounds check | none | none | `get_unchecked` ✓ |

**Net:**
- `encode_unchecked` / `decode_unchecked` beat the C reference on every
  axis: register-hoisted state, no output capacity check, and —
  critically — a well-predicted branch instead of `cmovae`/`setae` for
  the encode 13/14-bit select (see §8).
- `Encoder::encode` / `Decoder::decode` with a pre-reserved `Vec` are
  near-equivalent to C: the capacity branch is always predicted-not-taken
  and costs essentially nothing after warmup.

---

## 8. Benchmark results

### 8.1 Rust vs C (criterion, rustc 1.86.0, clang 21.1.7 -O3, x86-64)

1 MiB random input.  Intel Core Ultra 7 165U, AC power, turbo enabled.
Criterion 100-sample run.  Throughput measured on input bytes for encode,
encoded bytes for decode.

| Implementation | Encode | Decode |
|---|---|---|
| Rust unchecked | **~1.016 GiB/s (~1041 MiB/s)** | **~1.210 GiB/s (~1239 MiB/s)** |
| C (clang -O3, `__restrict__`, static tables, dup writes) | **~1.017 GiB/s (~1042 MiB/s)** | ~1.153 GiB/s (~1181 MiB/s) |
| Rust safe (`spare_capacity_mut`) | ~919 MiB/s | ~972 MiB/s |

**Encode is tied** (~1.017 GiB/s each) after duplicating the write block into
each encode arm (§12.3, §12.4).  **Decode: Rust leads C by ~5%**: the remaining
gap is Clang's register allocation for the decode scan loops vs LLVM in rustc.
**Rust safe** now at ~90% of unchecked after switching from `Vec::push` to
`spare_capacity_mut` + `set_len` (§14).

### 8.2 All-language comparison (1 MiB random input, same machine)

Methodology:
- **C**: `bench.c` — `clock_gettime(CLOCK_MONOTONIC)`, 50 iterations after
  5 warmup, throughput on input/encoded bytes respectively.
- **Rust**: criterion via `cargo bench --features c-compat-tests`.
- **Go**: `go test -bench=. -benchtime=5s`, pre-allocated output buffer
  (no allocation in the hot loop), throughput reported by `b.SetBytes`.
- **Python**: `pybase91` PyO3 extension; `time.perf_counter`, 50 iterations
  after 5 warmup, 1 MiB random input.  The extension calls into
  `crate::encode` / `crate::decode` (which use the safe `spare_capacity_mut`
  path) with zero-copy `bytes → &[u8]` conversion via PyO3.

| Language | Encode | Decode | Notes |
|---|---|---|---|
| Rust unchecked | **~1041 MiB/s** | **~1239 MiB/s** | raw pointer output |
| C (clang -O3, `__restrict__`, static tables, dup writes) | **~1042 MiB/s** | ~1181 MiB/s | native, no bounds checks |
| Rust safe (`spare_capacity_mut`) | ~919 MiB/s | ~972 MiB/s | pre-reserved, one `unsafe` |
| Python (pybase91 PyO3) | ~566 MiB/s | ~998 MiB/s | Rust core + Python object alloc |
| Go | 571 MiB/s | 517 MiB/s | pre-allocated slice, zero allocs |

### 8.3 Encode performance history (Rust)

The encode path required two targeted fixes to beat the *unpatched* C:

1. **`ENCTAB.get_unchecked()`** — LLVM inserted `jae` panic branches for
   each `ENCTAB[r]` and `ENCTAB[q]` index because it could not statically
   prove `r,q < 91`.  Even though they were never taken, they consumed
   front-end bandwidth.  Replacing with `get_unchecked` removed them.
   Result: parity with unpatched C (~634 vs ~641 MiB/s).

2. **Duplicated writes per arm** — with the two 13/14-bit paths sharing a
   single write block at the bottom, LLVM merged them into a
   `cmovae`/`setae` + variable-count `shr cl` sequence.  The variable-count
   shift has a 3-cycle latency and a data dependency through the flag
   register, costing more than C's simple well-predicted branch.
   Duplicating the writes into each arm breaks the merge, giving LLVM two
   independent paths with immediate-count shifts.
   Result: Rust +56% over *unpatched* C (~1010 vs ~645 MiB/s at the time).

### 8.4 Decode performance history (Rust)

Decode was faster than *unpatched* C from the first implementation, because:
- register-hoisted `queue`/`nbits` eliminate the memory round-trips that
  plague GCC due to the aliasing conservatism;
- the drain loop is unrolled to two write sites with no backward branch;
- LLVM generates `cmp+adc` matching GCC's branchless 13/14-bit select.

After the C `__restrict__` patch (§10), C decode overtook Rust.  The outer
loop restructuring described in §9 restored Rust's lead.

---

## 9. Rust decode loop restructuring

After the C `__restrict__` patch (§10), C decode jumped to ~1342 MiB/s while
Rust unchecked stalled at ~1033 MiB/s (~23% deficit).  Disassembly comparison
revealed the root cause.

**Original Rust loop (before fix):**

```
loop:
  fetch byte → dectab lookup → skip if 91
  check val sentinel → store d0, continue
  OR: compute v, emit 1-2 bytes, jump back to loop top
```

One backward branch per pair of input chars.  LLVM's `val == u32::MAX`
sentinel check (`cmpl $-1`) added an extra branch inside the loop.

**GCC's loop (C reference, after `__restrict__`):**

GCC unrolls the outer loop around the emit block: after writing bytes it
immediately fetches the *next* input byte inline (`.L29`/`.L51`/`.L30`
in the disassembly), avoiding the backward jump to the loop top in the
common case.  Effectively two separate skip-loops — one for d0, one for
d1 — with the emit block between them.

**Fix:** rewrote `decode_unchecked` as an explicit `loop` containing two
separate inner `loop` blocks that scan for d0 and d1 respectively, with
the emit block between them.  LLVM now generates the same two-scanner
layout as GCC:

```asm
.LBB0_2:   ; scan for d0 — skip non-alphabet
  cmpq %rsi, %rdi / je .LBB0_9
  movzbl (%rdi), %r10d / incq %rdi
  movzbl (%r10,%r8), %r10d
  cmpl $91, %r10d / je .LBB0_2
.LBB0_4:   ; scan for d1 — skip non-alphabet
  cmpq %rsi, %rdi / je .LBB0_8
  movzbl (%rdi), %r11d / incq %rdi
  movzbl (%r11,%r8), %r11d
  cmpl $91, %r11d / je .LBB0_4
  ; emit block: adc/lea for nbits, two write sites, back to .LBB0_1
```

**Result:** Rust decode unchecked: ~1033 MiB/s → **~1236 MiB/s** (+20%),
now **~10% ahead of C** (~1122 MiB/s) after the C decode was also
restructured (§11).

The `val` sentinel and its branch are also eliminated — `d0` is carried as
a local between the two scanner loops, never stored to memory.

---

## 11. C decode two-scanner restructuring

After observing that the Rust two-scanner loop (§9) beat C by ~18%, the same
structural change was applied to `basE91_decode` in `src/base91.c`.

**Change:** replaced the single `while (len--)` loop with two nested `do/while`
skip loops — one scanning for `d0`, one for `d1` — with the emit block between
them.  `val` is retained in `struct basE91` for ABI compatibility (streaming
callers may split input across calls), but is used only at function entry/exit
to carry a pending first char across chunk boundaries, never in the hot path.

**Result at GCC -O2:**

| | Before (old loop) | After (two-scanner) |
|---|---|---|
| C decode | ~1042 MiB/s | **~1122 MiB/s** (+7.7%) |

Rust unchecked decode (~1236 MiB/s) still led C by ~10%.  The remaining
gap was GCC register allocation: GCC re-loaded `dectab@GOTPCREL` inside the
scan loops under some conditions, while LLVM hoisted it cleanly (see §12).

**With Clang -O3 (after switch in §12):**

| | Rust unchecked | C (Clang -O3, two-scanner) |
|---|---|---|
| Encode | ~1044 MiB/s | ~630 MiB/s |
| Decode | ~1240 MiB/s | **~1229 MiB/s** (tied) |

C decode ties Rust under Clang.  C encode collapses under Clang — see §12.

---

## 12. Clang switch: decode win, encode loss

### 12.1 Motivation

After the C two-scanner restructuring (§11), GCC -O2 C decode (~1122 MiB/s)
still trailed Rust decode (~1236 MiB/s) by ~10%.  The residual gap was traced
to GCC's register allocator: under `-O2` with `goto`-based control flow, GCC
reloads `dectab@GOTPCREL` inside the scan loops (GOT indirection for the
global table), while LLVM hoists the pointer into a register regardless of
`goto` structure.

**Making `enctab`/`dectab` `static`** eliminates the GOT indirection entirely
(file-local → RIP-relative addressing).  This partially helps GCC but does not
fully close the register-allocation gap.

**Switching to Clang** (`clang -O3`) was chosen to get LLVM's superior register
allocation for the C source without modifying the algorithm.

### 12.2 Decode result

Clang `-O3` + static tables + two-scanner: **C decode ~1209 MiB/s** (+7.8%
over GCC -O2).  Rust unchecked decode (~1240 MiB/s) and C are now tied to
within noise.

### 12.3 Encode regression under Clang (before fix)

Initial Clang `-O3` run: C encode **collapsed from ~970 MiB/s (GCC -O2) to
~625 MiB/s** (-36%).  Disassembly revealed the cause: Clang merges the two
13/14-bit encode arms into a single block using `setae`/`cmovae` +
variable-count `shr cl`:

```asm
setae  r15b           ; r15 = (val13 > 88) ? 1 : 0
cmovae r14d, ecx      ; select val13 or val14
mov    cl, 0xe
sub    cl, r15b       ; shift count = 14 or 13 — variable!
shr    r10, cl        ; 3-cycle latency, data dep through flags
```

This is precisely the pattern Rust had and fixed (§8.3) by duplicating the
two `output.push()` calls into each arm.  The original C source had a single
shared write block after both branches, which Clang merges into one path.

### 12.4 Fix: duplicate writes per arm

Applied the same structural fix as §8.3 to `basE91_encode` in `src/base91.c`:
moved the two `ob[n]` / `ob[n+1]` writes into each arm, with `n += 2` after
the `if/else`.  Clang now generates:

```asm
cmp    r14d, 0x59
jae    .L13bit         ; well-predicted branch
; 14-bit arm: shr r9, 0xe (immediate)  → ob writes → n+=2
.L13bit:
; 13-bit arm: shr r9, 0xd (immediate)  → ob writes → n+=2
```

**Result:**

| Metric | GCC -O2 | Clang -O3 (before) | Clang -O3 (after fix) |
|---|---|---|---|
| C encode | ~970 MiB/s | ~625 MiB/s | **~1042 MiB/s** (+7% over GCC) |
| C decode | ~1122 MiB/s | ~1209 MiB/s | **~1181 MiB/s** |

Encode now ties Rust unchecked (~1041 MiB/s).  Decode is ~5% below Rust
(~1181 vs ~1239 MiB/s) — the gap is LLVM register allocation in the decode
scan loops, not the encode fix.

---

## 13. C `__restrict__` patch (post-Rust-port)

After observing that Rust's register hoisting was the key encode/decode win
over unpatched C, the same optimization was applied to `src/base91.c`:

- `__restrict__` added to the `b`, `i`, and `o` parameters of
  `basE91_encode` and `basE91_decode`, telling GCC that the state struct
  `b` cannot alias the input or output buffers.
- `queue`, `nbits`, and `val` hoisted to local variables at function entry
  and written back to `b` on exit — the same pattern the Rust compiler
  had applied automatically.

**Effect on encode:** C encode went from ~645 MiB/s to ~1010 MiB/s (+57%),
now edging out Rust unchecked (~949 MiB/s).  The remaining gap is attributable
to Rust's `cmovae`/`setae` sequence for the 13/14-bit select vs GCC's
well-predicted branch (see §7).

**Effect on decode:** C decode went from ~528 MiB/s to ~1342 MiB/s (+154%),
now leading Rust unchecked (~1033 MiB/s) by ~30%.  The structural advantage
comes from GCC's drain loop (backward branch + direct memory writes) vs
LLVM's restructuring around Vec capacity checks — even in the unchecked path,
LLVM's drain has slightly more overhead.

---

## 14. Rust safe: `spare_capacity_mut` + `set_len`

**Problem:** `Encoder::encode` and `Decoder::decode` used `Vec::push` for
output, which caused a ~60% throughput penalty vs the unchecked path.

`Vec::push` inlines to:

```rust
if self.len == self.buf.capacity() { self.buf.grow_one(); }
unsafe { ptr.add(self.len).write(val); }
self.len += 1;
```

After `grow_one()` (the slow reallocation path), `ptr` and `cap` may have
changed.  LLVM cannot hoist the capacity check or keep `ptr` in a register
because it cannot prove `grow_one` won't run — even when the Vec was
pre-reserved before the loop.  Result: `ptr`, `cap`, and `len` are all
spilled to the stack, reloaded after each push.

**Fix:** reserve once, then write directly into `spare_capacity_mut()`:

```rust
output.reserve(crate::encode_size_hint(input.len()));
let spare = output.spare_capacity_mut();
// ... hot loop writes spare[n].write(...) ...
unsafe { output.set_len(output.len() + n) };
```

`spare_capacity_mut()` returns a `&mut [MaybeUninit<u8>]` with a fixed
length for the whole call.  LLVM sees a plain slice: `ptr` stays in a
register, no reallocation calls in the hot path, no per-element capacity
check.  The duplicate-writes-per-arm fix (§12.4) was also applied to the
encode path, giving LLVM immediate-count shifts.

The only `unsafe` is `set_len`, which is sound because the loop writes
exactly `n` bytes into `spare[0..n]` before calling it.  The public
`encode`/`decode` API remains safe (no `unsafe fn`).

**Result:**

| | Before (`Vec::push`) | After (`spare_capacity_mut`) |
|---|---|---|
| Rust safe encode | ~494 MiB/s | **~919 MiB/s** (+86%) |
| Rust safe decode | ~748 MiB/s | **~972 MiB/s** (+30%) |

Safe is now ~88–78% of unchecked.  The residual gap is the bounds check on
`spare[n]` (LLVM cannot prove `n < spare.len()` without profiling) and the
`reserve` call overhead amortised over the input.
