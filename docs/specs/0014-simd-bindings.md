<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0014 — Python and C bindings aligned with the SIMD variant

**Status:** implemented
**App:** base91
**Implemented in:** 2026-10-16

## Problem

What sets base91 apart is the speed of its SIMD variant: about 16 GiB/s
encoding with AVX2, 16 times our Henke path and 25 times the `base91`
crate.  Only the `base91` command
and the Rust crate offer it.

| Package | Implementation | Henke | SIMD |
|---|---|---|---|
| `base91` (CLI; nixpkgs, crates.io `base91-cli`) | Rust | ✓ | ✓ |
| `base91-rs` (crates.io) | Rust | ✓ | ✓ |
| `pybase91` (PyPI; nixpkgs PR 504526) | Rust, PyO3 | ✓ | — |
| `libbase91` (nixpkgs PR 504524, stale) | Henke's C (`src/`) | ✓ | — |

- **Python** binds the Rust library, but `rust/base91/src/python.rs`
  exposes only `encode`, `decode` and the streaming `Encoder` / `Decoder`
  of Henke's format.
- **C**: `libbase91` is Henke's C code from `src/`, lightly modified
  (`ba47d9c`), built by `default.nix` (`base91-clib`) and proposed to
  nixpkgs at 0.2.2 (NixOS/nixpkgs#504524, open since March, a committer's
  review unanswered since April).  A first plan, a C interface over the
  Rust library (cdylib, `c_api.rs`, cbindgen), was dropped the day the PR
  was opened.  So C programs cannot use the SIMD variant, nor the Rust
  codec of Henke's format.
- **Python wheels** are built for CPython 3.11 to 3.13; 3.14, nixpkgs'
  default `python3` since 2026-05-30, gets the sdist and needs a Rust
  toolchain.

## The model: one core, two satellites

- **Core: `base91`**, the Rust workspace and its CLI.  In nixpkgs, the
  `base91` package owns the version, the source and the vendored crates.
- **Satellites: `pybase91` (Python) and `libbase91` (C)**, thin bindings
  over `base91-rs`, offering both formats, the SIMD variant first.  In
  nixpkgs they `inherit (base91) version src cargoDeps`, so one
  `nix-update base91` (or r-ryantm) moves all three.

## Goals

- **G1.** Python: the SIMD variant (one-shot and streaming; the decoders
  detect the format), the SIMD level, and typed errors with offsets.
- **G2.** C: `libbase91` built from the Rust library, with
  - Henke's five `basE91_*` functions and `struct basE91`, binary
    compatible with his `base91.h` (a program built against Henke's C
    library runs unchanged with ours), and
  - a new API covering both formats, without allocation.
- **G3.** `libbase91` packaged as a satellite: the staged nixpkgs recipe
  inherits `base91`'s source and crates; `default.nix` builds it; CI
  tests it.
- **G4.** Wheels for CPython 3.11 to 3.14.
- **G5.** Shipped in 0.3.3, with spec 0015.

## Non-goals

- **N1.** A streaming SIMD API in C (it needs an opaque state with a
  stable size; later, if asked for).
- **N2.** Publishing `base91-capi` to crates.io: C users get the library
  from distributions or a source build.
