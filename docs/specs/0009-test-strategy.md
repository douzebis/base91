<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0009 — Test strategy: oracle, differential, property, fuzzing, cross-arch, performance

**Status:** draft
**App:** base91
**Implemented in:** —

## Problem

Reading the code while writing specs 0004–0008 turned up five bugs in
0.2.2, three of which corrupt data and two memory. The first run of the §2
matrix and invalid-input tests found two more, in code shared by both
formats:

| Bug | Kind | Spec |
|---|---|---|
| `simd::*_unchecked` reallocate the caller's buffer | memory corruption | 0005 |
| `simd::Decoder` loses state at chunk boundaries (CLI `-d` ≥ 16 kB) | data corruption | 0006, bug 1 |
| `ScalarDecoder` writes past its reservation | memory corruption (safe API) | 0006, bug 2 |
| wrapped SIMD encoding when the payload fills its last line (CLI, sizes multiple of 52) | data corruption | 0006, bug 3 |
| streaming encoder turns empty into `-` | inconsistency | 0006, bug 4 |
| legacy `Decoder` panics on small chunks (also `pybase91.Decoder`, 0.2.0–0.2.2) | crash | 0006, bug 5 |
| `io::DecoderReader` returns EOF early when a read decodes to nothing (CLI `-d` on pipes, 0.2.0–0.2.2) | data loss (truncation) | 0006, bug 6 |

Plus the silent wrong output on invalid input (spec 0008, annex A9–A13b),
and a `no_std` build that never compiled (spec 0004).

None of this was caught, because the existing tests:

- mostly **compare the code with itself**: round-trips through the same
  implementation, and SIMD levels against each other. Consistent errors
  pass. All three levels are equally wrong on bug 3;
- test **only the one-shot SIMD API**. The streaming decoder, the
  `unchecked` functions and `ScalarDecoder` chunking had no tests;
- have **no wrapped SIMD round-trip** at the sizes that fail;
- test `io` adapters **only with legacy streams**;
- **never execute the aarch64 (NEON) kernels**. CI does not even compile
  them (spec 0004 §7 adds compilation only);
- never feed **invalid or unusual input** (CR LF, leading whitespace,
  impossible lengths) to the SIMD decoder.

## Goals

- Any deviation from the normative format (spec 0008) is caught, on every
  public API, SIMD level, line width, chunking and architecture.
- Memory-safety bugs in the `unsafe` code are caught by construction
  (canaries, debug assertions) and by fuzzing, not by luck.
- Every bug in the table above has a named regression test that fails on
  0.2.2.
- A performance sanity test checks that the SIMD levels keep their
  expected relative speed.
- Everything runs in CI from the pinned nixpkgs (no rustup, no new Nix
  input), in keeping with spec 0004 §7.

## Non-goals

- Fixing the bugs (specs 0005 and 0006 do that; this spec provides the
  tests they must pass).
- Absolute performance targets (GB/s). Those belong to `criterion`
  benchmarks, which are not pass/fail.
- Tests for APIs that do not exist yet (the slice API and the
  strict decoder, spec 0010). They join the same matrix when they land.

## Specification

### 1 — Independent oracles

Both oracles live in **`rust/base91-testkit`**, an unpublished workspace
crate used as a dev-dependency by `base91-rs` and `base91-cli`. A
library crate avoids the `dead_code` warnings that shared modules under
`tests/common/` would raise in every test binary not using all of them.

**SIMD format oracle** (`base91_testkit::oracle`). A deliberately simple,
bit-by-bit model of spec 0008, written from the spec text only and sharing
**no code** with the library:

- `encode(input, W) -> Vec<u8>`: §3.2 rules one bit at a time, then §3.4
  layout;
- `classify(stream)` and `decode(stream) -> Result<Vec<u8>, DecodeError>`:
  §2 classification, §3.5 acceptance with its error classes, §3.2
  unpacking;
- `payload_len(n)`, `encoded_len(n, W)`, `decoded_len(L)`: the §3.3 / §3.4
  formulas;
- self-tests (`base91-testkit/tests/oracle_selftest.rs`): hand-computed
  vectors, written down with their derivation (bit order, both tail
  forms, wrapped layouts, padding bits, every error class).

Speed is irrelevant; obviousness is the requirement. Code review of the
oracle is against spec 0008, line by line.

