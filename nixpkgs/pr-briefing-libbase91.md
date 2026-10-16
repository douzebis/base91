# Briefing: nixpkgs PR `libbase91: init at 0.3.3`

Preparation notes for the maintainer before marking NixOS/nixpkgs#504524
ready, so that reviewer questions can be answered first-hand (nixpkgs
automation/AI policy).

## The stack

The PR needs `base91` 0.3.3 in master: libbase91 takes `base91`'s
`version`, `src` and `cargoDeps`, and the crate it builds,
`rust/base91-capi`, exists from 0.3.3 on.  The `base91` update is left to
r-ryantm (a test of merging the bot's PR with the merge bot); if it does
not come within a week or two, we open it ourselves.  Until then, the
branch carries a `base91: 0.3.2 -> 0.3.3` commit so that it builds; then:
rebase onto master (the commit disappears), force-push, update the
description's first paragraph, mark ready.

## What it adds (1 file: `pkgs/by-name/li/libbase91/package.nix`)

- **`rustPlatform.buildRustPackage`, `buildAndTestSubdir =
  "rust/base91-capi"`**: a Rust crate with `crate-type = ["cdylib",
  "staticlib"]`.  The default cargo install hook copies `.so`, `.a` and
  `.dylib` files from the release directory into `$out/lib`, so the recipe
  has no build or install phase of its own.  The check phase runs the
  crate's 9 Rust tests (Henke's API against base91-rs on random splits,
  overlapping buffers as in Henke's tool, the new API, the struct layout).
- **`inherit (base91) version src cargoDeps`**: as `pybase91`.  Updating
  `base91` updates this package.
- **`outputs = [ "out" "dev" ]`**: the libraries in `out`, the header and
  the `.pc` file in `dev`, written to `$dev` explicitly.
- **`postInstall`**:
  - the shared library is installed under its SONAME, `libbase91.so.0`
    (set by the crate's `build.rs` on Linux), with `libbase91.so` linking
    to it for the linker;
  - `base91.h` into `$dev/include`;
  - `base91.pc` from the crate's template (`base91.pc.in`), with the
    store paths, the version and `Libs.private`.
- **`libsPrivate`**: the system libraries a static link of `libbase91.a`
  needs (it contains Rust's standard library), as `rustc --print
  native-static-libs` reports them: `-lgcc_s -lutil -lrt -lpthread -lm
  -ldl -lc` on Linux; `-lSystem -lc -lm` on Darwin (not tested on Darwin).
- **`fixDarwinDylibNames`** on Darwin: sets the dylib's install name to
  its store path.
- **`passthru.tests`**:
  - `pkg-config`: `testers.hasPkgConfigModules`, with
    `meta.pkgConfigModules = [ "base91" ]`;
  - `c-api`: `rust/base91-capi/tests/c/api.c`, built with `-Wall -Wextra
    -Werror` through `pkg-config`, linked dynamically and then statically
    (`pkg-config --static`); it calls both APIs and checks that the SIMD
    level is the CPU's best kernel (`__builtin_cpu_supports`);
  - `henke-compat`: Henke's basE91 0.6.0 `cli.c` (vendored unmodified in
    the source, `rust/base91-testkit/reference/`), compiled against our
    header and library, must write exactly what `base91` writes, for
    several options, including `-m 64` and `-m 1K`, where his tool
    overlaps its input and output buffers.
- **`meta.license = [ mit bsd3 ]`**: the library is MIT; the header
  carries Henke's declarations (BSD-3-Clause).

## Likely questions

- **Why a C library built from Rust?**  The SIMD variant's speed comes
  from base91-rs's kernels; the C API calls them with no overhead beyond
  a function call.  A program built against Henke's library gets the
  faster codec without recompiling.
- **Is it really binary compatible?**  `struct basE91` has Henke's layout
  (checked by a test), the five functions his signatures and contracts;
  the Rust tests compare every call's output with Henke's 0.6.0 code, and
  `henke-compat` runs his own tool against the library.
- **Who uses it?**  No package in nixpkgs yet: hence the `passthru.tests`
  (`pkgs/README.md` asks for a minimal test program).  Any C program using
  Henke's basE91 API can link it.
- **Does it conflict with anything?**  No other package provides
  `libbase91` or `base91.h` in nixpkgs.
- **Static library size**: `libbase91.a` is about 9 MB, Rust's standard
  library included; a static link keeps only what is used.
- **Panics**: no Rust panic crosses into C: each exported function aborts
  the process instead (unwinding into C is undefined behavior).
- **Why does mdaniels5757's suggestion no longer apply?**  It was for the
  0.2.2 recipe, which compiled Henke's C with a custom `buildPhase`; the
  library is now a cargo build, handled by the default hooks.

## Before marking ready

- `base91` 0.3.3 in master; branch rebased onto master; description's
  first paragraph updated.
- Read the diff and this file.
- PR CI green.
