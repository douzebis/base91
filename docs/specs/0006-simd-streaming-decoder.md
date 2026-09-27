<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0006 — Fix SIMD streaming and wrapping corruption (and a `ScalarDecoder` heap overflow)

**Status:** draft
**App:** base91
**Implemented in:** —

## Problem

Four bugs in the SIMD variant, all in 0.2.2 (the SIMD variant first
shipped in 0.2.2). Bugs 1 and 2 were found while writing spec 0007; bugs 3
and 4 while writing spec 0008, which defines the target format these
fixes conform to.

### Bug 1 — `simd::Decoder` loses state at chunk boundaries (silent data corruption)

`simd::Decoder::decode` (`simd/mod.rs:521`) passes every chunk to the
**one-shot** helper `decode_into` (`simd/mod.rs:306`). For each chunk, that
helper:

- decodes the full SIMD blocks;
- creates a **fresh** `ScalarDecoder` for the remaining characters;
- **flushes** it (`sc.finish(output)`) as if the stream ended there.

Any chunk whose payload is not a multiple of 16 characters therefore gets
its tail decoded as an end of stream. The next chunk then starts in the
middle of a block with an empty state.

`Decoder` does have a persistent `self.scalar`, but it is never fed:
`decode` never uses it, and `finish` (`simd/mod.rs:545`) flushes an empty
decoder.

Observed on 0.2.2 (x86_64, AVX2):

| Path | Input | Result |
|---|---|---|
| `simd::Decoder`, chunks of 1, 7, 16, 17, 4096, 8192 bytes | 20 kB SIMD stream, `wrap` 0 and 64 | **wrong output** at every chunk size |
| `io::DecoderReader` (reads 8 KiB at a time, `io.rs:13`) | same | **wrong output** |
| CLI, `base91 --simd < f \| base91 -d` (pipe or file input) | random files up to 12 kB (≤ 15 002 encoded bytes) | OK |
| same | random files of 16 kB and above (≥ 20 002 encoded bytes; tried up to 100 kB) | **corrupted** at every size |
| one-shot `simd::decode` | 20 kB SIMD stream | OK |
| Henke streams, every path above | all sizes above | OK |

No error is reported. The first chunk always contains the `-` prefix, so
even chunks of 16 bytes are misaligned.

Existing tests missed it: the `io` tests (`reader_chunked_matches_one_shot`
and others, `io.rs:206–250`) only use Henke streams, and the `simd` tests
only use the one-shot API.

### Bug 2 — `ScalarDecoder::decode` writes past its reservation (memory corruption from safe code)

`simd::scalar::ScalarDecoder` is public, and its `decode(&mut self, &[u8],
&mut Vec<u8>)` is a safe function. It reserves `input.len()` bytes
(`simd/scalar.rs:221`), then writes through `spare.get_unchecked_mut(n)`
with no bounds check.

Without a pending character, k input characters give at most k output
bytes, so the reservation is enough. **With** a pending character from the
previous call (`self.first != u32::MAX`), k characters can give **k + 1**
bytes: the pending character completes a pair, and each pair can emit 2
bytes. The last write lands 1 byte past the reserved space.

Observed on 0.2.2, feeding a 20 kB SIMD payload to `ScalarDecoder` in
chunks:

| Chunk size | Result |
|---|---|
| 1 | aborts: `malloc(): corrupted top size` / `malloc_consolidate(): invalid chunk size` |
| 2, 3, 4, 7, 8, 13, 16, 17, 26, 4096, 4097 | correct output, no visible crash |

The larger odd sizes still overflow, but the extra byte usually lands in
capacity the `Vec` over-allocated anyway. It is undefined behavior either
way. Apart from the overflow, the carry logic is correct: the decoded bytes
match the one-shot result.

Reachability today: only by using `ScalarDecoder` directly. `simd::Decoder`
and `decode_into` always use a fresh `ScalarDecoder` (no carry). But the
fix for bug 1 feeds a persistent `ScalarDecoder` across chunks, which would
make bug 2 reachable through `simd::Decoder`, `io::DecoderReader` and the
CLI. **The two must be fixed together.**

### Bug 3 — Wrapped encoding corrupts output when the payload fills its last line

`insert_wrap_newlines` (`simd/mod.rs:369`) computes
`newline_count = 1 + payload_len / wrap`. When `payload_len` is a multiple
of `wrap`, that counts one newline too many for the layout the code
intends: header LF plus separators, no final LF, i.e.
`⌈payload_len / wrap⌉`. The right-to-left shift then runs
one position off, and the output becomes `-X` LF `X…`: the first payload
character is duplicated **before** the header newline.

