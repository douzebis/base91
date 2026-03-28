<!-- SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis) -->
<!-- SPDX-License-Identifier: MIT -->

# Publishing Proposals

Three artifacts to publish, three venues.

---

## 1. crates.io — `base91` crate

**What:** The `rust/base91` library crate (pure-Rust basE91 codec, `no_std`-compatible, C API included).

**Readiness checklist:**

- [ ] `Cargo.toml`: confirm `description`, `repository`, `homepage`, `keywords`, `categories`, `license` are all filled (they are).
- [ ] Write `rust/base91/README.md` — crates.io renders it as the crate homepage. Cover: quick-start, streaming API, C API, `no_std` usage, `python` feature gate.
- [ ] Decide whether to publish the `python` feature. PyO3 pulls in a heavy optional dep tree; consider splitting it into a separate `pybase91` crate if you don't want pyo3 as an optional dep on crates.io.
- [ ] Run `cargo publish --dry-run -p base91` to catch any packaging issues (missing files, path deps, etc.).
- [ ] The `build.rs` C compilation (`../../src/base91.c`) uses a path that is **outside** the crate root and will be excluded from the published crate. Either:
  - (a) Copy `src/base91.c` and `src/base91.h` into `rust/base91/c_ref/` and update the path in `build.rs`, or
  - (b) Gate the C reference benchmarks behind a cargo feature (`c-compat-tests`) that is off by default, and only compile `base91.c` when that feature is active. Users on crates.io never need the C reference — only the Rust tests and the C-API functions, which are pure Rust.

  Option (b) is cleaner: the benches that compare against the C reference simply won't build on crates.io, which is fine.

**Publish command (once ready):**

```sh
cargo publish -p base91
```

---

## 2. PyPI — `pybase91` package

**What:** The `pybase91` Python extension (PyO3 `.so` + `.pyi` stubs), packaged as a wheel.

**Build approach:** [maturin](https://github.com/PyO3/maturin) is the standard PyO3 → PyPI build tool. It handles:
- Cross-platform wheel building (Linux `manylinux`, macOS, Windows)
- `pyproject.toml`-based packaging
- Automatic `.pyi` stub integration (via `pyo3-stub-gen`)
- `pip install pybase91` / `maturin develop` workflow

**Steps:**

1. Replace hatchling with maturin as the build backend in `pyproject.toml`:
   ```toml
   [build-system]
   requires      = ["maturin>=1.7,<2"]
   build-backend = "maturin"

   [tool.maturin]
   features    = ["python"]
   module-name = "pybase91.pybase91"
   python-source = "."
   ```
2. Add `maturin` to the dev-shell and CI.
3. Set up a GitHub Actions workflow using `maturin-action` to build wheels for all target triples and upload to PyPI on tag push.
4. The `post_build` step (pyo3-stub-gen) needs to run as part of the maturin build; hook it via `maturin build --interpreter python3 -- --bin post_build --features python` or a custom `build.rs` step.

**Nix integration:** The existing `base91-py` / `pybase91` derivations in `default.nix` serve for Nix users. PyPI is for the wider Python ecosystem.

---

## 3. nixpkgs — `base91` package

**What:** Contribute the Rust-based `base91` CLI tool (and optionally the C library) to the official nixpkgs package set, replacing or supplementing the existing `base91` package (currently Joachim Henke's C implementation).

**Current nixpkgs status:** `pkgs.base91` exists (the C reference). A Rust port would be a separate package or an updated derivation.

**Approach:**

1. Fork nixpkgs and create `pkgs/by-name/ba/base91/package.nix` (nixpkgs new-style packaging).
2. The derivation uses `rustPlatform.buildRustPackage` (not crane, which is project-local):
   ```nix
   { lib, rustPlatform, installShellFiles }:
   rustPlatform.buildRustPackage {
     pname   = "base91";
     version = "0.2.0";
     src     = fetchFromGitHub { owner = "douzebis"; repo = "base91"; rev = "v0.2.0"; hash = "..."; };
     cargoHash = "...";
     buildAndTestSubdir = "rust";
     nativeBuildInputs = [ installShellFiles ];
     # postInstall: completions, symlinks, man page
   }
   ```
3. The C compilation in `build.rs` (`../../src/base91.c`) needs the same fix as crates.io (see §1 above): either bundle the C source inside `rust/` or gate it behind a feature.
4. Open a PR against nixpkgs. The review process typically asks for:
   - `nix-build` passes
   - `nixpkgs-review` passes
   - Meta fields complete (description, homepage, license, maintainers, platforms)
   - Man page and shell completions installed

**Timeline note:** nixpkgs PRs for new packages typically take 2–6 weeks to merge. A maintainer listing of `@douzebis` keeps future update PRs self-serviceable.

---

## Summary table

| Venue    | Package name | Build tool         | Priority |
|----------|--------------|--------------------|----------|
| crates.io | `base91`    | `cargo publish`    | High — unblocked, just needs README + path fix |
| PyPI     | `pybase91`   | `maturin`          | Medium — needs maturin migration |
| nixpkgs  | `base91`     | `rustPlatform`     | Low — needs upstream path fix first |

The C-source path fix (option b in §1: make it a feature-gated optional bench dep) unblocks all three venues simultaneously.
