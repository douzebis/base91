<!-- SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis) -->
<!-- SPDX-License-Identifier: MIT -->

# Publishing

A `v*` tag publishes to crates.io and PyPI through
`.github/workflows/publish.yaml`, each upload after the maintainer's
approval (spec 0013, `docs/specs/0013-crates-io-publishing.md`). The
release procedure, the one-time setup and the first upload of
`base91-testkit` are in `nixpkgs/README.md`. nixpkgs follows by hand.

---

## 1. crates.io

- `base91-rs` (`https://crates.io/crates/base91-rs`): the library. The
  name `base91` was taken by `dnsl48`.
- `base91-cli` (`https://crates.io/crates/base91-cli`): the `base91`
  command.
- `base91-testkit`: the test oracles, published so that the tests of the
  other two compile from their packages (spec 0013 §1); internal.
- Uploaded by `scripts/publish-crates`, after `scripts/check-crates` (which
  CI also runs on every push). Authentication: trusted publishing,
  environment `crates-io`.

---

## 2. PyPI — `pybase91`

- `https://pypi.org/project/pybase91/`. Trusted publisher: owner
  `douzebis`, repo `base91`, workflow `publish.yaml`, environment `pypi`.
- Wheels: Linux x86_64 + aarch64 (manylinux_2_36), macOS x86_64 +
  aarch64, sdist. Python 3.11–3.13.

---

## 3. nixpkgs

Superseded by spec 0012 (`docs/specs/0012-nixpkgs-aligned-build.md`): the
recipes are staged in `nixpkgs/`, built by `default.nix` and CI, and the
release and PR procedure is in `nixpkgs/README.md`.

- nixpkgs ships `base91` 0.1.0, the C implementation (NixOS/nixpkgs#409224).
- NixOS/nixpkgs#504523 (`base91`, the Rust CLI) and #504526
  (`python3Packages.pybase91`) are open at 0.2.2; both move to the first
  release built from the staged recipes (0.3.2) before anything is merged.

---

## Summary

| Registry  | Package      | Status |
|-----------|--------------|--------|
| crates.io | `base91-rs`  | v0.2.2; from 0.3.2 by `publish.yaml` (spec 0013) |
| crates.io | `base91-cli` | v0.2.2; from 0.3.2 by `publish.yaml` (spec 0013) |
| crates.io | `base91-testkit` | first upload at 0.3.2 (spec 0013) |
| PyPI      | `pybase91`   | by `publish.yaml` on each tag |
| Go proxy  | `github.com/douzebis/base91/go` | live (automatic on tag) |
| nixpkgs   | `base91`     | 0.1.0 (C); PRs #504523, #504526 open (spec 0012) |