**Legacy oracle** (`base91_testkit::reference`, feature `reference`). The
**pristine upstream basE91 0.6.0** C code (`base91.c`, `base91.h`, plus
`cli.c` for the reference CLI of §9), not the optimized copy in `src/`.
Vendored under `rust/base91-testkit/reference/basE91-0.6.0/` with its
provenance (see *Decisions recorded*), BSD-3-Clause, declared in
`REUSE.toml` as imported files. The testkit's `build.rs` compiles it (see
*Decisions recorded*, item 2), and `reference_selftest.rs` checks the
vendored files' hashes and the upstream fixtures.

### 2 — Differential matrix (every API against the oracle)

A generator enumerates, for each case:

| Axis | Values |
|---|---|
| API | one-shot `encode`/`decode`; streaming `Encoder`/`Decoder`; `*_unchecked`; `io` adapters; later the slice API and strict decoder |
| format | legacy, SIMD |
| level | `Scalar`, `Simd128`, `Simd256` (levels the CPU lacks are skipped and reported, not failed) |
| W | 0, 32, 64, 96 |
| chunking | one-shot; fixed chunks 1, 2, 3, 7, 13, 15, 16, 17, 31, 32, 33, 4096, 8191, 8192; random splits (seeded) |
| input | lengths 0 … 3000, plus 20 000 and 1 MB; contents: random (seeded), all-0x00, all-0xFF, counting, bit patterns that maximize the 13-bit group values |

Checks per case:

- the encoded bytes equal `oracle_encode` exactly, and the length equals
  `oracle_len_encoded`;
- decoding gives back the input, and equals `oracle_decode`;
- legacy: encoded and decoded bytes equal the C reference;
- `unchecked` and (later) slice APIs: output buffers are sized **exactly**
  by the documented bound and surrounded by 64 canary bytes (0xA5) on each
  side, which must be untouched afterwards.

The full matrix is large, so it runs with a seed-rotating subset in the
normal test run, and in full in a dedicated CI job (§11).

### 3 — Exhaustive small cases

These run completely in every test run:

- every n in 0 … 3·13·8 = 312 (covers every tail length, block boundary
  and 32-/64-character line boundary several times) × every W in the
  matrix × every level, one-shot and chunk size 1;
- decode of every payload length 0 … 64 (including the invalid residues
  1, 6 and 11), valid content;
- classification: every combination of up to 3 leading whitespace bytes
  before `-`, legacy, and empty input.

### 4 — Invalid and unusual input

A **mutation generator** derives inputs from valid streams:

- insert, delete or replace one byte (inside the alphabet, and outside it);
- LF → CR LF everywhere, and at one position only;
- re-wrap at another width, including non-multiples of 32;
- leading and trailing whitespace; a lone `-`; truncation at every
  position of a short stream;
- quoted-printable-style soft breaks (`=` CR LF).

The expected outcome is always `oracle_decode(mutated)`: specific bytes
or a specific error class. Until the strict decoder exists, the current
SIMD decoders are only checked to be **memory-safe** on these inputs: a
panic is caught and counted (with overflow checks on, the scalar decoder
panics on invalid characters, annex A11), while memory corruption aborts
the test process and so cannot be missed. Equality with the oracle, and
the absence of panics, are enforced when the strict decoder lands.

Legacy streams are enforced from the start: every decoder must match the
upstream reference on mutated legacy streams, junk included (spec 0008
annex A15).

### 5 — Property tests

The same invariants as §2, stated as properties over generated inputs,
with shrinking to minimal counterexamples:

- `decode(encode(x, W)) == x`;
- `encode(x, W) == oracle_encode(x, W)`;
- streaming with any split == one-shot;
- output independent of level;
- `decode(s)` equals `oracle_decode(s)` for arbitrary `s`.

Tooling: `proptest` (see *Decisions recorded*).

### 6 — Fuzzing

Targets, each also checking memory safety through canaries and debug
assertions:

| Target | Input | Check |
|---|---|---|
| `fuzz_decode` | arbitrary bytes + level + split points | no panic / UB; equals `oracle_decode` (memory safety only until the strict decoder) |
| `fuzz_roundtrip` | arbitrary data + W + level + split points | round-trip, oracle equality, exact length |
| `fuzz_unchecked` | arbitrary data / stream + level | exact-size buffers with canaries |
| `fuzz_legacy` | arbitrary bytes | decode equals the C reference |

- **Corpus in the repo** (`rust/base91/fuzz/corpus/<target>/`), seeded
  with the regression cases of §12.
