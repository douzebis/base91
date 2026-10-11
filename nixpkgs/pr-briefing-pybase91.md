# Briefing: nixpkgs PR `python3Packages.pybase91: init at 0.3.2`

Preparation notes for the maintainer before marking NixOS/nixpkgs#504526
ready, so that reviewer questions can be answered first-hand (nixpkgs
automation/AI policy).

## The stack

The PR followed #504523 (`base91: 0.1.0 -> 0.3.2`): pybase91 takes
`base91`'s `version`, `src` and `cargoDeps`, which master's `base91` had
only once that PR was merged (2026-10-11, merge bot, approved by
Sigmanificient).  Until then the branch carried both commits; it is now
rebased onto master and holds only `python3Packages.pybase91: init at
0.3.2`.

## What it adds (2 files)

### `pkgs/development/python-modules/pybase91/default.nix`

- **`buildPythonPackage (finalAttrs: …)`, `pyproject = true`**: a PEP 517
  build; the backend is maturin, declared in `rust/base91/pyproject.toml`.
- **`inherit (base91) version src cargoDeps`**: the same tarball and the
  same vendored crates as the CLI.  Consequences: no `hash` or
  `cargoHash` here; `nix-update base91` (or r-ryantm) updates both
  packages at once; the two cannot drift to different versions.  The
  vendored crates include pyo3 because `cargoDeps` vendors the whole
  workspace's `Cargo.lock`.
- **`cargoRoot = "rust"`, `buildAndTestSubdir = "rust/base91"`**: the
  workspace is in `rust/`; the extension is the `base91-rs` crate in
  `rust/base91`, where maturin is run.
- **`rustPlatform.cargoSetupHook`**: points cargo at the vendored crates
  (offline build).  **`rustPlatform.maturinBuildHook`**: runs `maturin
  build` and puts the wheel in `dist/` for the install phase.
- maturin's settings are in `pyproject.toml` (`[tool.maturin]`):
  `bindings = "pyo3"`, `features = ["python"]` (the PyO3 code is behind
  that Cargo feature), `module-name = "pybase91.pybase91"`.  The
  `bindings` line matters: without it maturin can pick the crate's helper
  binary and build a wheel without the extension; `pythonImportsCheck`
  would catch that.
- **`nativeCheckInputs = [ pytestCheckHook ]`, `enabledTestPaths =
  [ "rust/base91/python-tests" ]`**: the package's Python tests (the
  streaming decoder against one-shot decoding, for every chunk size; two
  tests).  They import the installed package.
- **`pythonImportsCheck = [ "pybase91" ]`**: imports the module, which
  loads the compiled extension.
- **`passthru.tests.matches-cli`**: the "minimal test program"
  `pkgs/README.md` asks for when a library has no dependent.  A
  `runCommand` with `base91` and a Python environment holding this very
  package (`finalAttrs.finalPackage`, so it also tests overridden
  versions) encodes 10,502 fixed bytes with `pybase91.encode` and with
  `b91enc`, requires identical output, and decodes it back.  Run it with
  `nix-build -A python3Packages.pybase91.tests.matches-cli`.  (Checked to
  fail when the two outputs differ.)
- **`__structuredAttrs = true`**: as in `base91`.
- **`meta.changelog`** uses `finalAttrs.src.tag`, as `base91` does
  (`src` is `base91`'s).
- **`meta`**: MIT (what the crate and `pyproject.toml` declare; the old
  revision said BSD-3-Clause and MIT, wrongly), `changelog` at the tag,
  maintainer `douzebis`.  No `mainProgram`: no executable.

### `pkgs/top-level/python-packages.nix`

One line, alphabetical, between `pybase64` and `pybbox`.  This file is
outside `pkgs/by-name`, so the merge bot cannot merge the PR: a committer
has to.

## Likely questions

- **Who uses it? (`pkgs/README.md`: "Library packages should have at
  least one dependent … If it is not possible to package the dependent, a
  minimal test program should be added to `passthru.tests`.")**  No
  package in nixpkgs depends on it: it is for direct use from Python, and
  is on PyPI as `pybase91`.  The check phase runs its tests and imports the
  extension, and `passthru.tests.matches-cli` is the minimal test program
  the README asks for in that case (above).
- **Why not in `pkgs/by-name`?**  Python libraries live in
  `pkgs/development/python-modules` and are registered in
  `python-packages.nix`, for every Python version.
- **Why does it depend on the CLI package?**  Only for its source and
  vendored crates (one fetch, one hash).  At run time it needs nothing
  from `base91`.
- **The previous revision was approved twice; why change it?**  It
  packaged 0.2.2, whose Rust library has a soundness bug in two
  functions (`simd::encode_unchecked` / `decode_unchecked`, fixed in
  0.2.3; the Python bindings never called them).  0.3.2 also reuses
  `base91`'s source instead of fetching it again, runs the tests, and fixes
  the license.
- **Which Python versions?**  Built and tested with
  `python311Packages` to `python314Packages` (`python3` is 3.14 in master
  since 2026-05-30).  `pyproject.toml` requires Python 3.11 or later.
  `python315Packages.pybase91` (3.15 is a pre-release in nixpkgs) fails:
  PyO3 0.26 supports up to 3.14 ("the configured Python interpreter
  version (3.15) is newer than PyO3's maximum supported version").  Fixed
  upstream by moving to a newer PyO3 when 3.15 support is needed; not
  something to work around in the recipe.
- **The SIMD format?**  Not exposed in Python yet; the bindings cover
  Henke's format only.

## Before marking ready

- (Done: #504523 merged; branch rebased onto master; description
  updated.)
- Read the diff and this file.
- PR CI green.
