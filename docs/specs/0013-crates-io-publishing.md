<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# 0013 — Publish to crates.io from CI, on approval

**Status:** implemented
**App:** base91
**Implemented in:** 2026-10-11

## Problem

A `v*` tag publishes pybase91 to PyPI and nothing else.

**crates.io is behind (checked 2026-10-11):**

| Crate | crates.io | Repository |
|---|---|---|
| `base91-rs` | 0.2.2 (2026-03-30) | 0.3.2-dev, tags up to `v0.3.1` |
| `base91-cli` | 0.2.2 (2026-03-30) | 0.3.2-dev |

0.2.3, 0.3.0 and 0.3.1 were tagged but never published there: publishing
is a manual `cargo publish` (`docs/publishing.md`), and it was skipped.
crates.io users get 0.2.2, the version with the `simd::*_unchecked`
soundness bug (spec 0005), and none of the later fixes or the faster
kernels; `cargo install base91-cli` installs a 0.2.2 tool.

**PyPI publishes without approval.** `pypi.yml` runs on any `v*` tag, in
the `pypi` environment, which has no protection rule: pushing a tag
uploads to PyPI at once. It does not check that the tag matches the
version it builds, and its actions are pinned by tag, not by commit.

**Nothing checks what crates.io would receive.** CI tests the workspace,
never the packaged crates: a file missing from a package, a path
dependency without a version, metadata crates.io rejects, or tests that
only pass inside the workspace would be found at upload time.

One such defect exists today. Cargo drops a dev-dependency that has a
path and no version when it packages a crate, and both crates' tests use
`base91-testkit` (`publish = false`) that way:

- `base91-rs`: `tests/{invalid_input,matrix,perf_sanity,properties,
  regressions,unchecked}.rs` and the unit tests of `src/simd/driver.rs`;
- `base91-cli`: `tests/end_to_end.rs`.

The packages would build (`cargo package` verifies the library and the
binary only), but `cargo test` from a published `.crate`, as a
distribution packager runs it, would not compile.

yb solved the same problem (yb spec 0029 §9: `scripts/check-crates`,
`scripts/publish-crates`, `publish.yaml`, crates.io trusted publishing,
a `crates-io` environment with a required reviewer), in place since
2026-10-09.

## Goals

- **G1.** A `v*` tag publishes the crates to crates.io from CI, after the
  maintainer's approval, with no stored credential.
- **G2.** PyPI waits for the same approval: nothing leaves CI for a
  registry until the maintainer approves.
- **G3.** CI checks, on every push, the crates exactly as crates.io
  would receive them, tests included.
- **G4.** A failed upload is finished by re-running the job, with nothing
  to do by hand.
- **G5.** The release procedure (`nixpkgs/README.md`) and
  `docs/publishing.md` describe it.

## Non-goals

- **N1.** Yanking 0.2.2 or publishing a security advisory (decided
  2026-09-28: no yank, no advisory).
- **N2.** Publishing the skipped versions (0.2.3, 0.3.0, 0.3.1): the next
  release on crates.io is 0.3.2.
- **N3.** Automating the nixpkgs PR (spec 0012 §7 stays manual).
- **N4.** Publishing the C library or the Go module (the Go proxy picks
  up tags by itself).
- **N5.** Changing the wheel matrix or the build of the PyPI packages.

## Specification

### 1. `base91-testkit` is published

So that the packaged tests compile, `base91-testkit` becomes a published
crate, the third one, at the workspace version:

- `rust/base91-testkit/Cargo.toml`: `publish = false` goes;
  `version.workspace = true`; `repository`; `license = "MIT AND
  BSD-3-Clause"` (the vendored basE91 0.6.0 C sources, compiled only with
  its `reference` feature, off by default); its description says it is
  the test oracles of base91-rs, internal, with no stability promise
  across versions.
- `rust/Cargo.toml`, `[workspace.dependencies]`: `base91-testkit = {
  path = "base91-testkit", version = "…" }`, the same version as the
  `base91` line. `base91-rs` and `base91-cli` take it as
  `base91-testkit.workspace = true` in their dev-dependencies (keeping
  their `reference-tests` features). `base91-fuzz` too; it stays
  unpublished.
- Spec 0012 §5 and the release procedure: the version is now written in
  three lines of `rust/Cargo.toml` (`workspace.package` and the two
  `workspace.dependencies`), all with or without `-dev` together.

`scripts/check-crates` (§2) proves it: the tests run from the unpacked
`.crate` files.

### 2. `scripts/check-crates`

As yb's, adapted:

1. `cargo package --manifest-path rust/Cargo.toml --locked --workspace
   --exclude base91-fuzz`. Online, cargo packages each crate against the
   not-yet-published versions of the others through its temporary
   registry (Rust 1.90+), and builds each `.crate` on its own.
2. Each `.crate` is unpacked outside the workspace and its tests run
   there with default features, in dependency order: `base91-testkit`,
   `base91-rs`, `base91-cli`, each with the crates before it patched to
   their unpacked copies (`--config patch.crates-io.<crate>.path=…`), as
   those versions are not on crates.io at release time.
3. The `.crate` files stay in `rust/target/package/`.

Extra arguments go to `cargo package` (`--allow-dirty` to check a dirty
tree). It is a networked cargo task, not a Nix build, for the reasons
yb spec 0029 §9 gives (vendored sources and cargo's temporary registry do
not combine).

### 3. `scripts/publish-crates`

As yb's: publishes in dependency order (`base91-testkit`, `base91-rs`,
`base91-cli`) with `cargo publish -p … --locked --no-verify`, and skips
a crate already on crates.io at this version (crates.io API: 200 skip,
404 publish, anything else stops).