- The encoded stream itself is corrupt. Decoding it (with any decoder)
  gives one extra byte.
- Observed on 0.2.2: for `n = 0 … 3000` and `wrap ∈ {16, 32, 48, 64}`,
  exactly the cases where the payload length is a multiple of `wrap` fail
  (478 of 15 005 cases tested, `wrap = 0` included). All levels behave the
  same.
- **CLI:** `base91 --simd` defaults to `wrap = 64`, and the payload length
  is a multiple of 64 exactly when the input size is a multiple of 52
  bytes. Checked: 52, 104, 520, 1040 and 5200 bytes are corrupt (decoded
  size +1); 51 and 53 are fine.
- **Repair:** the damage is deterministic. Deleting the single character
  between `-` and the first LF restores a correct stream. The release
  notes should say so.

### Bug 4 — The streaming encoder turns an empty stream into `-`

`simd::Encoder::encode` writes the `-` prefix on its first call
(`simd/mod.rs:431`), even when that call's input is empty. `encode(b"")`
followed by `finish` gives `-`, while one-shot `simd::encode(b"")` gives
the empty stream. That breaks chunking invariance (spec 0008 §3.6). It is
harmless for decoding (`-` decodes to empty), but the two encoders must
agree.

### Not affected (checked)

- `simd::Encoder` (streaming, no wrapping): for non-empty input, chunks of
  1, 7, 13, 16, 17, 26 and 4096 bytes at every level give the same output
  as `simd::encode` (the empty case is bug 4).
- Output independence from the SIMD level: `Scalar`, `Simd128` and
  `Simd256` give identical output for n = 0 … 3000 and
  `wrap ∈ {0, 16, 32, 48, 64}` (bug 3 included: all levels are equally
  wrong).
- `ScalarEncoder`: its reservation `⌈16k/13⌉ + 2` covers the pending bits
  (at most 12): 2·⌊(8k+12)/13⌋ ≤ (16k+24)/13 < (16k+26)/13.
- The CLI encode path (`base91 --simd`) uses the one-shot `simd::encode`
  (`base91-cli/src/main.rs:249`).
- `pybase91`: its functions and classes use the Henke codec only
  (`crate::encode` / `decode` / `Encoder` / `Decoder`, `python.rs`), never
  `simd`.

### Exposure

- `base91-rs` 0.2.2 and `base91-cli` 0.2.2 on crates.io. The CLI is the
  main exposure, in both directions:
  - **decode** (bug 1): `base91 -d` silently produces wrong data for SIMD
    streams of about 16 kB of input and above (observed threshold above).
    The exact threshold depends on the CLI's and `DecoderReader`'s
    buffering and does not matter for the fix;
  - **encode** (bug 3): `base91 --simd` writes a corrupt file for every
    input whose size is a multiple of 52 bytes (default `wrap = 64`), and,
    with `-w W`, whenever the payload length is a multiple of W. Such files
    are on users' disks. They can be repaired (see bug 3).
- `pybase91` is not affected.

## Goals

- `simd::Decoder` gives exactly the same output as one-shot `simd::decode`
  for **any** chunking of **any** valid stream (with or without `\n`
  wrapping, every `SimdLevel`).
- `io::DecoderReader` and the CLI round-trip SIMD streams of any size.
- `ScalarDecoder::decode` never writes outside its reservation, for any
  chunking.
- Wrapped encoding produces exactly the layout of spec 0008 §3.4, with
  length E(n, W), for every n and W.
- An empty input gives the empty stream, whether encoded one-shot or
  streaming.
- The one-shot decoder's performance is unchanged (this is a patch
  release).
- Tests that would have caught all four bugs, and that keep the streaming
  and one-shot paths equivalent from now on.

## Non-goals

- A slice-based or allocation-free decoder (spec 0007 builds on this one).
- The decoder acceptance rules of spec 0008 §3.5 (errors on invalid
  characters, CR LF, irregular layouts, invalid lengths) and the in-place
  fast path they enable. They change behavior and belong in a separate
  "strict SIMD decoder" spec. Behavior on invalid input stays as in 0.2.2.
- Writing wrapped blocks in place instead of the insertion pass (spec
  0007).
- Validating `wrap` in the library (spec 0008, annex item A2).

## Specification

### 1 — `ScalarDecoder`: correct reservation and alignment query