- **N3.** Changing Python's existing `encode`, `decode`, `Encoder`,
  `Decoder` (Henke's format, as today).
- **N4.** Changing `src/` (it stays the C reference; base91-testkit
  vendors its own pristine copy of upstream 0.6.0 as the oracle).
- **N5.** Windows builds of `libbase91`.
- **N6.** Go bindings.

## Specification

### 1. Format detection

`simd::decode`, `simd::decode_slice` and `simd::Decoder` already detect
the format (spec 0008 §2: after optional whitespace, `-` starts a SIMD
stream, decoded strictly; anything else is Henke's format).  The bindings
use them as they are, so their SIMD decoders decode both formats, as
`base91 -d` does.

`base91-rs` gains one hidden module, `#[doc(hidden)] pub mod __capi`,
outside the documented API and its semver promises, for `base91-capi`:
loading and storing the Henke codec's state (`queue`, `nbits`, `val`:
the Rust codec is a port of Henke's, with the same state, `u32::MAX` for
his -1), and the codecs' loops on `MaybeUninit` output, each checking the
output's length against what it may write before writing.  It holds no
`unsafe`: that stays in `base91-capi`.

### 2. Python (`rust/base91/src/python.rs`)

Added, next to the unchanged Henke API:

| Python | Rust |
|---|---|
| `simd_encode(data: bytes, wrap: int = 0) -> bytes` | `simd::encode` |
| `simd_decode(data: bytes) -> bytes` (either format, detected) | `simd::decode` |
| `SimdEncoder(wrap: int = 0)`, `.update(data)`, `.finish() -> bytes` | `simd::Encoder` |
| `SimdDecoder()`, `.update(data)`, `.finish() -> bytes` (either format) | `simd::Decoder` |
| `simd_level() -> str` (`"avx2"`, `"sse4.1"`, `"neon"`, `"scalar"`) | `simd::detect` |
| `Error(ValueError)`, with `.kind` and `.offset` (or `None`), declared in the stub | `Error` |

The best available level is used, as in the CLI.  An invalid width
raises `Error` (kind `"invalid_width"`).  `Error` is a PyO3 class
extending `ValueError` (`#[pyclass(extends = PyValueError)]`, with
`kind` and `offset` getters), so type checkers see its attributes.

pyo3-stub-gen renders default argument values (`wrap = 0`) with a real
interpreter, so the stub generator links libpython: pyo3's
`extension-module` moves from the `python` feature to a new
`extension-module` feature, which maturin enables (`pyproject.toml`).  The stub (`pybase91.pyi`) is
regenerated; `pybase91-stub-check` keeps it honest.  Python tests cover
each function, chunked streaming against one-shot, errors and offsets.

### 3. C library: crate `rust/base91-capi`

A workspace member, `publish = false`, `crate-type = ["cdylib",
"staticlib"]`, depending on `base91-rs` with its `std` feature only (no
`io`): with `std`, the SIMD level is detected at run time; without it,
from the build's target features, which a portable build would leave at
the scalar kernel.  It builds `libbase91.so` (`.dylib`
on macOS) and `libbase91.a`.  It is a separate crate because a cdylib
in `base91-rs` itself breaks `no_std` and `default-features = false`
builds downstream (spec 0004 §6).

**Henke-compatible part** (`basE91_init`, `basE91_encode`,
`basE91_encode_end`, `basE91_decode`, `basE91_decode_end`):

- `struct basE91 { unsigned long queue; unsigned int nbits; int val; }`
  exactly as in Henke's `base91.h`: callers allocate it, so its size and
  layout are ABI.  The functions keep their state in it (`#[repr(C)]`),
  between calls, as Henke's do.
- The same signatures and contracts: same bytes written, same return
  values, for every input, every split of the input across calls, and
  every sequence of calls Henke's code accepts.  Output buffer sizes are
  the caller's responsibility, as in Henke's API.
- Input and output may overlap as in Henke's command-line tool, whose
  input buffer lies inside its output buffer: the functions read the input
  through a 4 KiB stack buffer, one chunk at a time, before writing that
  chunk's output, so the writes stay behind the unread input, as in his
  byte-by-byte loop.  No heap allocation.

**New API**, prefix `base91_`, no allocation, sizes checked before
anything is written:

```c
typedef enum { BASE91_HENKE = 0, BASE91_SIMD = 1 } base91_format;

/* Return codes: 0 on success. */
enum {
  BASE91_OK = 0,
  BASE91_ERR_BUFFER_TOO_SMALL, BASE91_ERR_INVALID_WIDTH,
  BASE91_ERR_INVALID_CHARACTER, BASE91_ERR_INVALID_LAYOUT,
  BASE91_ERR_INVALID_LENGTH, BASE91_ERR_INVALID_VALUE,
  BASE91_ERR_NULL_POINTER, BASE91_ERR_INVALID_FORMAT,
};

size_t base91_encoded_len_max(base91_format f, size_t len, size_t wrap);
int    base91_encode(base91_format f, const void *in, size_t len,
                     size_t wrap, void *out, size_t out_cap,
                     size_t *written);
size_t base91_decoded_len_max(size_t len);
int    base91_decode(const void *in, size_t len, void *out,
                     size_t out_cap, size_t *written,
                     size_t *error_offset);   /* either format, detected */
const char *base91_simd_level(void);
const char *base91_strerror(int code);
const char *base91_version(void);
```

- `wrap` applies to the SIMD format (0, or a multiple of 32); for Henke's
  format it must be 0 (`BASE91_ERR_INVALID_WIDTH` otherwise).
- `error_offset` (may be NULL) receives the offset of a decoding error.
- `BUFFER_TOO_SMALL` sets `*written` to the size needed.

**Header** `base91.h`: hand-written, in `rust/base91-capi/include/`.  Its
Henke part is Henke's declarations verbatim; the new API follows.  The
`c-api` test (§4), compiled with `-Wall -Werror`, calls every function
and checks its results, so a function missing from the library, renamed,
or declared with the wrong arguments fails CI.

**Safety at the boundary:** no Rust panic crosses into C.  Every exported
function runs its body in `catch_unwind` and aborts the process on a
panic.  A null pointer with a non-zero length is
`BASE91_ERR_NULL_POINTER` in the new API, and undefined behavior in the
Henke part, as in Henke's.

**Artifacts:** `lib/libbase91.so.0` (SONAME `libbase91.so.0`, with a
`libbase91.so` link; on macOS the install name is fixed by the nixpkgs
hook), `lib/libbase91.a`, `include/base91.h`, `lib/pkgconfig/base91.pc`
(`Libs: -lbase91`, `Libs.private`: the system libraries of the static
library, from `rustc --print native-static-libs`; `Version` = the
workspace version).

### 4. Nix and nixpkgs

- **Staged recipe** `nixpkgs/pkgs/by-name/li/libbase91/package.nix`:
  `rustPlatform.buildRustPackage`, `inherit (base91) version src
  cargoDeps`, `cargoRoot = "rust"`, `buildAndTestSubdir =
  "rust/base91-capi"`; `outputs = [ "out" "dev" ]` (header and `.pc` in
  `dev`); `passthru.tests`:
  - `pkg-config`: `testers.hasPkgConfigModules`;
  - `c-api`: a C program (`rust/base91-capi/tests/c/api.c`) built with
    `pkg-config`, `-Wall -Werror`, linked dynamically then statically
    (`--static`), calling both APIs: round trips, errors and offsets, and
    the SIMD level, which must be the CPU's best kernel;
  - `henke-compat`: Henke's basE91 0.6.0 `cli.c`, compiled against our
    header and linked with `libbase91`, must produce the same output as
    `${base91}/bin/base91` on the same inputs (encode, decode, wrapped).
- **`default.nix`**: `libbase91` built from the staged recipe through the
  overlay (as `base91` and `pybase91`); `nixpkgs-staging-check` covers
  the new file.  `base91-clib` (Henke's C) is removed; the C workflow
  builds `libbase91` and its tests instead.  The Rust tests of
  `base91-capi` (the Henke part against the test kit's C reference, on
  random inputs and random splits) join `rust-tests`.
- **PyPI wheels**: `-i python3.11 python3.12 python3.13 python3.14` in
  `publish.yaml` (G4).

### 5. Release and nixpkgs order

1. 0.3.3 (with spec 0015): tag; `publish.yaml` publishes the crates and
   the wheels (`base91-capi` is not published, N2).
2. r-ryantm opens `base91: 0.3.2 -> 0.3.3` in nixpkgs (`updateScript`).
   The maintainer reviews it and merges it with the merge bot (allowed
   for r-ryantm's PRs).  If `pybase91` (PR 504526) is merged by then, it
   moves with it.
3. `libbase91` (PR 504524, updated in place, `libbase91: init at
   0.3.3`): needs `base91` 0.3.3 in master; stacked on the bot's PR until
   then, as PR 504526 was on PR 504523.  It touches only
   `pkgs/by-name`, so the merge bot can merge it once a committer
   approves.  The reply to mdaniels5757's review explains that the build
   is now cargo, so `make install` no longer applies.

## Tests

- Python: the new functions and classes, streaming at every chunk size
  against one-shot, errors with kind and offset; stub check.
- `base91-capi` (Rust tests): the Henke part byte-identical to the test
  kit's C reference on random inputs, random splits across calls, both
  directions; the new API against `base91-rs`, every error code.
- The `c-api` and `henke-compat` tests (§4), built by CI.
- `sizeof(struct basE91)` and its field offsets equal Henke's, checked
  from C on the CI platforms.
- Wheels for 3.11 to 3.14 contain the extension (existing wheel check).

## Open questions

- None.  Confirmed 2026-10-15: the C and Python names, SONAME `.so.0`,
  `base91-clib` removed, one release (0.3.3).

## References

- [spec 0004](0004-no-std-build.md) §6 — why the cdylib is a separate
  crate
- [spec 0008](0008-wire-format.md) §2 — format detection
- [spec 0012](0012-nixpkgs-aligned-build.md) — staged recipes, `inherit`
  from `base91`, N2 (the C library, set aside then)
- [spec 0015](0015-crates-io-readme.md) — the README, shipped with this
- NixOS/nixpkgs#504524 (`libbase91`), #504526 (`pybase91`)
- `ba47d9c` — the dropped cdylib plan
