<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0004 — Make `base91-rs` actually build under `no_std`

**Status:** implemented
**App:** base91
**Implemented in:** 2026-09-27

## Problem

GitHub issue [#1](https://github.com/douzebis/base91/issues/1) reports that
`base91-rs` advertises `no_std` support (`#![cfg_attr(not(feature = "std"),
no_std)]` in `src/lib.rs`, the crate description, `rust/base91/README.md`,
spec 0001 §3.4) but uses `std` items unconditionally.

The issue lists five occurrences, but the real breakage is broader. Building
with `cargo build --lib --no-default-features` on x86_64 (commit `f994029`)
fails with 235 errors, in four groups:

1. **`std::` paths that exist in `core`** — `std::mem::forget`
   (`simd/mod.rs:234`, `:251`), `std::slice::from_raw_parts_mut`
   (`simd/mod.rs:376`), `std::array::from_fn` (`simd/scalar.rs:308`),
   `std::ptr::copy_nonoverlapping` (`simd/x86.rs:694`,
   `simd/aarch64.rs:334`), `std::arch::x86_64::*` (`simd/x86.rs:53`,
   ~190 knock-on errors), `std::arch::aarch64::*` (`simd/aarch64.rs:39`).
2. **`Vec` without an `alloc` import** — `codec.rs`, `lib.rs`,
   `simd/mod.rs`, `simd/scalar.rs`. The whole public API is `Vec`-based.
3. **Runtime CPU detection is `std`-only** — `std::sync::OnceLock`
   (`simd/mod.rs:42`, `:83`) and `is_x86_feature_detected!`
   (`simd/mod.rs:93`, `:96`) have no `core` equivalent.
4. **`cdylib` crate-type** — see below.

Nothing in CI builds without default features, so none of this was caught.

### The `cdylib` crate-type breaks downstream users

`Cargo.toml` declares `crate-type = ["rlib", "cdylib"]`. Cargo builds every
declared crate-type **even when the crate is only a dependency**, and a
`no_std` `cdylib` needs a `#[panic_handler]`, a global allocator and
`panic = "abort"`. Tested with a minimal crate that has the same layout
(rustc 1.91.1 and 1.95.0):

| Downstream build | Result |
|---|---|
| bare metal (`thumbv7em-none-eabihf`, `x86_64-unknown-none`) | builds; rustc warns "dropping unsupported crate type `cdylib`" in the user's build |
| `wasm32v1-none` | **fails** (wasm supports `cdylib`) |
| `no_std` library, Linux host | **fails** |
| ordinary `std` binary with `base91-rs = { default-features = false }` | **fails** |

Even after groups 1–3 are fixed, anyone who disables default features on
Linux, macOS or Windows (for example to drop the `io` adapters) could not
build.

The only consumer of the `cdylib` is the Python extension (maturin).

maturin does not need `cdylib` in `Cargo.toml`. With Rust ≥ 1.64 it runs
`cargo rustc --lib --crate-type cdylib` itself (verified with maturin 1.9.6:
"Setting crate_type to cdylib for Rust >= 1.64.0"). However, **it must be
told the bindings type**. Without `cdylib`, auto-detection sees the
`post_build` binary target and silently produces a "bin" wheel that
contains only the `post_build` script and no extension module. With
`-b pyo3`, the wheel contains `pybase91/pybase91.cpython-*.so`, and an
encode/decode round-trip works.

### Stale C API files

`src/c_api.rs` and `cbindgen.toml` are not built: `lib.rs` has no
`mod c_api`, and `libbase91` is built from the C sources. Commit
`ba47d9c` ("Restructure C library: native C build, drop Rust cdylib +
cbindgen") says it deletes both files, but on `main` it only removed
`mod c_api` from `lib.rs`. The pre-rewrite copy of that commit
(`1e1f485`, still reachable through tags `v0.2.0` / `v0.2.1`) does delete
them. The history rewrite that added "(GitHub: @douzebis)" to the SPDX
headers resurrected the two files.

## Goals

- `cargo build --lib --no-default-features` (no `std`, no `alloc`) and
  `cargo build --lib --no-default-features --features alloc` compile
  cleanly (no errors, no warnings) for the x86_64 host, `wasm32v1-none` and
  aarch64.
- A downstream `std` or `no_std` crate depending on `base91-rs` with
  `default-features = false` builds on hosted targets.
- Under `std`, SIMD level selection keeps working exactly as it does today
  (runtime detection, cached in `OnceLock`).
- Under `no_std`, SIMD level selection happens at compile time from
  `cfg(target_feature = …)`.
- The Python wheel is unchanged (same module, same `.so`).
- CI catches any future `no_std` regression, using only the pinned nixpkgs
  (no rustup, no new Nix inputs).
- Close issue #1.

## Non-goals

- Runtime CPU detection under `no_std` (hand-written `cpuid` / `xgetbv`,
  atomic caching). This was considered and rejected; see *Alternatives*.
- Allocation-free encode/decode APIs. This spec only makes a build without
  `alloc` *possible*; the APIs come in spec 0010 (which supersedes 0007).
- Fixing the `simd::*_unchecked` memory-safety bug (spec 0005).
- SVE2 support on aarch64.
- New Cargo dependencies.

## Specification

### 1 — Feature layout: `std` ⊃ `alloc` ⊃ core

`rust/base91/Cargo.toml`:

```toml
[features]
default        = ["std", "io"]
std            = ["alloc"]
alloc          = []
io             = ["std"]
python         = ["std", "pyo3", "pyo3-stub-gen", "pyo3-stub-gen-derive"]
```

`src/lib.rs`:

```rust
#![cfg_attr(not(any(feature = "std", test)), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;
```

- In every module that uses `Vec`, add `use alloc::vec::Vec;`:
  - `lib.rs`, `codec.rs`, `simd/mod.rs`: these modules are still compiled
    without `alloc` (for the §2 items), so the import itself is
    `#[cfg(feature = "alloc")]`. Otherwise it refers to a crate that is not
    linked.
  - `simd/scalar.rs`, `simd/x86.rs`, `simd/aarch64.rs`: the whole module is
    gated on `alloc` (§2), so a plain import is enough.
- Adding `test` to the `cfg_attr` lets `cargo test --no-default-features
  --features alloc` run. The test harness links `std`, but the `std`
  *feature* stays off, so the compile-time detection path (§4) is the one
  exercised. The `wasm32v1-none` build (§7) is what checks that the library
  itself does not need `std`.
- The unit-test modules that use the `Vec` API are gated
  `#[cfg(all(test, feature = "alloc"))]` instead of `#[cfg(test)]`:
  `lib.rs:176`, `codec.rs:397`, `simd/mod.rs:564`. The test code in
  `simd/scalar.rs` (`:32`, `:409`) and `io.rs:178` needs no change: those
  modules are already gated on `alloc` (§2) or `std`.

  `cargo test --lib --no-default-features` then compiles and runs (with
  zero tests), instead of failing.

### 2 — What is available without `alloc`

Everything that returns or takes a `Vec` is gated with
`#[cfg(feature = "alloc")]`. Without `alloc`, the public API is:

| Item | Why it's available |
|---|---|
| `encode_size_hint`, `decode_size_hint` | pure arithmetic |
| `encode_unchecked`, `decode_unchecked` (Henke, crate root) | already pointer-based (`codec.rs:267`, `:329`) |
| `simd::SimdLevel`, `simd::detect()` | no allocation |
| `simd::encode_size_hint`, `simd::decode_size_hint` | pure arithmetic |

Gated (require `alloc`): `encode`, `decode`, `Encoder`, `Decoder`,
`simd::encode`, `simd::decode`, `simd::encode_unchecked`,
`simd::decode_unchecked` (they use `Vec` internally; see spec 0005),
`simd::Encoder`, `simd::Decoder`, and the `simd::scalar` module.

Internal code only used by gated items (`effective_level`, `encode_into`,
`decode_into`, `insert_wrap_newlines`, the `x86` / `aarch64` kernel
modules) is gated the same way, with `#[cfg(all(feature = "alloc", …))]`
on the `mod` declarations. That way nothing is dead code in a build
without `alloc`. **No `#[allow(dead_code)]`.**

Spec 0010 will make the SIMD and streaming paths available without `alloc`.

### 3 — `std::` → `core::` for items available in `core`

Mechanical replacement, with no change in behavior:

| Location | From | To |
|---|---|---|
| `simd/mod.rs:234`, `:251` | `std::mem::forget` | `core::mem::forget` |
| `simd/mod.rs:376` | `std::slice::from_raw_parts_mut` | `core::slice::from_raw_parts_mut` |
| `simd/scalar.rs:308` | `std::array::from_fn` | `core::array::from_fn` |
| `simd/x86.rs:53` | `std::arch::x86_64::*` | `core::arch::x86_64::*` |
| `simd/x86.rs:694` | `std::ptr::copy_nonoverlapping` | `core::ptr::copy_nonoverlapping` |
| `simd/aarch64.rs:39` | `std::arch::aarch64::*` | `core::arch::aarch64::*` |
| `simd/aarch64.rs:334` | `std::ptr::copy_nonoverlapping` | `core::ptr::copy_nonoverlapping` |

Test modules that use `std`-only items (e.g. `std::collections::HashSet` in
`simd/scalar.rs`) keep them as they are. `io.rs` and `python.rs` stay as
they are: they are already gated behind `std`-requiring features.

### 4 — Split arch detection by `std` feature

In `src/simd/mod.rs`, only `detect_arch()` and `arch_level()` change. The
bodies of `ArchLevel`, `effective_level()`, `detect()` and the kernels are
unchanged (§2 only adds `cfg` gates to some of them).

**With `feature = "std"`** — current code, unchanged:

```rust
#[cfg(feature = "std")]
static ARCH_LEVEL: std::sync::OnceLock<ArchLevel> = std::sync::OnceLock::new();

#[cfg(feature = "std")]
fn arch_level() -> ArchLevel {
    *ARCH_LEVEL.get_or_init(detect_arch)
}
```

`detect_arch()` keeps its current body (runtime `is_x86_feature_detected!`
on x86_64, `Neon` on aarch64, `Scalar` elsewhere, `force-scalar` override)
and becomes `#[cfg(feature = "std")]`, like `ARCH_LEVEL` and this
`arch_level()`.

**Without `feature = "std"`** — compile-time selection, no cache needed.
Both `detect_arch_static()` and this `arch_level()` are
`#[cfg(not(feature = "std"))]`, so each build contains exactly one
detection path and neither is dead code:

```rust
#[cfg(not(feature = "std"))]
#[inline(always)]
fn arch_level() -> ArchLevel {
    detect_arch_static()
}
```

`detect_arch_static()` returns:

| Condition | Result |
|---|---|
| `feature = "force-scalar"` | `Scalar` |
| x86_64, `cfg!(target_feature = "avx2")` | `Avx2` |
| x86_64, `cfg!(all(target_feature = "sse4.1", target_feature = "ssse3"))` | `Sse41` |
| x86_64, otherwise | `Scalar` |
| aarch64, `cfg!(target_feature = "neon")` | `Neon` |
| aarch64, otherwise | `Scalar` |
| any other arch | `Scalar` |

Implementation notes:

- Use `if cfg!(…)` expressions rather than `#[cfg(…)]` blocks inside the
  arch branches. That way every `ArchLevel` variant stays constructed
  whatever the target features are, which avoids `dead_code` warnings. The
  branches are constant-folded.
- The feature sets checked must match the `#[target_feature(enable = …)]`
  attributes on the kernels in `x86.rs` (`avx2,ssse3` and `sse4.1,ssse3`).
  Unsafe calls into those kernels keep their `// SAFETY:` comments, updated
  to say "feature guaranteed at compile time" on the `no_std` path.
- On aarch64 the `no_std` path checks `neon` explicitly, rather than
  assuming it as the `std` path does, because
  `aarch64-unknown-none-softfloat` disables NEON.
- The `std` path does not change: a `std` build with
  `-C target-feature=+avx2` still detects at runtime (and `OnceLock` caching
  stays).

**Soft-float targets** (verified on `x86_64-unknown-none`, rustc 1.91.1
and 1.95.0):

- The `#[target_feature(enable = "avx2,ssse3")]` and `"sse4.1,ssse3"`
  kernels compile with no errors or warnings, so no extra `cfg` gate is
  needed.
- By default the target enables only `fxsr`, so `detect_arch_static()`
  returns `Scalar`, which is correct.
- If the user passes `-C target-feature=+avx2` (or `+sse4.1,+ssse3`),
  `cfg!(target_feature = "avx2")` becomes true, but LLVM still emits no
  `xmm`/`ymm` instructions: the intrinsics are lowered to scalar code (a
  32-byte vector add became ~150 instructions). The result is presumably
  correct but much slower than the dedicated scalar path. No stable `cfg`
  exposes soft-float, so the crate cannot detect this. It is documented
  (§5) rather than handled in code.
- Soft-float is a promise the toolchain makes to the execution environment
  (kernel, firmware) on behalf of every crate compiled for the target: no
  FP/SIMD register is touched. `base91-rs` is bound by it without taking
  part in it. The compiler enforces it on ordinary Rust code, including
  `#[target_feature]` kernels, as shown above. The crate could only break
  it by bypassing the compiler (`asm!` using `xmm`/`ymm`, or linked C or
  assembly objects). It does neither, and must keep it that way.

### 5 — Documentation

- `simd::detect()` rustdoc: add a paragraph saying that without the `std`
  feature, the level comes from the compile-time target features, and
  recommend `-C target-cpu=…` or `-C target-feature=…` to enable SIMD
  kernels in `no_std` builds. Add a warning: on soft-float targets
  (`x86_64-unknown-none`, `aarch64-unknown-none-softfloat`, …) enabling
  SIMD target features makes things slower, not faster; keep the target's
  defaults.
- `simd/mod.rs` module doc (dispatch table): the same note.
- `rust/base91/README.md` `no_std` section: document the three levels
  (`std`, `alloc`, bare core), the §2 table, compile-time SIMD selection,
  and the soft-float warning.
- Spec 0001 §3.4: update the feature table (`alloc` added, `std` implies
  `alloc`). Spec 0001 §4: mark the Rust C API as dropped (see `ba47d9c`).
  Spec 0001 §5: the `cdylib` is now produced by maturin, not declared in
  `Cargo.toml`.
- `CHANGELOG.md`:
  - `Fixed`: issue #1, and `default-features = false` on hosted targets.
  - `Added`: the `alloc` feature.

### 6 — Drop `cdylib` and the stale C API

- `rust/base91/Cargo.toml`: remove the `crate-type` line (`rlib` is the
  default).
- `rust/base91/pyproject.toml`, `[tool.maturin]`: add
  `bindings = "pyo3"`. This is **required**; without it maturin builds a
  bin wheel with no extension module (see *Problem*). It covers every
  Python build path, since they all go through maturin and read
  `pyproject.toml`: `default.nix` (`maturinBuildHook`),
  `.github/workflows/python.yml`, and `.github/workflows/pypi.yml`
  (`maturin-action`).
- The `pyproject.toml` requirement `maturin>=1.7` already covers
  crate-type injection (Rust ≥ 1.64, and the crate's MSRV is 1.91).
- Delete `rust/base91/src/c_api.rs` and `rust/base91/cbindgen.toml`, as
  `ba47d9c` intended. Nothing else references them (`REUSE.toml` and
  `.gitignore` checked).

This is not a breaking change for Rust users: the only symbols the Rust
`cdylib` exported were the PyO3 ones.

### 7 — CI (pinned nixpkgs only)

rustup is not used. It is impure (mutable `~/.rustup`, network access at
build time, moving channels). Everything comes from the nixpkgs pin in
`default.nix`:

- **x86_64 host** — nixpkgs `rustc`.
- **`wasm32v1-none`** — a target with no `std` at all. nixpkgs' `rustc`
  already ships its `core` / `alloc` / `compiler_builtins` (verified on the
  current pin), so it costs nothing.
- **aarch64** — crane instantiated from `pkgsCross.aarch64-multiplatform`
  (target `aarch64-unknown-linux-gnu`). Building for a hosted target with
  `--no-default-features` still exercises the `no_std` code path, since
  the crate is `#![no_std]` in that configuration. Verified with a proof
  of concept (pinned nixpkgs + pinned crane, no other input):
  - The cross `rustc` is not in `cache.nixos.org`, and nixpkgs builds only
    the target's `std` (`fastCross`): **8 min** on the dev machine. The
    cross gcc comes from the cache. The result goes into the existing
    `base91` Cachix cache after the first CI run.
  - Its sysroot contains `std` for both aarch64 and x86_64, so `build.rs`
    compiles for the host.
  - `crane.cargoBuild` from `pkgsCross` targets aarch64 automatically: no
    `--target`, no `overrideToolchain`. Output lands in
    `target/aarch64-unknown-linux-gnu/`.
  - With default features, `base91-rs` builds for aarch64 with no warnings.
    This is the first time `simd/aarch64.rs` is compiled by the repo's
    tooling.
  - With `--no-default-features` on the current code, it fails, including
    on the NEON intrinsics (`simd/aarch64.rs:39`). So the job catches
    aarch64-specific `no_std` breakage that the host and `wasm32v1-none`
    builds cannot see.

Add a `rust-nostd` attribute to `default.nix`, modeled on `rustClippy`.
crane runs cargo from the workspace root (`rust/`), so every command
selects the package explicitly. Otherwise `--no-default-features` would
apply to the wrong package, or be rejected at the workspace level:

```bash
P="-p base91-rs --lib --no-default-features"
for feat in "" "--features alloc"; do
  cargo build  $P $feat
  cargo build  $P $feat --target wasm32v1-none
  RUSTFLAGS="-C target-feature=+avx2" cargo build $P $feat
  cargo clippy $P $feat -- -D warnings
  cargo test   $P $feat
done
```

`cargo test` without `alloc` runs zero tests. It only checks that the test
build compiles (§1).

And a `rust-nostd-aarch64` attribute. It uses a second crane instance
built from the cross package set:

```nix
craneAarch64 = pkgs.pkgsCross.aarch64-multiplatform.callPackage cranePath {
  pkgs = pkgs.pkgsCross.aarch64-multiplatform;
};
```

where `cranePath` is the existing `fetchgit` of crane (factor it out of the
current `crane = …` binding). It runs `craneAarch64.cargoBuild` with
`cargoArtifacts = null` (no dependency to build) and
`cargoExtraArgs = "--locked -p base91-rs --lib --no-default-features"`,
once without and once with `--features alloc`. crane passes the target
itself, so there is no `--target` flag.

Add matching steps to `.github/workflows/rust.yml`:

```yaml
      - name: cargo no_std
        run: nix-build default.nix -A rust-nostd

      - name: cargo no_std (aarch64)
        run: nix-build default.nix -A rust-nostd-aarch64
```

**Python wheel check** — in the existing Python CI, assert that the wheel
contains `pybase91/pybase91.cpython-*.so`, to guard against the silent
bin-wheel failure mode.

`thumbv7em-none-eabihf`, `x86_64-unknown-none` and
`aarch64-unknown-none[-softfloat]` are not in CI. The crate has no
arch-specific code for 32-bit ARM, and the x86_64 soft-float behavior was
checked once while writing this spec (§4).

### 8 — Release

Bump `base91-rs` and `pybase91` to `0.2.3`. Do not publish 0.2.3 until
specs 0005 and 0006 are also implemented. Implementing this spec does not
depend on either of them.

## Alternatives considered

- **Atomic cache (`AtomicU8`) instead of `OnceLock`.** It replaces the
  cache but not `is_x86_feature_detected!`. Runtime detection under
  `no_std` would then need hand-written `cpuid` + `xgetbv` (for AVX OS
  support), which is error-prone, and there's no portable equivalent on
  aarch64. The risk is concentrated in custom targets that enable SSE but
  not AVX state: `cpuid` reports that the CPU supports AVX2, but the OS has
  not enabled AVX in `XCR0`, so AVX2 instructions fault with `#UD`. Getting
  the `xgetbv` check right is exactly the error-prone part. (On stock
  soft-float targets such as `x86_64-unknown-none`, runtime detection would
  not be unsafe, only slow: the compiler lowers the "AVX2" kernel to scalar
  code, see §4.) Compile-time selection avoids the question: it trusts what
  the target, and therefore the environment, declares.
- **`once_cell` / `spin` crates.** Same limitations as the atomic cache, and
  they add a dependency to a crate that has none (and to the nixpkgs
  packaging of spec 0003). `once_cell::sync` needs `critical-section` under
  `no_std`.
- **Making `no_std` imply `alloc`** (no `alloc` feature). Simpler now, but
  spec 0010 would then have to add an `alloc` feature later, and that is a
  breaking change for `default-features = false` users of the `Vec` API.
- **Moving the Python bindings to a separate crate** (instead of §6). Also
  fixes the `cdylib` problem, but it is a larger restructuring (workspace,
  nixpkgs derivation, PyPI workflow). §6 is enough.
- **rustup-based CI job, or `rust-overlay` / `fenix`** for bare-metal
  targets. rustup is impure. The overlays are pure but add a Nix input;
  `wasm32v1-none` + `pkgsCross` give enough coverage without one.

## Open questions

None. (The aarch64 CI build time and feasibility were verified; see §7.)

## Verification

```bash
cd rust/base91
for feat in "" "--features alloc"; do
  cargo build --lib --no-default-features $feat                          # 0 errors, 0 warnings
  cargo build --lib --no-default-features $feat --target wasm32v1-none   # 0 errors, 0 warnings
done
cargo test --lib --no-default-features --features alloc                  # pass
cargo test                                                               # pass, unchanged
maturin build --release                                                  # wheel contains pybase91/*.so
nix-build ../../default.nix -A rust-nostd -A rust-nostd-aarch64          # pass
```

Downstream check: a scratch `std` binary with
`base91-rs = { path = "…", default-features = false, features = ["alloc"] }`
builds and runs on the host.

`reuse lint` must pass.
