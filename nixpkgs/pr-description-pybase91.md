python3Packages.pybase91: init at 0.3.2

Follows #504523 (`base91: 0.1.0 -> 0.3.2`, merged): this package takes
`base91`'s source and vendored crates.

Add `python3Packages.pybase91`, Python bindings (PyO3) to base91-rs, the
Rust library behind the `base91` command, for Joachim Henke's basE91
binary-to-text format:

- `pybase91.encode(data: bytes) -> bytes`, `pybase91.decode(data: bytes)
  -> bytes`;
- `pybase91.Encoder` / `pybase91.Decoder`: streaming, `update(data)` then
  `finish()`.

Homepage: https://github.com/douzebis/base91 (on PyPI as `pybase91`).
Changelog: https://github.com/douzebis/base91/blob/v0.3.2/CHANGELOG.md

Packaging:

- `buildPythonPackage` with `pyproject = true`,
  `rustPlatform.cargoSetupHook` and `rustPlatform.maturinBuildHook`; the
  extension is the `rust/base91` crate of the Cargo workspace in `rust/`
  (`cargoRoot`, `buildAndTestSubdir`).  maturin's settings (PyO3 bindings,
  the `python` feature) are in the crate's `pyproject.toml`.
- `inherit (base91) version src cargoDeps`: no second fetch or vendor
  hash; one `nix-update base91` moves both packages.
- The check phase runs the package's Python tests (`pytestCheckHook`) and
  imports the compiled extension (`pythonImportsCheck`).
- No package uses the library yet, so `passthru.tests.matches-cli` is a
  minimal test program (`pkgs/README.md`): `pybase91.encode` must give
  exactly the `base91` command's output (`b91enc`), and decode it back.

Since the previous revision of this PR (0.2.2, approved but not merged):
0.3.2, which fixes a soundness bug of 0.2.2 in two Rust library functions;
`src` and `cargoDeps` from `base91` instead of a separate fetch; the
Python tests run; `meta.license` is MIT, as the package declares (it said
BSD-3-Clause and MIT).

## Things done

- Built on platform:
  - [x] x86_64-linux
  - [ ] aarch64-linux
  - [ ] aarch64-darwin
  - (`python311Packages` to `python314Packages`, tests passing.  Upstream
    CI builds wheels of the same crate for Linux and macOS, x86_64 and
    aarch64.)
- Tested, as applicable:
  - [ ] [NixOS tests] in [nixos/tests].
  - [x] [Package tests] at `passthru.tests`.
  - [ ] Tests in [lib/tests] or [pkgs/test] for functions and "core" functionality.
- [ ] Ran `nixpkgs-review` on this PR. See [nixpkgs-review usage].
  - (A new package with no dependents: built `pybase91` directly for each
    Python version instead.)
- [x] Tested basic functionality of all binary files, usually in `./result/bin/`.
  - (No binaries; `encode`/`decode` round trip, streaming, and output
    identical to `b91enc`, checked in Python.)
- Nixpkgs Release Notes
  - [ ] Package update: when the change is major or breaking.
- NixOS Release Notes
  - [ ] Module addition: when adding a new NixOS module.
  - [ ] Module update: when the change is significant.
- [x] Fits [CONTRIBUTING.md], [pkgs/README.md], [maintainers/README.md] and other READMEs.
- [x] Follows the [automation/AI policy].

## AI disclosure

This package was prepared with Claude Code (Claude Opus 5.5): the
packaging, the commit message and this description were drafted with it
and reviewed by me, the maintainer, who is responsible for them.  The
commit carries an `Assisted-by:` trailer.

[NixOS tests]: https://nixos.org/manual/nixos/unstable/index.html#sec-nixos-tests
[Package tests]: https://github.com/NixOS/nixpkgs/blob/master/pkgs/README.md#package-tests
[nixpkgs-review usage]: https://github.com/Mic92/nixpkgs-review#usage

[CONTRIBUTING.md]: https://github.com/NixOS/nixpkgs/blob/master/CONTRIBUTING.md
[automation/AI policy]: https://github.com/NixOS/nixpkgs/blob/master/CONTRIBUTING.md#automationai-policy
[lib/tests]: https://github.com/NixOS/nixpkgs/blob/master/lib/tests
[maintainers/README.md]: https://github.com/NixOS/nixpkgs/blob/master/maintainers/README.md
[nixos/tests]: https://github.com/NixOS/nixpkgs/blob/master/nixos/tests
[pkgs/README.md]: https://github.com/NixOS/nixpkgs/blob/master/pkgs/README.md
[pkgs/test]: https://github.com/NixOS/nixpkgs/blob/master/pkgs/test

🤖 Generated with [Claude Code](https://claude.com/claude-code)