- **Replay without a fuzzing engine.** A normal integration test iterates
  over the corpus files and runs each target's check function. So every
  CI run replays the corpus on the stable toolchain, and a crash found
  once stays a regression test forever.
- **Long fuzzing runs** are manual or scheduled, not part of the blocking
  CI. Engine: libFuzzer via `cargo-fuzz -s none` on the pinned stable
  `rustc` (see *Decisions recorded*), in a `rust-fuzz` Nix attribute
  taking a time budget.

### 7 — Memory-safety checking

- `debug_assert!` bounds checks before every unchecked write in the codec
  hot loops (spec 0006 §1 introduces the pattern; extend it to every
  `get_unchecked_mut` / raw-pointer write in `codec.rs` and `simd/`).
  Tests run in debug mode, so they panic instead of corrupting memory.
- Canary-guarded buffers (§2) for every API that writes to caller memory.
- No AddressSanitizer and no Miri: neither is available with the pinned
  toolchain (see *Decisions recorded*). The two points above are what
  replace them.

### 8 — Executing the aarch64 kernels

A `rust-tests-aarch64` Nix attribute cross-compiles the test suite with
crane from `pkgsCross.aarch64-multiplatform` (the toolchain set up by spec
0004 §7). It **runs** the tests under qemu user-mode emulation, set as
the cargo runner for the aarch64 target. The NEON kernels have never been
executed so far; this is the only way to run them on x86_64 CI machines.
The whole matrix of §2 and §3 runs there (full `Simd128` = NEON coverage;
`Simd256` is skipped as unavailable).

The x86_64 run also covers the `force-scalar` feature, and `SimdLevel`
caps below the CPU's best level.

### 9 — CLI end-to-end tests

In `base91-cli/tests/cli.rs`, through real processes, both stdin pipes and
file arguments:

- sizes 0, 1, 12, 13, 14, 51, 52, 53, 104, 520, 8191, 8192, 8193, 12 000,
  16 000, 100 000, 1 000 000 (the 52-byte and ~16 kB thresholds of bugs 3
  and 1, and the I/O buffer edges);
- `--simd` with the default width and with `-w` 32, 64, 96; rejection of
  invalid widths;
- decode of each SIMD output with plain `-d` (auto-detection), with the
  output compared to the input and the encoded file compared to
  `oracle_encode`;
- legacy mode compared **byte for byte with the upstream C CLI** (built
  from the vendored 0.6.0 sources), for encode with the default width 76,
  `-w`, `b91enc`/`b91dec` names, and decode.

Implementation notes (phase 3):

- **aarch64 (§8):** `rust-tests-aarch64` runs `cargo test` in the
  derivation's *build* phase (`craneAarch64.mkCargoDerivation`), not the
  check phase: stdenv silently skips the check phase when the build
  machine cannot execute the host platform, so a cross `crane.cargoTest`
  would pass without running anything. crane sets qemu user-mode as the
  cargo runner itself. The test output is kept in `$out/test-output.txt`.
  A matrix test (`detected_level_matches_the_cpu`) asserts that
  `simd::detect()` is `Simd128` on aarch64, so a passing run means NEON
  really ran.
- **CLI (§9):** the tests live in `base91-cli/tests/end_to_end.rs`.
  `base91-cli` has its own `reference-tests` feature (Cargo does not build
  a dependency's dev-dependencies, so `base91-rs`'s feature cannot reach
  them). The upstream CLI is exposed as `base91_testkit::REFERENCE_CLI`.
  Workspace-wide attributes enable both features; attributes selecting
  only the library enable `base91-rs`'s alone (Cargo rejects features of
  unselected packages).

### 10 — Performance sanity test

A small **relative** test, `tests/perf_sanity.rs`, `#[ignore]` by default
and run by its own Nix attribute `rust-perf` (release build):

- data: 4 MB of seeded random bytes; median of 7 runs per measurement;
- measured, for encode and decode:
  - legacy;
  - SIMD at `Scalar`, `Simd128`, `Simd256`, each with W = 0 and W = 64;
- output: a table of throughputs, printed in the build log;
- assertions on **ratios only**, so that they hold across machines:
  - `Simd128 / Scalar ≥ R₁` and `Simd256 / Simd128 ≥ R₂`, for encode and
    decode;
  - wrapped (W = 64) vs unwrapped at the same level ≥ R₃, i.e. wrapping
    must not cost more than a set fraction;
  - SIMD `Scalar` vs legacy: reported, not asserted;
