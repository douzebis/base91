<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0012 — Build with the nixpkgs recipes, and get them into nixpkgs

**Status:** draft
**App:** base91
**Implemented in:** <!-- YYYY-MM-DD, fill after implementation -->

## Problem

nixpkgs ships a `base91` that is not this project's current code, and the
recipes meant to replace it are written and tested nowhere but in two
stale pull requests.

**What nixpkgs has (checked 2026-10-11):**

- `pkgs/by-name/ba/base91/package.nix`: **0.1.0, the C implementation**
  (`make -C src`), from NixOS/nixpkgs#409224 (merged May 2025). It installs
  `base91`, `b91enc` and `b91dec`. No other package depends on it.
- No `python3Packages.pybase91`.

**What is pending, both opened 2026-03-28, labeled stale, last touched in
April:**

| PR | Title | Branch content | State |
|---|---|---|---|
| NixOS/nixpkgs#504523 | `base91: init at 0.2.2` (wrong: it is an update) | the Rust CLI at v0.2.2 | no human review; "needs: reviewer" |
| NixOS/nixpkgs#504526 | `python3Packages.pybase91: init at 0.2.1` (branch: 0.2.2) | the PyO3 extension at v0.2.2 | approved twice (Sigmanificient), never merged |

0.2.2 is the version with the `simd::*_unchecked` soundness bug (spec
0005); both PRs must move to a 0.3.x release before anything is merged.

**Defects of the PR recipes**, none of which our CI can see, because it
never builds them:

- The CLI recipe sets `cargoExtraArgs = "-p base91-cli"`, a Crane
  attribute that `buildRustPackage` ignores: it builds and tests the whole
  workspace (the fuzz crate and the test kit included).
