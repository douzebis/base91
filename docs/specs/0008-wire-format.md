<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0008 — Wire format (normative)

**Status:** implemented
**App:** base91
**Implemented in:** 2026-09-28

## Purpose

This document is the normative definition of the byte streams that
`base91-rs`, `base91-cli` and `pybase91` produce and accept. It is
independent of any implementation. Code, rustdoc, CLI help and other specs
conform to it, not the other way around.

It is written with two drivers:

1. **Legacy compatibility.** Every decoder recognizes legacy basE91
   streams (Joachim Henke's format) and decodes them exactly as the
   reference implementation does.
2. **SIMD efficiency.** The SIMD format, including its line layout, is
   defined so that both encoding and decoding can run as straight-line
   vector code over whole blocks: no data-dependent control flow, no
   second pass, no moving or compacting of data, in bounded memory
   (no heap needed, streaming included).

Deviations of the current implementation from this document are listed
in *Annex A*. They are not part of the format.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in
RFC 2119.

## 1 — Terminology

- **Stream**: the complete sequence of bytes produced by one encoding, or
  given to one decoding.
- **Legacy format**: Joachim Henke's basE91 format (§4).
- **SIMD format**: the fixed-width format defined in §3.
- **Whitespace**: the bytes SP (0x20), HT (0x09), CR (0x0D), LF (0x0A).
- **Line terminator**: LF, or the two-byte sequence CR LF.
- **Payload**: the characters of a SIMD stream that carry data, i.e.
  everything except the `-` prefix and line terminators.

## 2 — Stream classification (decoders)

A decoder classifies its input once, before decoding:

1. Skip leading whitespace.
2. If no byte remains, the stream is **empty** and decodes to zero bytes.
3. If the first remaining byte is `-` (0x2D), the stream is a **SIMD
   stream** (§3), starting at that byte.
4. Otherwise it is a **legacy stream** (§4). The whole input is decoded as
   legacy, including the skipped whitespace, which the legacy decoder
   ignores anyway.

Rationale. `-` is not in the legacy alphabet and the legacy encoder never
emits it, so rule 3 never misclassifies a stream produced by a legacy
encoder. Skipping whitespace first changes nothing for legacy streams
(whitespace is ignored there) and makes SIMD streams robust to a leading
blank line. A legacy stream that was hand-edited to begin with `-` is
out of scope.

Classification needs no lookahead beyond the first non-whitespace byte.
A streaming decoder keeps a "not yet classified" state across chunks.

## 3 — SIMD format

### 3.1 Alphabet

91 characters: every byte from 0x23 to 0x7E except 0x27 (`'`).

| Index | Character bytes |
|---|---|
| 0–3 | 0x23–0x26 (`#` `$` `%` `&`) |
| 4–90 | 0x28–0x7E (`(` … `~`) |

In index order:

```
#$%&()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\]^_`abcdefghijklmnopqrstuvwxyz{|}~
```

- `char(i) = 0x23 + i + (i ≥ 4)`; `index(c) = c − 0x23 − (c > 0x26)`.
  Both are branch-free and map to one vector compare plus one add/sub per
  byte.
- The alphabet contains no whitespace, no `"` and no `'`. SIMD streams can
  be single-quoted in POSIX shells. (They contain `\`, so they need
  escaping in JSON and C strings.)

### 3.2 Bit packing

Let the input be bytes b₀ … bₙ₋₁ (n ≥ 1).

1. **Bit stream.** Concatenate the input bytes least-significant bit
   first: bit 8i + j of the stream is bit j of bᵢ.
2. **Groups.** Split the bit stream into consecutive 13-bit groups, least
   significant bit first. Group k has the value
   vₖ = Σⱼ bit(13k + j)·2ʲ, for j = 0 … 12, so 0 ≤ vₖ ≤ 8191.
3. **Full group → 2 characters.** Emit `char(vₖ mod 91)`, then
   `char(vₖ div 91)`. Since 8191 div 91 = 90, both indices are valid.
4. **Final partial group.** Let r = 8n mod 13. If r > 0, the last group
   has only r data bits; the missing high bits are 0 (padding). It is
   emitted as:
   - r ≤ 6: **one** character, `char(v)` (here v < 64 < 91);
   - r ≥ 7: **two** characters, as in rule 3.

### 3.3 Blocks and lengths

13 bytes = 104 bits = 8 groups = **16 characters**. This is a **block**.
Block boundaries are bit-aligned, so every block is encoded and decoded
independently of the others. That is the property the SIMD kernels rely
on: 1 block per 128-bit lane (13 → 16), 2 per AVX2 register, 4 per
512-bit register.

**Payload length** for n input bytes:

  P(n) = 16·⌊n / 13⌋ + T[n mod 13]

| n mod 13 | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| T | 0 | 2 | 3 | 4 | 5 | 7 | 8 | 9 | 10 | 12 | 13 | 14 | 15 |

T is injective, so a payload length L determines the decoded length
exactly:

  D(L) = 13·⌊L / 16⌋ + T⁻¹[L mod 16]

A payload with L mod 16 ∈ {1, 6, 11} is **invalid**.

Both lengths are exact, not bounds. An encoder or decoder can compute
the size of its output before writing it, and the position of every
block in it.

### 3.4 Stream layout

- **Empty input** (n = 0) encodes to the **empty stream** (zero bytes, no
  prefix).
- **Unwrapped** (width W = 0): `-` followed by the payload.
- **Wrapped** (width W > 0): `-` LF, then the payload split into lines of
  exactly W characters, **each followed by LF, the last one included**.
  The last line has 1 … W characters. This matches the legacy reference
  CLI, whose wrapped output also ends with a newline, while unwrapped
  output does not.

W MUST be a multiple of 32 (W = 32k, k ≥ 1). Encoders MUST reject any
other nonzero W. Encoders SHOULD default to W = 64, the PEM / OpenSSL line
width, which is also a whole number of 64-character (AVX-512) units.

Encoded length for n ≥ 1:

  E(n, 0) = 1 + P(n)          E(n, W) = 2 + P(n) + ⌈P(n) / W⌉

Why this layout is SIMD-efficient:

- **The header `-` LF puts the payload on a line boundary**, so the first
  block starts at the start of a line.
- **W = 32k means every line holds whole 32-character units** (2 blocks,
  one AVX2 register; 2 SSE4.1 / NEON registers). An encoder writes each
  unit directly at its final position, and every line ends with the same
  terminator, so the layout has no special case. No vector load or store
  ever spans a line end. There is no insertion pass or data movement, and
  encoding is embarrassingly parallel.
- **A decoder knows where every terminator is** once it has read the first
  line. It checks one position per line, runs the kernels on whole units
  in place, and never compacts or copies input.
- **Wider vectors are deferred.** A future 64-character (AVX-512) kernel
  would see units spanning a line end when W is an odd multiple of 32
  (e.g. 32, 96). Handling that (split loads/stores, or falling back to
  32-character units on such lines) is left to the spec that introduces
  that kernel. The format does not change for it.

The canonical encoding of an input is unique for a given W: an encoder's
output is a pure function of (input, W). It MUST NOT depend on the SIMD
level, the CPU, or how a streaming encoder's input was chunked.

### 3.5 What decoders accept

A SIMD stream, after classification (§2), MUST be accepted if and only if
it has the form:

```
"-"  ( payload                                   ; unwrapped
     | TERM  line ( TERM line )*  )              ; wrapped
     [ trailing whitespace ]
```

where:

- `payload` and each `line` consist only of alphabet characters (§3.1);
- `TERM` is LF or CR LF, **the same one throughout the stream**;
- in the wrapped form, every line except the last has the same length W,
  which is a positive multiple of 32. The decoder learns W from the first
  line. The last line has 1 … W characters. (The first line is the last
  line when there is only one line; any length is then accepted, subject
  to the payload-length rule.) A single line of length 0 is allowed only
  if the payload is empty (`-` followed by TERM);
- the total payload length L satisfies L mod 16 ∉ {1, 6, 11};
- every character pair of the payload has a value of at most 8191 (see
  *invalid value* below);
- trailing whitespace is ignored. This covers the canonical final LF
  (§3.4), its absence (streams written by 0.2.2 and earlier), and a
  newline added by an editor.

`-` alone (and `-` followed only by whitespace) is a valid SIMD stream
with an empty payload, and decodes to zero bytes. Encoders never produce
it (§3.4), but accepting it costs nothing and cannot be ambiguous.

Anything else is an **error**, and the decoder MUST report it rather than
return data:

| Error | Examples |
|---|---|
| invalid character | a byte outside the alphabet in the payload, e.g. `"`, a space, or a CR that is not part of the stream's CR LF terminator |
| invalid layout | lines of unequal length, W not a multiple of 32, mixed LF / CR LF, a terminator in the middle of a line |
| invalid length | L mod 16 ∈ {1, 6, 11} |
| invalid value | a character pair whose value `index(lo) + 91·index(hi)` exceeds 8191 |

About **invalid value**: two characters can express 0 … 8280 (90 + 91·90),
but a 13-bit group only holds 0 … 8191, so no encoder produces a pair worth
8192 … 8280. Such a pair is structural corruption and MUST be reported.
Silently truncating it to 13 bits, or letting its 14th bit spill into the
next group, would both return wrong data. The check costs one comparison
per pair (one vector compare per block).

**Which error is reported.** A stream can contain several errors. The
decoder reports the **first** one, reading left to right: the error at the
earliest point where the stream stops being the beginning of any valid
stream. This is the only rule compatible with chunking invariance (§3.6):
a streaming decoder can report each error as soon as it sees it, and never
has to hold one back in case an earlier-reported one turns up later.
Precisely, reading the payload after `-` (and after the header TERM of a
wrapped stream):

1. **Whitespace is judged when non-whitespace follows it.** A run of
   whitespace bytes might be trailing whitespace, which is valid, so it is
   only an error once a non-whitespace byte follows it. In a wrapped
   stream, a run that consists of exactly one TERM, right after a
   non-empty line, is a line break. Any other run is an error at its first
   byte that is not part of such a line break: *invalid layout* if that
   byte starts a terminator (LF, or CR followed by LF), *invalid character*
   otherwise. So `-AB CD` fails at the space, `-ABC` LF `DEF` fails
   (invalid layout) at the LF, and a wrapped stream with an empty line
   followed by more lines fails (invalid layout) at the empty line's
   terminator.
2. **Line lengths (wrapped) are checked where the next line starts**, since
   until then a line might be the last one. When the first byte of a new
   line arrives: if the previous line was the first, W is its length, and W
   must be a positive multiple of 32; otherwise the previous line must be W
   long. A line may never grow beyond W: the byte where a terminator was
   expected is an error. These are *invalid layout* errors at the byte
   that starts the new line or overflows the line.
3. **Structure before content.** When a byte breaks rule 2 and is also not
   an alphabet character, *invalid layout* is reported.
4. **A byte outside the alphabet and outside whitespace** is an *invalid
   character* at that byte.
5. **A pair worth more than 8191** is an *invalid value*, reported at the
   pair's first character, as soon as its second character is read.
6. **The payload length** (*invalid length*) is checked last, at the end of
   the stream, and has no position.

Reporting the offset is still optional, but a decoder that reports one
MUST report this one, and a decoder that reports no offset MUST still
report this error's class.

Decoders MAY also reject nonzero padding bits (§3.2 rule 4), but they are
not required to check them.

This strictness is deliberate (driver 2). Because every accepted stream
has a regular layout, the fast path is the *only* path. Accepting
arbitrary whitespace would force a slow path that compacts data, for
input that no conforming encoder produces.

**Cost of error detection.** Silent wrong output is not an acceptable
price for speed. On valid input, detection SHOULD cost at most 30 % of
decode throughput compared with the same decoder without checks. (The
reference kernels measure +17 to +22 % on a performance core and +20 to
+27 % on an efficiency core. The first version of this section asked for
3 %, which exact checks cannot reach with this alphabet: they take four
vector operations per 16 characters, against about eleven for decoding
itself; see spec 0011, stage 3.) Implementations should get there as
follows:

- **Characters:** the SIMD kernels already compute every character's
  index. Keep running maximums of the indices (with `'` forced out of
  range) and of the pair values, and compare them once per batch rather
  than once per block.
- **Layout:** one byte comparison per line.
- **Length:** a single check at the end.
- **Error position:** reporting the exact offset is optional. A decoder
  may locate it after the fact, by re-scanning the failing region on a
  slow path, so that valid input never pays for it.

### 3.7 Integrity

The format has **no checksum**. Errors (§3.5) catch **structural** damage:
characters outside the alphabet, changed line lengths, stray CR or
whitespace. That is what mail software, editors and line-ending
conversions typically cause, for example re-wrapped lines or
quoted-printable soft breaks (`=` CR LF). A substitution of one alphabet
character by another keeps the structure intact and is **not** detected.
Applications that need integrity must add their own (hash, MAC, or a
signed container).

### 3.6 Streaming

Encoding and decoding MUST be **chunking-invariant**: splitting the input
into any sequence of chunks, fed to a streaming encoder or decoder, gives
exactly the one-shot result (for a decoder: the same bytes, or an error at
the same point).

Both directions need only bounded state, and no heap:

- **Encoder:** up to 12 pending bits, the current column, and whether the
  prefix has been written. The prefix is written **lazily**, with the
  first non-empty input, so that an empty stream stays empty (§3.4).
- **Decoder:** classification state, up to one pending character plus
  7 pending bits, the current column, W once known, and the terminator
  style once known.

## 4 — Legacy format

The legacy format is defined by its reference implementation: Joachim
Henke's basE91 **0.6.0** (`base91.c`, `base91.h`), as distributed
upstream. The copy in `src/` has been optimized (`__restrict__`, hoisted
locals); upstream is the reference, not the copy.

- **Encoding.** Output MUST be byte-identical to the reference
  `basE91_encode` + `basE91_encode_end` for the same input.
- **Decoding.** For any input classified as legacy (§2), output MUST be
  byte-identical to the reference `basE91_decode` + `basE91_decode_end`.
  This includes ignoring every byte outside the legacy alphabet (the
  reference never reports errors).
- **Empty.** Empty input encodes to the empty stream, and vice versa (as
  in the reference).
- **Line wrapping** of legacy output is presentation, not format: the
  reference `b91enc` wraps at 76 columns, and decoders ignore line breaks.
  Our encoders MAY wrap legacy output at any width.

## 5 — Summary of guarantees

| Property | Legacy | SIMD |
|---|---|---|
| `decode(encode(x)) = x` | yes | yes, for every W |
| Encoded length | ≤ reference bound | exact: E(n, W) |
| Decoded length | ≤ bound | exact: D(L) |
| Output independent of CPU / SIMD level / chunking | yes | yes |
| Invalid input | ignored, as in the reference | error |
| Structural damage (re-wrapping, stray CR / spaces) | ignored | error |
| Character substitution within the alphabet | not detected | not detected (§3.7) |
| Empty ↔ empty | yes | yes (`-` alone also decodes to empty) |

## Decisions recorded

- **Strict SIMD decoding** (§3.5). A lenient mode would cost performance
  and is not part of the format.
- **Errors, not silent output**, for invalid SIMD input, with a cost budget
  (§3.5).
- **`-` alone decodes to empty** (§3.5).
- **Wrapped output ends with LF**, as in the legacy reference (§3.4).
- **W is a multiple of 32**, which is optimal for today's widest kernel
  (AVX2). 64-character units are deferred until an AVX-512 kernel exists
  (§3.4).

## Open questions

None.

---

## Annex A — Inconsistencies with the current implementation (informative)

State of `main` at `f994029`, i.e. release 0.2.2, checked while writing
this spec. Where a fix is already specified, the spec is named.

**Status (2026-09-28): every inconsistency is resolved.** A3–A8 are fixed
in 0.2.3 (specs 0005, 0006); A2, A4, A9–A14 and A18 in 0.3.0 (spec 0010);
A1 by marking spec 0002's wire-format sections as superseded. A15–A17 are
kept as tests. The table below records the 0.2.2 behavior.

| # | Topic | Current behavior | This spec | Fix |
|---|---|---|---|---|
| A1 | Alphabet documentation | Spec 0002 says the alphabet omits `\` (0x5C). The code omits `'` (0x27): output contains `\` and never `'`. | Omits `'` (§3.1), matching the code, so published streams stay valid | Mark spec 0002's wire-format sections as superseded by this spec |
| A2 | Allowed W | Spec 0002 says 16 (Goals) and 32 (body); rustdoc and CLI say 32 (the CLI enforces it); the library does not check W at all | Multiple of 32, enforced by encoders (library and CLI) | Spec 0010 (`Error::InvalidWidth`) |
| A3 | **Wrapped encoding when P(n) is a multiple of W** | **Corrupt output**: `-X` LF `X…` (the first payload character is duplicated before the header LF). Decoding gives one extra byte. Hits `base91 --simd` (default W = 64) for **every input whose size is a multiple of 52 bytes**; 478 of 15 005 (n, W) cases tested. Affected files can be repaired by deleting the character between `-` and the first LF. | §3.4 layout | Spec 0006, bug 3 |
| A3b | Final LF of wrapped output | Not written | Written (§3.4) | Spec 0006 §4 (same function as A3) |
| A4 | Wrapped encoder implementation | Encodes, then inserts newlines in a second pass that shifts the whole output (`insert_wrap_newlines`) | Blocks written in place (§3.4) | Spec 0010 (in-place layout) |
| A5 | Empty input, streaming encoder | `simd::Encoder` writes `-` on the first `encode` call, even with empty input, so an empty stream becomes `-` (one-shot gives empty) | Prefix written lazily | Spec 0006, bug 4 |
| A6 | Empty input, `simd::encode_unchecked` | Writes `-` | Empty | Spec 0005 |
| A7 | Chunking invariance, decoder | Violated: `simd::Decoder` loses state at chunk boundaries; CLI `-d` corrupts SIMD streams ≥ 16 kB | §3.6 | Spec 0006, bug 1 |
| A8 | Decoder bounded state | `ScalarDecoder` overflows its reservation with a pending character (heap corruption) | §3.6 | Spec 0006, bug 2 |
| A9 | CR LF | `\r` decoded as data: **wrong output, no error** | Accepted (§3.5) | Spec 0010 |
| A10 | Whitespace before `-` | Classified as legacy: **wrong output, no error** | Skipped (§2) | Spec 0010 |
| A11 | Invalid characters | Decoded as garbage: **wrong output, no error** (with overflow checks on, as in debug builds, the scalar decoder instead panics on an arithmetic overflow); `Decoder::decode` returns `true`; `simd::decode` never returns `None` | Error | Spec 0010 |
| A12 | Invalid payload length (L mod 16 ∈ {1, 6, 11}) | Decoded silently (a lone character is flushed as a byte) | Error | Spec 0010 |
| A13 | Irregular layouts | One-shot decode strips every LF anywhere (heap copy); anything else becomes garbage | Error (§3.5) | Spec 0010 |
| A13b | Pair value above 8191 | The scalar decoder adds the value to its bit queue as is, so the 14th bit spills into the next group: **wrong output, no error** | Error, invalid value (§3.5) | Spec 0010 (kernels check it, §5) |
| A14 | Sizes | `encode_size_hint` / `decode_size_hint` are upper bounds | Exact E and D exist (§3.3); APIs may expose them | Spec 0010 (`simd::encoded_len`, slice API) |
| A15 | Legacy conformance | Encoder and decoder tested against the reference test vectors (`tests/reference_vectors.rs`, sha256 of the C `test.sh` fixtures). Decoding of garbage-laden input vs the reference is not explicitly tested. | §4 | Spec 0009 (done: matrix, invalid-input and property tests, `fuzz_legacy`, CLI byte-for-byte comparison with upstream's CLI) |
| A16 | Trailing LF on input | Accepted (stripped) | Accepted | — |
| A17 | Level independence | Verified: identical output at `Scalar`, `Simd128`, `Simd256` for n = 0 … 3000 and W ∈ {0, 16, 32, 48, 64} (x86_64) | §3.4 | Keep as a test |
| A18 | Spec 0002 API section | `simd::decode` "returns `None` if no `-`", `decode_unchecked` "returns `usize::MAX`", `Encoder::new(level, wrap)`: none of these match the code (Henke fallback; no wrap in the streaming encoder) | Out of scope here (API spec) | Spec 0010 (defines the 0.3.0 API) |

Items still marked "New" have no spec yet. The decoder acceptance rules of
§3.5 (A9–A13b), the in-place layout (A4), width validation (A2), exact
sizes (A14) and the API (A18) are all covered by spec 0010, the driver-layer
rewrite (0.3.0).