- levels the CPU lacks are skipped and reported;
- thresholds R₁–R₃ are **calibrated from a first run** on the dev machine
  and set with a safety margin, e.g. 70 % of the measured ratio. Until
  then the test only reports. After calibration, `rust-perf` fails on a
  ratio below threshold, but it is kept out of the correctness job, so a
  noisy machine does not block correctness CI.

`criterion` benchmarks (`benches/throughput.rs`) remain the tool for
detailed numbers. The performance budgets in specs 0006 (§7), 0007 (§7) and
0008 (§3.5, error detection ≤ 3 %) are measured there.

Implementation notes (phase 4):

- **Fuzzing (§6):** the fuzz crate is the workspace member
  `rust/base91/fuzz` (`base91-fuzz`). Its check functions live in its
  library, shared by the four libFuzzer targets and by the engine-free
  corpus replay (`tests/corpus_replay.rs`, run by `rust-tests`).
  `libfuzzer-sys` is optional (feature `libfuzzer`), so workspace builds
  never compile libFuzzer. The committed seed corpus has 38 inputs,
  covering the known bugs' triggers and spec 0008's tricky stream forms.
- **Fuzzing while the strict decoder is pending:** libFuzzer aborts on any
  panic, and with overflow checks on, the scalar SIMD decoder panics on
  invalid characters (annex A11). `rust-fuzz` therefore builds with
  `-Coverflow-checks=off` until then; debug assertions stay on. Under
  `cargo test`, the same panic is caught instead
  (`pending::quiet_expected_panics` keeps it out of the log).
- **First runs:** 30 to 60 s per target found one bug, in the test code
  itself. `check_legacy` compared the auto-detecting `simd::decode` with
  upstream on a stream that classifies as SIMD (it began with `-`). Fixed,
  and the input was added to the corpus. After that, tens of millions of
  runs per target came out clean.
- **Performance (§10):** calibrated on 2026-09-27 (see `perf_sanity.rs`
  for the measured table), with thresholds per direction (encode,
  decode), since the two behave very differently. Two findings for future
  specs:
  - wrapping (W = 64) currently costs 60–85 % of SIMD throughput;
  - `Simd256` is barely faster than `Simd128` (×0.87–1.45 depending on
    run and direction).

  The wrapped thresholds are to be raised once spec 0008's in-place layout
  is implemented. `rust-perf` runs in its own CI job.
- **Dev shell:** `cargo-fuzz` and `qemu-user` are added (`cargo-llvm-cov`
  comes with §11 coverage, phase 5).

### 11 — CI layout

All attributes in `default.nix`, built from the pinned nixpkgs:

| Attribute | Content | Blocking |
|---|---|---|
| `rust-tests` (existing, extended) | unit + integration tests, §3 exhaustive, §2 rotating subset, §6 corpus replay, §9 CLI | yes |
| `rust-tests-aarch64` | the same, cross-compiled, run under qemu | yes |
| `rust-tests-full` | full §2 matrix | yes (scheduled or on `main`) |
| `rust-nostd`, `rust-nostd-aarch64` | spec 0004 §7 | yes |
| `rust-perf` | §10 | no, until calibrated; then yes, in its own job |
| `rust-coverage` | line coverage report of `rust/base91/src` (`cargo-llvm-cov`) | no; the threshold is set after the first report |
| `rust-fuzz` | §6 long fuzzing runs, time budget as an argument | no (manual / scheduled) |

