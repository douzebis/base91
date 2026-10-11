<!-- SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis) -->
<!-- SPDX-License-Identifier: MIT -->

# Publishing checklist

Three registries, in dependency order: crates.io first (PyPI and nixpkgs
depend on it being published), then PyPI, then nixpkgs.

---

## Pre-flight (all registries)

- [x] All CI workflows green on `main` (Rust, Go, C, Python, REUSE).
- [x] `CHANGELOG.md` has a dated release entry for the version being published.
- [x] Git tag `v0.2.1` created and pushed.
- [x] Git tag `v0.2.2` created and pushed.

---

## 1. crates.io

### 1a. `base91-rs` library crate — DONE

- [x] Published at `https://crates.io/crates/base91-rs` (latest: v0.2.2)
- Note: name `base91` was taken by `dnsl48`; published as `base91-rs`.

### 1b. `base91-cli` binary crate — DONE

- [x] Published at `https://crates.io/crates/base91-cli` (latest: v0.2.2)

---

## 2. PyPI — `pybase91` — DONE

- [x] PyPI project `pybase91` created.
- [x] Trusted Publisher configured: owner `douzebis`, repo `base91`,
  workflow `pypi.yml`, environment `pypi`.
- [x] Published at `https://pypi.org/project/pybase91/` (latest: v0.2.2)
- Wheels: Linux x86_64 + aarch64 (manylinux_2_36), macOS x86_64 + aarch64,
  sdist. Python 3.11–3.13.

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
| crates.io | `base91-rs`  | published v0.2.2 |
| crates.io | `base91-cli` | published v0.2.2 |
| PyPI      | `pybase91`   | published v0.2.2 |
| Go proxy  | `github.com/douzebis/base91/go` | live (automatic on tag) |
| nixpkgs   | `base91`     | 0.1.0 (C); PRs #504523, #504526 open (spec 0012) |
