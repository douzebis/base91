# nixpkgs staging

The files under `pkgs/` are base91's nixpkgs packages, laid out as in the
nixpkgs tree:

| Here | In nixpkgs |
|---|---|
| `pkgs/by-name/ba/base91/package.nix` | same path |
| `pkgs/development/python-modules/pybase91/default.nix` | same path |
| `pkgs/by-name/li/libbase91/package.nix` | same path |

They are the single recipe for the release builds (spec 0012): the
repository's `default.nix` builds them from the local source
(`nix-build -A base91`, `-A pybase91`, `-A libbase91`), so CI tests exactly what a
nixpkgs PR submits.  Locally, `src` is the repository restricted to `rust/`
and `cargoDeps` comes from `rust/Cargo.lock` (`importCargoLock`): the staged
`hash` and `cargoHash` are only used in nixpkgs.

Keep them in nixpkgs style (nixfmt, no SPDX header: their licensing is
declared in `REUSE.toml`), and copy them into nixpkgs verbatim.
`nix-build -A nixpkgs-staging-check` checks the formatting and the version:
between releases, `rust/Cargo.toml` has the next version with a `-dev`
suffix (e.g. `0.3.2-dev`) and the staged `version` is the one nixpkgs ships,
which must be older; for a release, both are equal.

pybase91 and libbase91, the satellites (spec 0014), take `version`, `src`
and `cargoDeps` from `base91`: one `nix-update base91` (or the r-ryantm bot)
updates all three.  pybase91 is registered in
`pkgs/top-level/python-packages.nix`, in alphabetical order (after
`pybase64`), with:

```nix
  pybase91 = callPackage ../development/python-modules/pybase91 { };
```

The PR notes live next to this file: `pr-description-<package>.md` (the
description, from the nixpkgs PR template) and `pr-briefing-<package>.md`
(what a reviewer may ask, and the answers).

## Releasing

1. In `rust/Cargo.toml`, drop the `-dev` suffix from
   `workspace.package.version` and from the `base91` and `base91-testkit`
   versions under `workspace.dependencies` (three lines); run
   `cargo update -w`.  Rename `[Unreleased]` in `CHANGELOG.md`.  Set the
   same `version` in `pkgs/by-name/ba/base91/package.nix` (the check above
   requires it).
2. Commit, tag `vX.Y.Z`, push `main` and the tag.  The tag starts
   `.github/workflows/publish.yaml` (spec 0013): it checks the tag against
   the Cargo version, packages and tests the crates (artifact `crates`) and
   builds the wheels (artifacts `wheels-*`), then waits for approval.
3. Inspect the artifacts, then approve, in the workflow run:
   - `Publish to crates.io` (environment `crates-io`): publishes
     `base91-testkit`, `base91-rs`, then `base91-cli`;
   - `Publish to PyPI` (environment `pypi`): publishes pybase91.

   If an upload fails halfway, re-run its job: what is already published
   is skipped.  A tag pushed by mistake: reject the approvals, delete the
   tag.
4. nixpkgs, in a checkout of `master`:
   - copy the staged files (and, the first time, add the
     `python-packages.nix` line above);
   - `nix-update base91` (sets `version`, `hash` and `cargoHash`);
   - build `base91`, `python3Packages.pybase91` and `libbase91` (with
     their `passthru.tests`), run `nixpkgs-review`;
   - open or update the PR (`base91: A.B.C -> X.Y.Z`).  The description
     follows the nixpkgs template, with an AI disclosure section; the
     commit carries an `Assisted-by:` trailer (nixpkgs `CONTRIBUTING.md`).
     Before the PR is marked ready, write its briefing.
5. Copy the updated `package.nix` back here (with its new `hash` and
   `cargoHash`), and commit.
6. Start the next cycle: set `rust/Cargo.toml` to the next version with a
   `-dev` suffix (the three lines), `cargo update -w`, add an
   `[Unreleased]` section to `CHANGELOG.md`, commit.

### First release with spec 0013 (0.3.2) only

crates.io trusted publishing cannot create a crate, and `base91-testkit`
is new.  Between steps 2 and 3: once the `crates` job has passed, from the
tagged checkout, with a personal crates.io API token scoped to publishing
new crates (revoked afterwards):

```sh
nix-shell default.nix -A release-shell \
  --run 'cargo publish --manifest-path rust/Cargo.toml -p base91-testkit --locked'
```

Then approve as in step 3 (`base91-testkit` is skipped), and add the
trusted publisher of `base91-testkit` on crates.io (below).

### One-time setup for publishing

- GitHub, repository settings, environments: `crates-io` (new) and `pypi`
  (existing), each with a required reviewer (the maintainer) and a
  deployment rule limited to tags `v*`.
- crates.io, for each of `base91-rs`, `base91-cli` and (after its first
  upload) `base91-testkit`: a trusted publisher with repository
  `douzebis/base91`, workflow `publish.yaml`, environment `crates-io`;
  then "trusted publishing only".
- PyPI, project `pybase91`, trusted publisher: workflow `publish.yaml`
  (was `pypi.yml`), environment `pypi`.

## The PRs

1. `base91: 0.1.0 -> 0.3.2` (NixOS/nixpkgs#504523): merged 2026-10-11.
2. `python3Packages.pybase91: init at 0.3.2` (NixOS/nixpkgs#504526): the
   python module and the `python-packages.nix` line.  A committer must
   merge it (outside `by-name`).
3. `libbase91: init at X.Y.Z` (NixOS/nixpkgs#504524, updated in place;
   spec 0014 §5): once `base91` X.Y.Z is in master, as it inherits from
   it; only `pkgs/by-name`, so the merge bot can merge it after a
   committer's approval.

Updates: r-ryantm proposes `base91: A -> B` on its own (`updateScript`);
the maintainer reviews it and merges it with the merge bot (allowed for
r-ryantm's PRs).  The satellites follow.  Recipe changes (such as the
staged `package.nix` layout) need a PR of our own.