All tools these attributes need are in the pinned nixpkgs (verified; see
*Decisions recorded*). `nix-shell` (via `default.nix`'s dev shell) gets
`cargo-llvm-cov`, `cargo-fuzz` and `qemu-user` added, so the same commands
run locally.

### 12 — Regression tests for known bugs

| Bug | Named test | Fails on 0.2.2 by |
|---|---|---|
| 0005 `unchecked` realloc | `unchecked_simd_encode_len_26_27_52`, `unchecked_simd_decode_len_32_100` | abort (double free) / wrong output |
| 0006 bug 1 | `simd_decoder_chunked_20k`, `io_reader_simd_20k`, CLI `simd_roundtrip_16000` | wrong output |
| 0006 bug 2 | `scalar_decoder_chunk_size_1` | abort / debug-assert panic |
| 0006 bug 3 | `simd_wrap_payload_fills_last_line` (n = 26, W = 32; n = 52, W = 64), CLI `simd_roundtrip_52` | wrong output |
| 0006 bug 4 | `simd_streaming_encoder_empty` | `-` instead of empty |
| 0006 bug 5 | `legacy_decoder_chunk_size_1` | panic |
| 0006 bug 6 | `io_reader_lone_newline_read`, `io_reader_one_byte_reads`, CLI `decode_from_slow_pipe` | truncated output |
| 0008 A9–A13b | `strict_decoder_*` (one per annex item) | wrong output without error |
| 0004 | `rust-nostd*` builds | build failure |

The regression inputs also go into the fuzz corpus (§6).

Implementation notes (phase 2):

- The library-level tests live in `rust/base91/tests/regressions.rs`,
  each `#[ignore = "fixed by spec 000x …"]`; every one was checked to fail
  on 0.2.2 (`--ignored --exact`). The CLI ones (`simd_roundtrip_16000`,
  `simd_roundtrip_52`, `decode_from_slow_pipe`) come with the §9 CLI suite
  in phase 3.
- The `strict_decoder_*` tests (spec 0008 annex A9–A13b) are written with
  spec 0010: they depend on its error API, which does not
  exist yet. Until then, `pending::SimdStrictDecoding` covers them in the
  §4 and §5 tests.
- §3's decode of the invalid payload lengths and classification after
  leading whitespace also depend on the strict decoder. Until then they are
  exercised for memory safety only, through §4's mutations (truncation at
  every position of short streams, leading whitespace).

### 13 — Order of work

1. §1 oracles and their self-tests.
2. §2 / §3 matrix and §12 regression tests, **before** specs 0005 and
   0006 are implemented. The regression tests are checked to fail on 0.2.2
   (then marked `#[ignore = "fixed by spec 000x"]` until the fix lands in
   the same PR series).
3. §8 aarch64 execution, §9 CLI tests.
4. §6 fuzzing and §10 performance test.
5. §11 coverage.

From then on, every new public API or kernel (the slice API, the strict
decoder, a future AVX-512 kernel) joins the §2 matrix and the fuzz
targets in the same change that introduces it.

## Open questions

None.

## Decisions recorded (implementation, agreed 2026-09-27)

1. **Pending deviations.** The tests share one `pending` list. Each entry
   names a known deviation from spec 0008 and the spec that removes it
   (e.g. "wrapped output has no final LF, until spec 0006"). Checks consult
   the list and skip exactly those comparisons. A fix deletes its entry,
   which switches the checks on. Everything else is enforced from day one.
2. **Building the C reference.** `base91-rs`'s `reference-tests` feature
   enables `base91-testkit`'s `reference` feature, whose `build.rs`
   compiles the vendored upstream `base91.c` with the `cc` crate. Its
   global symbols (five functions, two tables) are renamed with a `ref_`
   prefix through `-D` defines, so the sources stay pristine. Nix test and
   clippy attributes enable it; plain `cargo test` works without a C
   compiler. `c-compat-tests` (the optimized `src/base91.c`, for
   benchmarks) is unchanged.
3. **Reference CLI (§9).** Under the same feature, `base91-testkit`'s
   `build.rs` builds the vendored `cli.c` + `base91.c` into an executable
   in its build directory. CLI tests skip the comparison when the feature
   is off.
4. **aarch64 scope (§8).** `rust-tests-aarch64` runs the `base91-rs`
   library and integration tests (where the NEON kernels run), not the CLI
   tests: a child aarch64 process cannot run under qemu user-mode in the
   Nix sandbox (no `binfmt`).
5. **Reproducible randomness.** Random subsets use a seed that is printed
   on failure and can be overridden with `BASE91_TEST_SEED`. In CI it is
   derived from the commit, so reruns of a commit test the same cases.
6. **Performance thresholds (§10)** are calibrated on the dev machine when
   the test is written, at 70 % of the measured ratios, so the test
   enforces them from the start.
7. **Commits:** one per §13 phase. Each phase ends with `cargo fmt`,
   `cargo clippy`, `reuse lint` and the Nix build of the affected
   attributes.

## Decisions recorded (test dependencies and reference sources)