- `decode` reserves `input.len() + 1`.
  Bound: each completed pair adds 13 bits to a queue that holds at most 7
  bits after draining, so it emits at most 2 bytes. k input characters plus
  one pending character complete at most p = ⌊(k+1)/2⌋ pairs, so they emit
  at most 2p ≤ k + 1 bytes. (Without a pending character, 2·⌊k/2⌋ ≤ k,
  which is why the bug needs a carry.)
- The writes stay unchecked in release builds (hot loop). Add
  `debug_assert!(n < spare.len())` before each `get_unchecked_mut(n)` in
  the `emit_pair`, `emit_pair_single` and `emit_pair_double` macros, so the
  tests turn any future overflow into a panic.
- Add `pub fn is_aligned(&self) -> bool`, true iff no character is pending
  and no bits are queued (`self.first == u32::MAX && self.nbits == 0`).
  This mirrors `ScalarEncoder::is_aligned` (`simd/scalar.rs:58`). Pairs
  emit 13 bits each, so the queue is empty only after a multiple of 8
  pairs = 16 characters. That is exactly a SIMD block boundary.

### 2 — `simd::Decoder`: persistent state, drain-then-blocks-then-tail

Rewrite the SIMD branch of `Decoder::decode` to mirror what
`simd::Encoder::encode` already does correctly (`simd/mod.rs:430–481`).
For each `\n`-separated segment of the chunk (the newlines themselves are
dropped):

1. **Drain to alignment.** While `!self.scalar.is_aligned()` and the
   segment has characters left, feed one character at a time to
   `self.scalar`. At most 15 characters.
2. **Full blocks.** Pass the largest prefix that is a whole number of
   blocks to the kernels, using the same level dispatch as `decode_into`
   today (AVX2 on multiples of 32 characters, then SSE4.1 / NEON on
   multiples of 16). The scalar state is aligned, so the kernels start at a
   block boundary. If a kernel returns `false`, `decode` returns `false`,
   as today.
3. **Tail.** Feed the remaining characters (fewer than one block) to
   `self.scalar`, **without** calling `finish`.

`Decoder::finish` keeps calling `self.scalar.finish(output)`, which now
flushes real state.

Format detection (`-` prefix on the first non-empty chunk, Henke
otherwise) does not change. The first chunk's payload after `-` goes
through the same path as later chunks.

Splitting on `\n` happens per chunk, so a newline at a chunk boundary needs
no special handling: each chunk is split independently, and the decoder
state carries across.

### 3 — One-shot `decode_into`: unchanged

The one-shot path keeps its current structure: strip the newlines, then
run the kernels over one contiguous buffer. Routing it through the §2
segment loop was considered and rejected for this patch release. With short
lines (e.g. `wrap = 32`), the per-segment dispatch overhead would slow
down the one-shot path. The proper fast path (kernels running in place
over a known, regular layout, with no copy) depends on spec 0008's strict
acceptance rules, and belongs in the strict-decoder spec.

To keep the two paths from drifting apart, §6's equivalence tests compare
the streaming decoder with the one-shot decoder on every case.

### 4 — Wrapped encoding: correct newline placement (bug 3)

The target is spec 0008 §3.4: `-` LF, then the payload in lines of W
characters, **every line followed by LF, the last one included**. For a
payload of P ≥ 1 characters the output has ⌈P / W⌉ lines, so it needs
**1 + ⌈P / W⌉ newlines** (header plus one per line). Total length
E(n, W) = 2 + P + ⌈P / W⌉.

`insert_wrap_newlines` is rewritten for that layout. The final LF is a
deliberate output change in this patch release. It is compatible: every
0.2.x decoder ignores a trailing newline (spec 0008 annex, A16).

The fix stays within the current two-pass design. Writing units in place
comes with spec 0007.

Width validation (multiple of 32) is not added to the library here (spec
0008 annex, A2). The tests below only use valid widths.

### 5 — Streaming encoder: lazy prefix (bug 4)

`simd::Encoder::encode` writes `-` only when it first receives a
**non-empty** input. An `Encoder` that only ever saw empty input finishes
with an empty output.

(`simd::Encoder` has no wrapping support today; that is unchanged here.)

### 6 — Tests

- **`ScalarDecoder` chunking** (unit, `simd/scalar.rs`): payload lengths
  `0..=200` and 20 000; chunk sizes 1 to 40 and random sizes. Output
  equals the one-shot result. The §1 `debug_assert!`s run in test builds,
  so an overflow panics. A regression test for chunk size 1 (the observed
  crash).
