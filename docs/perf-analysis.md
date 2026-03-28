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

## 8. Benchmark results (criterion, rustc 1.86.0, gcc 14.3.0, x86-64)

1 MiB random input.  Intel Core Ultra 7 165U, AC power, turbo enabled.

| Path | Rust unchecked | C reference | Ratio |
|---|---|---|---|
| encode | ~1010 MiB/s | ~645 MiB/s | **Rust +56%** |
| decode | ~1020 MiB/s | ~528 MiB/s | **Rust +93%** |

### Encode performance history

The encode path required two targeted fixes to beat C:

1. **`ENCTAB.get_unchecked()`** — LLVM inserted `jae` panic branches for
   each `ENCTAB[r]` and `ENCTAB[q]` index because it could not statically
   prove `r,q < 91`.  Even though they were never taken, they consumed
   front-end bandwidth.  Replacing with `get_unchecked` removed them.
   Result: parity with C (~634 vs ~641 MiB/s).

2. **Duplicated writes per arm** — with the two 13/14-bit paths sharing a
   single write block at the bottom, LLVM merged them into a
   `cmovae`/`setae` + variable-count `shr cl` sequence.  The variable-count
   shift has a 3-cycle latency and a data dependency through the flag
   register, costing more than C's simple well-predicted branch.
   Duplicating the writes into each arm breaks the merge, giving LLVM two
   independent paths with immediate-count shifts.
   Result: Rust +56% over C.

### Decode performance history

Decode was faster than C from the first implementation, because:
- register-hoisted `queue`/`nbits` eliminate the memory round-trips that
  plague GCC due to the aliasing conservatism;
- the drain loop is unrolled to two write sites with no backward branch;
- LLVM generates `cmp+adc` matching GCC's branchless 13/14-bit select.
