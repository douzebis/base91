# nixpkgs staging

The files under `pkgs/` are base91's nixpkgs packages, laid out as in the
nixpkgs tree:

| Here | In nixpkgs |
|---|---|
| `pkgs/by-name/ba/base91/package.nix` | same path |
| `pkgs/development/python-modules/pybase91/default.nix` | same path |

They are the single recipe for the release builds (spec 0012): the
repository's `default.nix` builds them from the local source
(`nix-build -A base91`, `nix-build -A pybase91`), so CI tests exactly what a
nixpkgs PR submits.  Locally, `src` is the repository restricted to `rust/`
and `cargoDeps` comes from `rust/Cargo.lock` (`importCargoLock`): the staged
`hash` and `cargoHash` are only used in nixpkgs.

Keep them in nixpkgs style (nixfmt, no SPDX header: their licensing is
declared in `REUSE.toml`), and copy them into nixpkgs verbatim.
`nix-build -A nixpkgs-staging-check` checks the formatting and the version:
between releases, `rust/Cargo.toml` has the next version with a `-dev`
suffix (e.g. `0.3.2-dev`) and the staged `version` is the one nixpkgs ships,
which must be older; for a release, both are equal.

pybase91 takes `version`, `src` and `cargoDeps` from `base91`: one
`nix-update base91` (or the r-ryantm bot) updates both.  It is registered in
`pkgs/top-level/python-packages.nix`, in alphabetical order, with:

```nix
  pybase91 = callPackage ../development/python-modules/pybase91 { };
```

The PR notes live next to this file: `pr-description-<package>.md` (the
description, from the nixpkgs PR template) and `pr-briefing-<package>.md`
(what a reviewer may ask, and the answers).

## Releasing

1. In `rust/Cargo.toml`, drop the `-dev` suffix from
   `workspace.package.version` and from the `base91` version under
   `workspace.dependencies`; run `cargo update -w`.  Rename `[Unreleased]`
   in `CHANGELOG.md`.  Set the same `version` in
   `pkgs/by-name/ba/base91/package.nix` (the check above requires it).
2. Commit, tag `vX.Y.Z`, push the tag.  The tag publishes pybase91 to PyPI
   (`.github/workflows/pypi.yml`): the version must be final.
3. nixpkgs, in a checkout of `master`:
   - copy the staged files (and, the first time, add the
     `python-packages.nix` line above);
   - `nix-update base91` (sets `version`, `hash` and `cargoHash`);
   - build `base91` and `python3Packages.pybase91`, run `nixpkgs-review`;
   - open or update the PR (`base91: A.B.C -> X.Y.Z`).  The description
     follows the nixpkgs template, with an AI disclosure section; the
     commit carries an `Assisted-by:` trailer (nixpkgs `CONTRIBUTING.md`).
     Before the PR is marked ready, write its briefing.
4. Copy the updated `package.nix` back here (with its new `hash` and
   `cargoHash`), and commit.
5. Start the next cycle: set `rust/Cargo.toml` to the next version with a
   `-dev` suffix (both lines), `cargo update -w`, add an `[Unreleased]`
   section to `CHANGELOG.md`, commit.

## The PRs (spec 0012 §8)

1. `base91: 0.1.0 -> X.Y.Z, rewrite in Rust`: `package.nix` only.  It
   replaces the C 0.1.0 package (NixOS/nixpkgs#409224); done in
   NixOS/nixpkgs#504523, force-pushed and retitled.
2. `python3Packages.pybase91: init at X.Y.Z`, after PR 1 is merged: the
   python module and the `python-packages.nix` line.  Done in
   NixOS/nixpkgs#504526, force-pushed and retitled; ask its reviewer to look
   again.

Afterwards, updates touch `pkgs/by-name/ba/base91/` only.
