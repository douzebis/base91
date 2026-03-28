<!-- SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis) -->
<!-- SPDX-License-Identifier: MIT -->

# Publishing checklist

Three registries, in dependency order: crates.io first (PyPI and nixpkgs
depend on it being published), then PyPI, then nixpkgs.

---

## Pre-flight (all registries)

- [x] All CI workflows green on `main` (Rust, Go, C, Python, REUSE).
- [x] `CHANGELOG.md` has a dated release entry for the version being published.
- [x] Git tag `v0.2.0` created and pushed.

---

## 1. crates.io

### 1a. `base91-rs` library crate — DONE

- [x] Published at `https://crates.io/crates/base91-rs`
- Note: name `base91` was taken by `dnsl48`; published as `base91-rs`.

### 1b. `base91-cli` binary crate — DONE

- [x] Published at `https://crates.io/crates/base91-cli`

---

## 2. PyPI — `pybase91` — DONE

- [x] PyPI project `pybase91` created.
- [x] Trusted Publisher configured: owner `douzebis`, repo `base91`,
  workflow `pypi.yml`, environment `pypi`.
- [x] Published at `https://pypi.org/project/pybase91/`
- Wheels: Linux x86_64 + aarch64 (manylinux_2_36), macOS x86_64 + aarch64,
  sdist. Python 3.11–3.13.

---

## 3. nixpkgs — TODO

**Current state:** Not yet submitted. Crates.io is live so this is unblocked.
The derivation has been tested locally and builds successfully.

- [ ] Sync `douzebis/nixpkgs` fork with upstream `master`.
- [ ] Create branch `base91-init`.
- [ ] Create `pkgs/by-name/ba/base91/package.nix`:
  ```nix
  { lib, rustPlatform, installShellFiles }:
  rustPlatform.buildRustPackage {
    pname   = "base91";
    version = "0.2.0";

    src = fetchFromGitHub {
      owner = "douzebis";
      repo  = "base91";
      rev   = "v0.2.0";
      hash  = "sha256-ZuSDn/W7oC0An8wbdS/KW+V5KG/1QwCVtf6uP4tMmiM=";
    } + "/rust";

    cargoHash = "sha256-4BVJl3KcsJOO3/TMvFzasZHVKe+AQoE/ItzlSvOYzTU=";

    cargoExtraArgs = "-p base91-cli";

    nativeBuildInputs = [ installShellFiles ];

    postInstall = ''
      ln -s base91 $out/bin/b91enc
      ln -s base91 $out/bin/b91dec
      out_dir=$(find . -path "*/build/base91-cli-*/out" -maxdepth 6 | head -1)
      if [ -n "$out_dir" ]; then
        install -Dm444 "$out_dir/base91.1" $out/share/man/man1/base91.1
        installShellCompletion --cmd base91 \
          --bash "$out_dir/base91.bash" \
          --zsh  "$out_dir/_base91" \
          --fish "$out_dir/base91.fish"
      fi
    '';

    meta = with lib; {
      description = "basE91 binary-to-text encoder/decoder";
      homepage    = "https://github.com/douzebis/base91";
      license     = licenses.mit;
      maintainers = [ maintainers.douzebis ];
      mainProgram = "base91";
      platforms   = platforms.unix;
    };
  }
  ```
- [ ] Add `douzebis` to `maintainers/maintainer-list.nix` if not present.
- [ ] Build and test locally against nixpkgs master.
- [ ] Open PR: `base91: init at 0.2.0`.
- [ ] Respond to reviewer feedback. Merge time typically 2–6 weeks.

---

## Summary

| Registry  | Package      | Status |
|-----------|--------------|--------|
| crates.io | `base91-rs`  | published v0.2.0 |
| crates.io | `base91-cli` | published v0.2.0 |
| PyPI      | `pybase91`   | published v0.2.0 |
| Go proxy  | `github.com/douzebis/base91/go` | live (automatic on tag) |
| nixpkgs   | `base91`     | PR not yet opened |