- It finds the man page and the completions with
  `find . -path "*/build/base91-cli-*/out"`: `build.rs` writes them to
  Cargo's hashed `OUT_DIR`. yb's reviewers objected to the same kind of
  `find` (yb spec 0029, NixOS/nixpkgs#514826).
- No `versionCheckHook`, no `updateScript`.
- pybase91 runs none of its Python tests, and its `meta.license` says
  BSD-3-Clause and MIT, while the crate and `pyproject.toml` declare MIT.
- Each fetches the source and vendors the dependencies on its own: two
  `hash`/`cargoHash` pairs to keep equal.

**Our own build** (`default.nix`) duplicates them with Crane and a hand
written `buildPythonPackage`:

- `base91` is a Crane `buildPackage` with the same `find` in `target/`;
  CI never builds it (no workflow runs `-A base91`).
- `pybase91` vendors its dependencies with `fetchCargoVendor` and a pinned
  hash, which must be recomputed by hand after every `Cargo.lock` change,
  and which a cached build does not reveal when stale (it shipped stale
  once, `c27dbc3`).
- The version is written in nine places: `rust/base91/Cargo.toml`,
  `rust/base91-cli/Cargo.toml` (twice), `rust/base91/pyproject.toml`, and
  five literals in `default.nix`.
- Crane is loaded with `callPackage (fetchgit …)`, an import from
  derivation: `nix-instantiate -A rust-tests --option
  allow-import-from-derivation false` fails on `crane-80ceeec.drv`.
- `base91.nix` (the C 0.1.0 recipe, "preserved for nixpkgs compat") and
  the `base91-c` attribute mirror what nixpkgs ships today.
- The nixpkgs pin dates from 2026-02-07, eight months behind the master
  a PR is built against.

The sibling projects solved the same problem: yb (spec 0029: the staged
recipe is the release build, CI builds it) and prototools (specs 0402,
0405: a staged recipe built by CI, one small PR at a time).

## Rationale: Crane and rustPlatform

Crane stays for the development checks. They share one dependency build
(`buildDepsOnly`): fmt, clippy, the test suites with the C reference, the
aarch64 tests under qemu, `no_std` on two targets, fuzzing, coverage,
the performance test. With `buildRustPackage`, each would rebuild every
dependency.

nixpkgs accepts only `rustPlatform`. So, as in yb: the staged nixpkgs
recipes become the single recipe for the packages (`base91`, `pybase91`),
built from the local source; Crane keeps the checks.

## Goals

- **G1.** `nix-build -A base91` and `nix-build -A pybase91` build the
  staged nixpkgs recipes (`nixpkgs/pkgs/…`), from the local source.
- **G2.** CI builds exactly what a nixpkgs PR submits, and checks the
  staged files' formatting and version.
- **G3.** The recipes are plain: no `find`, no `OUT_DIR`, no
  hand-maintained vendor hash in our build; `versionCheckHook`,
  `updateScript`, tests run in the check phase.
- **G4.** One version, in `rust/Cargo.toml`; between releases, the next
  version with a `-dev` suffix.
- **G5.** Evaluation needs no build (no IFD).
- **G6.** A documented release and PR procedure, and an order for the
  nixpkgs PRs, each small enough to review in one sitting.

## Non-goals

- **N1.** Dropping Crane (see *Rationale*).
- **N2.** Packaging the C library (`base91-clib`) or the Go module for
  nixpkgs. Go modules are not packaged as libraries in nixpkgs; the C
  library has no user there.
- **N3.** Publishing to crates.io (base91-rs and base91-cli are still at
  0.2.2 there). nixpkgs builds from the GitHub tag. Automating crates.io
  as yb does (yb spec 0029 §9) is a separate spec.
- **N4.** A flake, or an overlay for users.
- **N5.** Moving the Cargo workspace from `rust/` to the repository root.

## Specification

### 1. Staged nixpkgs files

- New directory `nixpkgs/`, laid out as the nixpkgs tree:

  | Here | In nixpkgs |
  |---|---|
  | `nixpkgs/pkgs/by-name/ba/base91/package.nix` | same path |
  | `nixpkgs/pkgs/development/python-modules/pybase91/default.nix` | same path |

  The `python-packages.nix` line (`pybase91 = callPackage
  ../development/python-modules/pybase91 { };`) is recorded in
  `nixpkgs/README.md`, not staged as a file.
- Their starting point is the PR branches (#504523 and #504526 heads),
  then edited as below.
- nixfmt-formatted, no SPDX header (they are copied into nixpkgs
  verbatim); their licensing goes in `REUSE.toml`, as in yb.
- `nixpkgs/README.md`: what the directory is, the staging check, the
  release and PR procedure (§7). The PR notes live next to it
  (`pr-description-<package>.md`, `pr-briefing-<package>.md`).

### 2. Upstream changes that make the recipes plain

- **Man page from the binary.** `base91` gets a hidden `--man` option that
  prints the roff page to standard output, next to the existing
  `--completions SHELL`. `build.rs` and the `clap_mangen` build
  dependency go (it moves to a regular dependency). The recipe installs
  both from the installed binary, guarded for cross builds:

  ```nix
  postInstall = ''
    ln -s base91 $out/bin/b91enc
    ln -s base91 $out/bin/b91dec
  '' + lib.optionalString (stdenv.buildPlatform.canExecute stdenv.hostPlatform) ''
    $out/bin/base91 --man > base91.1
    installManPage base91.1
    installShellCompletion --cmd base91 \
      --bash <($out/bin/base91 --completions bash) \
      --zsh <($out/bin/base91 --completions zsh) \
      --fish <($out/bin/base91 --completions fish)
  '';
  ```

  The page must not depend on the build environment (date, locale):
  checked by the test plan.
- **One Python version source.** `pyproject.toml` declares
  `dynamic = ["version"]`; maturin takes the crate's version.
- **The stub check leaves the recipe.** The `preBuild` that regenerates
  `pybase91.pyi` and compares it with the committed file is a check of
  ours, not packaging: it becomes a Crane check derivation
  (`pybase91-stub-check`, in CI), and the staged recipe ships the
  committed stub as is.

### 3. The recipes

**`base91`** (by-name), with yb's shape:

```nix
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "base91";
  version = "…";
  src = fetchFromGitHub {
    owner = "douzebis";
    repo = "base91";
    tag = "v${finalAttrs.version}";
    hash = "…";
  };
  cargoRoot = "rust";
  buildAndTestSubdir = "rust/base91-cli";
  cargoHash = "…";
  nativeBuildInputs = [ installShellFiles ];
  postInstall = …;                         # §2
  nativeInstallCheckInputs = [ versionCheckHook ];
  doInstallCheck = true;
  passthru.updateScript = nix-update-script { };
  meta = { description, homepage, changelog, license = lib.licenses.mit,
           maintainers = [ douzebis ], mainProgram = "base91", platforms = unix };
})
```

- `cargoRoot` + `buildAndTestSubdir` instead of `sourceRoot`: it works
  whatever the source's store name, so the local override (§4) needs no
  renaming. The check phase runs the CLI crate's tests.

**`pybase91`** (python-modules): `buildPythonPackage`, `pyproject = true`,
`rustPlatform.cargoSetupHook` + `rustPlatform.maturinBuildHook`,
`pytestCheckHook` on `rust/base91/python-tests`, `pythonImportsCheck`.
It fetches nothing of its own:

```nix
inherit (base91) version src cargoDeps;
cargoRoot = "rust";
buildAndTestSubdir = "rust/base91";
```

One `nix-update base91` (or the r-ryantm bot) moves both packages, and in
nixpkgs they read as one project. `meta.license` is MIT, as the package
declares.

### 4. Local build from the staged recipes

`default.nix` builds both through an overlay, so that `pybase91` sees the
local `base91`:

```nix
pkgsLocal = pkgs.extend (final: prev: {
  base91 = (final.callPackage ./nixpkgs/pkgs/by-name/ba/base91/package.nix { })
    .overrideAttrs (old: {
      version = cargoVersion;                  # §5
      src = localSource;                       # filtered ./. : rust/ only
      cargoDeps = final.rustPlatform.importCargoLock {
        lockFile = ./rust/Cargo.lock;
      };
    });
  pythonPackagesExtensions = prev.pythonPackagesExtensions ++ [
    (pyFinal: pyPrev: {
      pybase91 = pyFinal.callPackage
        ./nixpkgs/pkgs/development/python-modules/pybase91/default.nix { };
    })
  ];
});
```

- `importCargoLock` reads `Cargo.lock`: no hash to update when
  dependencies change (the workspace has no git dependencies). The
  pybase91 vendor hash, and the manual procedure it needs, disappear from
  our build.
- `localSource` keeps the layout of the GitHub tarball (`rust/` under the
  root), restricted to what the build reads.

Attributes:

| Attribute | Recipe |
|---|---|
| `base91` (and `default`) | staged `package.nix`, local source |
| `pybase91` | staged python module, `pkgsLocal.python3Packages`, local source |
| `nixpkgs-staging-check` | nixfmt and version of the staged files (§6) |
| `pybase91-stub-check` | the committed stub matches the generator (§2) |
| `rust-*`, `go-tests`, `base91-clib`, `dev-shell` | unchanged |

Removed: the Crane `base91Rust`, the hand-written `pybase91`,
`base91.nix` and `base91-c` (the C 0.1.0 recipe; nixpkgs moves away from
it in PR 1, §8; the C reference stays built and tested as `base91-clib`).

**No IFD.** Crane is fetched at evaluation time
(`builtins.fetchTarball { url; sha256; }`), and CI evaluates with
`--option allow-import-from-derivation false`.

**The pin:** all of `default.nix` moves to `nixos-unstable`, close to
the master the PRs are built against (as yb did), rather than a second,
recent nixpkgs for the staged builds alone, which would mean two
toolchains to reason about. Consequences, handled in the same change:
rustc moves past 1.91 (clippy 1.95 already flags `manual_checked_ops` in
`EncodeCore::encode_len`), and the `perf_sanity` thresholds, calibrated
with 1.91, are re-measured.

### 5. One version

- `rust/Cargo.toml` gains `[workspace.package] version` and
  `[workspace.dependencies] base91-rs = { path = "base91", version = "…" }`;
  `base91-rs` and `base91-cli` take `version.workspace = true`, and the
  CLI `base91-rs.workspace = true` (keeping its `default-features` and
  `features`). `base91-testkit` and `base91-fuzz` keep their own 0.0.0
  (never published).
- `pyproject.toml`: dynamic version (§2).
- `default.nix` reads it: `cargoVersion = (lib.importTOML
  ./rust/Cargo.toml).workspace.package.version`.
- Between releases, `main` carries the next version with `-dev` (the
  first is `0.3.2-dev`: `main` already has unreleased changes).
- The staged recipe keeps a literal `version`, which `nix-update` edits.

### 6. CI

- `rust.yml`: unchanged Crane jobs, plus `-A base91`.
- `python.yml`: `-A pybase91` (now the staged recipe) and
  `-A pybase91-stub-check`.
- A `packaging` step: `-A nixpkgs-staging-check`:
  - `nixfmt --check` on the staged files, as nixpkgs CI requires;
  - the version rule (yb's): the staged `version` equals the Cargo version
    at a release, and is older while `main` carries `-dev`.
- Evaluation without IFD (§4), first and in seconds.

### 7. Release and nixpkgs PR procedure

In `nixpkgs/README.md`:

1. In `rust/Cargo.toml`, drop the `-dev` suffix (both lines);
   `cargo update -w`; name the `[Unreleased]` section of `CHANGELOG.md`;
   set the same `version` in the staged `package.nix`.
2. Commit, tag `vX.Y.Z`, push the tag. The tag publishes pybase91 to PyPI
   (`pypi.yml`), so the version must be final.
3. In a checkout of nixpkgs `master`: copy the staged files;
   `nix-update base91` (`version`, `hash`, `cargoHash`); build `base91`
   and `python3Packages.pybase91`; `nixpkgs-review`; open or update the
   PR.
4. Copy the updated `package.nix` back (hashes included); commit.
5. Next cycle: the next version with `-dev`, an `[Unreleased]` section.

PR descriptions follow the nixpkgs template, with an AI disclosure
section and an `Assisted-by:` trailer on the commit (nixpkgs
automation/AI policy). Before a PR is marked ready, a briefing
(`pr-briefing-<package>.md`) prepares the maintainer to answer every
review question first-hand.

### 8. The PRs

The release they package is the first one with §2 (0.3.2): 0.3.1's man
page exists only in `OUT_DIR`.

- **PR 1: `base91: 0.1.0 -> 0.3.2, rewrite in Rust`** (yb's precedent:
  NixOS/nixpkgs#514826, `yb: 0.1.0 -> 0.4.2, rewrite in Rust`).
  - One file: `pkgs/by-name/ba/base91/package.nix`. Under `by-name` only,
    so the maintainer can merge it with the merge bot once approved, and
    later updates can come from the r-ryantm bot.
  - The description says what changes for users: the same three
    commands (`base91`, `b91enc`, `b91dec`), the same Henke format and
    defaults; new: a man page, completions, the `--simd` format. Any CLI
    difference from the C 0.1.0 tool is listed (checked against the C
    reference by the CLI comparison tests, spec 0009 §9).
  - Done in #504523, updated in place: force-push its branch, retitle
    it. This keeps its review history (a new PR closing it with a link is
    what prototools did, for a PR whose reviewer had asked for a split).
- **PR 2: `python3Packages.pybase91: init at 0.3.2`**, after PR 1 is
  merged (it inherits `base91`'s source).
  - Two files: the python module and the `python-packages.nix` line. That
    line is outside `by-name`, so a committer must merge it.
  - A library with no dependent in nixpkgs: `pkgs/README.md` asks for a
    user or tests. The check phase runs its Python tests and
    `pythonImportsCheck`; the briefing prepares the "who uses it?" answer
    (direct Python use; it is on PyPI).
  - Done in #504526, updated in place (it keeps its two approvals, given
    on an older recipe): force-push, retitle, and ask its reviewer to look
    again.
- **Afterwards:** updates touch `pkgs/by-name/ba/base91/` only; pybase91
  follows through `inherit`.

## Tests

- `nix-build -A base91`: the CLI's tests in the check phase,
  `versionCheckHook` (`base91 --version` prints the Cargo version), the
  three commands, the man page and the bash, zsh and fish completions in
  `result/`.
- `nix-build -A pybase91`: `pythonImportsCheck` and the Python tests pass.
- `nix-build -A nixpkgs-staging-check` passes, and fails on a
  deliberately unformatted staged file and on a staged version newer than
  the Cargo version.
- `nix-instantiate` of every attribute succeeds with `--option
  allow-import-from-derivation false`.
- `base91 --man` gives the same bytes under two `SOURCE_DATE_EPOCH`
  values and two locales.
- A cross evaluation (`pkgsCross.aarch64-multiplatform`) of `base91` gets
  past `postInstall` (evaluated, not built).
- In a nixpkgs `master` checkout, the staged files build with `hash` and
  `cargoHash` instead of the local override, and `nixpkgs-review` passes
  (dry run of §7 before opening PR 1).
- The Crane checks (`rust-*`) pass after the pin moves, with
  `perf_sanity` re-calibrated.

## Open questions

None. Resolved (2026-10-11):

- The existing PRs are updated in place, force-pushed and retitled (§8).
- All of `default.nix` moves to `nixos-unstable` (§4).

## References

- [spec 0003](0003-nixpkgs-pybase91.md) — the pybase91 PR's first review
  (the `Cargo.lock` build failure)
- [spec 0004](0004-no-std-build.md) §6 — maturin's bindings detection
- [spec 0005](0005-simd-unchecked-soundness.md) — why 0.2.2 must not be
  merged
- [spec 0009](0009-test-strategy.md) — the test suites and the CLI
  comparison with the C reference
- `docs/publishing.md` — registries; its nixpkgs section is superseded by
  this spec
- NixOS/nixpkgs#409224 (base91 0.1.0, merged), #504523, #504526 (open)
- yb spec 0029 (`../yb/docs/specs/0029-nixpkgs-aligned-build.md`) and
  `../yb/nixpkgs/README.md` — the model: staged recipe as the release
  build, version rule, procedure, briefings
- prototools specs 0402 and 0405 (`../oss-prototools/docs/specs/`) — no
  IFD, `passthru` sharing of `src`/`cargoDeps`, one small PR at a time
- nixpkgs: `pkgs/README.md`, `CONTRIBUTING.md` (automation/AI policy),
  the Rust and Python sections of the manual