### 4. `release-shell`

A `mkShell` in `default.nix` with cargo, rustc and curl (the C compiler
comes with stdenv): the toolchain pinned by the nixpkgs pin, the
dependencies by `Cargo.lock`. Lighter than `dev-shell` (no Go, qemu,
fuzzing or profiling tools), so the publishing jobs start quickly.

### 5. `.github/workflows/publish.yaml`

One workflow for both registries, replacing `pypi.yml`:

- **Trigger:** a pushed tag `v*`. Permissions `contents: read` by default.
- **`check-version`:** the tag equals `v` + `workspace.package.version`
  of `rust/Cargo.toml`, and the version has no `-dev` suffix; otherwise
  the workflow stops.
- **`crates`** (needs `check-version`): `scripts/check-crates` in the
  release shell; the `.crate` files as artifact `crates`.
- **`wheels`, `sdist`** (need `check-version`): `pypi.yml`'s build jobs,
  unchanged (N5); artifacts `wheels-*`.
- **`publish-crates`** (needs `crates`): environment `crates-io`,
  `id-token: write`; `rust-lang/crates-io-auth-action` exchanges the
  GitHub OIDC token for a short-lived crates.io token;
  `scripts/publish-crates` in the release shell.
- **`publish-pypi`** (needs `wheels`, `sdist`, and `crates`, so that a
  package that fails its crates.io checks stops PyPI too): environment
  `pypi`, `id-token: write`, `pypa/gh-action-pypi-publish` as today.
- **Approval:** both environments require the maintainer's approval and
  accept `v*` tags only (§6). The maintainer inspects the artifacts, then
  approves each upload; rejecting or ignoring one publishes nothing there.
- **Resumable:** a failed `publish-crates` is re-run; already published
  crates are skipped. PyPI rejects re-uploads, so `publish-pypi` uses
  `skip-existing: true` for the same effect.
- **Least privilege:** only the two publish jobs get `id-token: write`.
  Every third-party action is pinned by commit hash.

A tag pushed by mistake is harmless: reject the pending approvals and
delete the tag.

### 6. One-time setup (maintainer)

- GitHub, repository settings: environment `crates-io` with a required
  reviewer (the maintainer) and a deployment rule limited to tags `v*`;
  the same reviewer and rule added to the existing `pypi` environment.
- crates.io, for `base91-rs` and `base91-cli`: a trusted publisher with
  repository `douzebis/base91`, workflow `publish.yaml`, environment
  `crates-io`; then "trusted publishing only".
- crates.io, `base91-testkit`: trusted publishing cannot create a crate
  (crates.io answers 403, "Trusted Publishing tokens do not support
  creating new crates"), so its first version is uploaded by hand, once,
  during the 0.3.2 release: after the `crates` job passes and before
  approving `publish-crates`, from the tagged checkout in the release
  shell, `cargo publish -p base91-testkit --locked` with a personal API
  token scoped to publishing new crates, revoked afterwards.
  `publish-crates` then skips it (§3) and publishes the other two. Then
  the same trusted publisher and "trusted publishing only" as the others.
- PyPI, project `pybase91`: the trusted publisher's workflow changes from
  `pypi.yml` to `publish.yaml` (environment `pypi` unchanged).

### 7. CI

`rust.yml` runs `scripts/check-crates` on every push (network access,
release shell), so a packaging defect is found before a tag.

### 8. Documentation

- `nixpkgs/README.md`, *Releasing*: after the tag, inspect the `crates`
  and wheel artifacts and approve `publish-crates` and `publish-pypi`;
  then the nixpkgs steps.
- `docs/publishing.md`: crates.io and PyPI sections point to this spec
  and the workflow; the status table drops "published v0.2.2".

## Tests

- `scripts/check-crates` produces `base91-testkit-X.Y.Z.crate`,
  `base91-rs-X.Y.Z.crate` and `base91-cli-X.Y.Z.crate`, and their tests
  pass from the unpacked `.crate` files alone. Before §1, the packaged
  `base91-rs` tests do not compile (checked 2026-10-11: the unit tests
  and six integration tests, on `base91_testkit`).
- `scripts/publish-crates` skips `base91-rs` and `base91-cli` at 0.2.2
  (checked against crates.io, nothing uploaded) and gets 404 for 0.3.2,
  and for `base91-testkit` at any version.
- `publish.yaml` stops on a tag that differs from the Cargo version or
  carries `-dev`, and uploads nothing to either registry before approval.
- First real run: the 0.3.2 release, stopped at both approvals; a wrong
  trusted-publisher setting fails at the authentication step, before any
  upload.

## Open questions

None. Resolved (2026-10-11):

- The packaged tests get `base91-testkit` by publishing it (§1). Rejected:
  leaving the tests that use it out of the packages (they are most of
  them: the oracle and randomized tests), and shipping tests that do not
  compile.
- `publish-pypi` waits for `crates` (§5): one release, checked as a
  whole.

## References

- [spec 0005](0005-simd-unchecked-soundness.md) — the bug in 0.2.2
- [spec 0009](0009-test-strategy.md) — `base91-testkit`, the oracles
- [spec 0012](0012-nixpkgs-aligned-build.md) — one version, the release
  procedure, N3 (this spec)
- `docs/publishing.md` — current manual procedure
- yb spec 0029 §9 (`../yb/docs/specs/0029-nixpkgs-aligned-build.md`),
  `../yb/.github/workflows/publish.yaml`, `../yb/scripts/check-crates`,
  `../yb/scripts/publish-crates` — the model
- crates.io trusted publishing (`https://crates.io/docs/trusted-publishing`):
  existing crates only; `rust-lang/crates-io-auth-action`