- **`proptest` is accepted as a dev-dependency** (§5). The library stays
  dependency-free. The vendored crate set of `default.nix` (and of
  nixpkgs' `pybase91` derivation, spec 0003) grows, and its hash must be
  recomputed when `proptest` is added.
- **Upstream basE91 0.6.0 is vendored** (§1). Durable sources, checked on
  2026-09-27:
  - SourceForge release tarball
    `https://downloads.sourceforge.net/project/base91/basE91/0.6.0/base91-0.6.0.tar.gz`,
    15 009 bytes, sha256
    `02cfae7322c1f865ca6ce8f2e0bb8d38c8513e76aed67bf1c94eab1343c6c651`;
  - **Software Heritage**, which archives by content hash, so the files
    cannot silently change or disappear:

    | File | SWHID | sha256 |
    |---|---|---|
    | `base91.c` | `swh:1:cnt:3d9d7eaab6294bffff9bb152b5128c2b8f4820b9` | `ce6c825e24342271b525f2cecc5f70294047246963cc5361003326c9d6ce8290` |
    | `base91.h` | `swh:1:cnt:7a44a60b357e749b24d543d12473977c74b4acb1` | `30d476141130aa6391a240c9b94e42e60f71b38f056a2a349fec9bc96562b6bf` |
    | `cli.c` | `swh:1:cnt:f3659915368a1d514fbb6c56db88e9b01d0f48cc` | `a37ce4cebb71ad7e6d0c23157e651ec6848770fd6fb931706b70a2c044ba87d5` |

  These three files are **byte-identical** to `src/base91.c`,
  `src/base91.h` and `src/cli.c` at tag `v0.1.0` of this repo (checked with
  `cmp`). The current `src/` copies differ (optimized in `ba47d9c`).
  Vendored into `rust/base91-testkit/reference/basE91-0.6.0/`: `base91.c`,
  `base91.h`, `cli.c` (for the §9 reference CLI) and `LICENSE`, unmodified,
  with a `PROVENANCE` file giving the URLs, hashes and SWHIDs above. The
  vendored files are declared in `REUSE.toml` (`[[annotations]]`,
  BSD-3-Clause, Joachim Henke), not edited. A test checks their sha256
  against the table above.
  (nixpkgs' `base91` package is not an independent source: it builds this
  repo at `v0.1.0`.)

## Decisions recorded (tooling, verified on the pinned nixpkgs)

Checked with `nix-shell -I nixpkgs=<pin> -p …` (logs in
`/tmp/base91-tooling-check.log` and `/tmp/base91-cargo-fuzz-check.log` on
the dev machine):

| Tool | Pinned nixpkgs | Consequence |
|---|---|---|
| `cargo-llvm-cov` | 0.6.20 | §11 coverage: available |
| qemu user-mode (`qemu-user`) | 10.1.2, `qemu-aarch64` | §8 aarch64 execution: available |
| `cargo-fuzz` | 0.13.1 | fuzzing engine (below) |
| AFL++ (`aflplusplus`) | 4.34c | available, but `cargo-afl` (afl.rs) is **not** packaged |
| Miri, nightly `rustc` | **absent** | no Miri |
| ASan runtime in `rustc` | **absent** (`librustc-stable_rt.asan.a` missing; `rustc.nix` has no sanitizer option) | no ASan, not even with `RUSTC_BOOTSTRAP=1` (link fails) |

- **Fuzzing engine: libFuzzer through `cargo-fuzz`, with `-s none`** (no
  sanitizer), on the pinned **stable** `rustc`, with **no
  `RUSTC_BOOTSTRAP` hack**. Verified: it builds, runs, and found a planted
  bug in a demo target within 60 s. Coverage guidance works;
  instrumentation uses stable `-C` flags only. AFL++ would need packaging
  afl.rs, for no advantage here.
- **Memory checking: no Miri, no ASan** with the pinned toolchain. §7's
  `debug_assert!` bounds checks and canary buffers carry that role. Fuzz
  targets are therefore built with debug assertions on (`cargo fuzz`
  default: `-Cdebug-assertions`), so an out-of-bounds unchecked write
  becomes a panic that libFuzzer reports. Revisit if the project ever
  accepts a nightly toolchain input.

## Verification

```bash
cd rust
cargo test -p base91-rs                           # oracle self-tests, matrix subset, exhaustive, corpus replay
cargo test -p base91-cli                          # CLI end-to-end
cargo test -p base91-rs --release -- --ignored perf_sanity   # prints the ratio table
nix-build ../default.nix -A rust-tests -A rust-tests-aarch64 -A rust-tests-full
```

On the 0.2.2 code (before specs 0005 / 0006), the §12 regression tests
fail as listed, and everything else passes.

`reuse lint` must pass.
