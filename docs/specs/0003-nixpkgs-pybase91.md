<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0003 — Publish `pybase91` to nixpkgs

**Status:** draft
**App:** base91
**Implemented in:** —

## Problem

The nixpkgs PR #504526 (`pybase91-init`) has two blocking issues and several
style issues identified in the review:

1. **Build fails** — `cargoSetupHook` always looks for `Cargo.lock` at the
   root of the unpacked source (`/build/source/Cargo.lock`). The current
   derivation passes `sourceRoot = "source/rust"` to `fetchCargoVendor` but
   the hook still validates against the top-level path, so the build fails.
2. **Package not accessible** — no entry in
   `pkgs/top-level/python-packages.nix`.

Style issues (required by nixpkgs conventions):
- `rec` must be replaced by the `finalAttrs` pattern (nixpkgs #315337).
- `pythonOlder` / `disabled` guard is unnecessary (nixpkgs ships no Python
  older than 3.1).
- `tag` should reference `finalAttrs.version`.

Non-blocking:
- The upstream GitHub repo has no top-level `LICENSE` file; GitHub cannot
  auto-detect the license from the REUSE layout.

## Goals

- Fix the nixpkgs derivation so that `nix-build` succeeds.
- Satisfy all reviewer style requirements.
- Add the `python-packages.nix` entry.
- Add a top-level `LICENSE` file to this repo.

## Non-goals

- Adding smoke tests to the nixpkgs derivation (the existing `src/test/test.sh`
  tests the C CLI binaries, not the Python package; the Rust test suite already
  runs as part of `cargo test` during the maturin build).
- Packaging `base91` (the CLI) for nixpkgs in this PR.

## Background: repo layout

```
rust/               ← Cargo workspace root (contains Cargo.lock)
  Cargo.toml        ← workspace manifest (members: base91, base91-cli)
  Cargo.lock
  base91/           ← library + PyO3 bindings
    pyproject.toml  ← maturin build config; module-name = pybase91.pybase91
    src/
  base91-cli/
src/                ← C reference implementation (unrelated to Python package)
```

The Python package (`pybase91`) lives in `rust/base91/`. The Cargo workspace
root — and therefore `Cargo.lock` — is `rust/`.

## Idiomatic nixpkgs pattern

The canonical pattern for a maturin package whose `pyproject.toml` is not at
the repo root is (see `kanalizer`, `hf-xet` for live examples):

```nix
buildPythonPackage (finalAttrs: {
  ...
  src = fetchFromGitHub { ... };

  sourceRoot = "${finalAttrs.src.name}/rust";   # ← points at Cargo.lock

  cargoDeps = rustPlatform.fetchCargoVendor {
    inherit (finalAttrs) pname version src sourceRoot;
    hash = "...";
  };

  buildAndTestSubdir = "base91";   # ← relative to sourceRoot

  nativeBuildInputs = [
    rustPlatform.cargoSetupHook
    rustPlatform.maturinBuildHook
  ];
  ...
})
```

Setting `sourceRoot = "${finalAttrs.src.name}/rust"` makes `cargoSetupHook`
find `Cargo.lock` at `rust/Cargo.lock` (which is where the workspace root is).
`buildAndTestSubdir = "base91"` then tells maturin which sub-crate to build,
relative to `sourceRoot`. Passing `sourceRoot` to `fetchCargoVendor` ensures
the vendored snapshot is computed from the same subdirectory.

## Specification

### 1 — nixpkgs: update `default.nix`

File: `pkgs/development/python-modules/pybase91/default.nix`

- Switch from `buildPythonPackage rec {` to `buildPythonPackage (finalAttrs: {`.
- Remove `pythonOlder` from imports and the `disabled = pythonOlder "3.11";`
  line.
- Change `rev = "v${version}";` to `tag = "v${finalAttrs.version}";`.
- Add `sourceRoot = "${finalAttrs.src.name}/rust";`.
- Update `cargoDeps` to `inherit (finalAttrs) pname version src sourceRoot;`
  (drop the explicit `sourceRoot = "source/rust"` override — it is now
  inherited).
- Keep `buildAndTestSubdir = "base91";`.
- Move `cargo` and `rustc` from top-level inputs to `nativeBuildInputs`
  (they were already listed there; remove the duplicate top-level declarations
  if present).
- Close the expression with `})` instead of `}`.
- Recompute `cargoDeps.hash` after the `sourceRoot` change.
- Update `version` to the current release (`0.2.2`); recompute `src.hash` and
  `cargoDeps.hash` accordingly.

### 2 — nixpkgs: add entry to `python-packages.nix`

File: `pkgs/top-level/python-packages.nix`

Insert in alphabetical order, after `pybase64`:

```nix
  pybase91 = callPackage ../development/python-modules/pybase91 { };
```

### 3 — this repo: add `LICENSE` file

Add a plain `LICENSE` file at the repository root containing the MIT license
text (identical to `LICENSES/MIT.txt`).  This is a human-facing convenience
file; it does not conflict with the REUSE layout.

Use `reuse annotate` with `--force-dot-license` (or add an `[[annotations]]`
block in `REUSE.toml`) to keep `reuse lint` passing.

## Verification

```bash
# In a nixpkgs checkout on the PR branch:
nix-build -A python3Packages.pybase91

# Quick smoke test:
result/bin/python -c "import pybase91; print(pybase91.encode(b'hello'))"
```

`reuse lint` must pass on the base91 repo after adding `LICENSE`.