- **`simd::Decoder` chunking** (integration, new
  `tests/simd_streaming.rs`):
  - levels `Scalar` / `Simd128` / `Simd256`;
  - `wrap` ∈ {0, 32, 64};
  - input lengths `0..=100`, 1000, 20 000;
  - chunk sizes 1, 2, 3, 7, 13, 15, 16, 17, 31, 32, 33, 4096, 8191, 8192,
    and random splits.

  Output equals `simd::decode` and the original input.
- **Wrapped encoding** (integration, same file): for n = 0 … 3000,
  `wrap ∈ {0, 32, 64, 96}` and every level:
  - the length is exactly E(n, W) (spec 0008 §3.4);
  - the layout is exact: `-` LF, full lines of W characters, a last line of
    1 … W characters, every line followed by LF;
  - the output round-trips.

  Named regression tests for n = 26 with W = 32, and n = 52 with W = 64
  (the CLI default). Both are corrupt on 0.2.2.
- **Streaming encoder**: `encode(b"")` + `finish` gives the empty stream;
  empty chunks interleaved with non-empty ones do not change the output.
- **`io::DecoderReader`**: add SIMD-stream variants of
  `reader_matches_one_shot` and `reader_chunked_matches_one_shot`
  (`io.rs`), with streams larger than `BUF_SIZE` (8 KiB).
- **CLI** (`base91-cli/tests/cli.rs`): `--simd` encode then `-d` decode
  round-trip, with and without `-w`, for inputs of 0, 1, 1000, 12 000,
  16 000, 100 000 and 1 000 000 bytes. 12 kB and 16 kB bracket the
  observed threshold: on 0.2.2, 12 kB passes and 16 kB fails. Also 51, 52,
  53 and 520 bytes with the default wrap (bug 3: 52 and 520 fail on 0.2.2).
- The existing tests must still pass.

### 7 — Performance

`cargo bench --bench throughput`, on the same machine as the 0.2.2
baseline:

- one-shot SIMD decode and encode (wrapped and unwrapped): within noise
  (≤ 2 %) of 0.2.2;
- streaming SIMD decode: measured and recorded in the CHANGELOG, with no
  target. It was wrong before, so there is no meaningful baseline, and its
  fast path comes with the strict-decoder spec.

### 8 — Documentation and release

- `CHANGELOG.md` for 0.2.3:
  - `Fixed`, stating each impact plainly:
    - bug 1: "`base91 -d` and `io::DecoderReader` could silently corrupt
      SIMD streams of about 16 kB and more";
    - bug 3: "`base91 --simd` wrote corrupt output for inputs whose size is
      a multiple of 52 bytes (default wrap), or generally whenever the
      payload filled its last line", with the repair recipe (delete the
      character between `-` and the first newline);
    - bug 4: the empty streaming case.
  - `Security`: bug 2, next to spec 0005's entry.
- Ship in 0.2.3 with specs 0004 and 0005 (see spec 0004 §8).
- Disclosure: covered by the same advisory as spec 0005 (bug 2 is also
  memory corruption in 0.2.2). Bugs 1 and 3 corrupt data, so the advisory
  and the release notes must mention them too. Users who encoded or
  decoded SIMD streams with 0.2.2 should re-check their outputs; encoded
  files hit by bug 3 can be repaired.

## Open questions

1. **Should `simd::scalar` stay public?** It is `pub mod scalar`, which
   exposes `ScalarEncoder` / `ScalarDecoder`, and that is how bug 2 is
   reachable today. If it only exists for benchmarks and tests, making it
   `pub(crate)` (or `#[doc(hidden)]`) shrinks the public surface. That
   would be a breaking change, so it belongs in 0.3.0 (with spec 0007), not
   in 0.2.3.

## Verification

```bash
cd rust
cargo test -p base91-rs                     # pass, including the new tests
cargo test -p base91-cli                    # pass, including SIMD round-trips
cargo bench -p base91-rs --bench throughput # §7 criteria
head -c 100000 /dev/urandom > /tmp/f && \
  cargo run -q -p base91-cli -- --simd < /tmp/f | \
  cargo run -q -p base91-cli -- -d | cmp - /tmp/f      # no output
nix-build ../default.nix -A rust-tests      # pass
```

Before the fix, the new tests must fail (bug 1) and the chunk-size-1 test
must panic or abort (bug 2). Check this once against the 0.2.2 code.

`reuse lint` must pass.
