libbase91: init at 0.3.3

**Needs `base91` 0.3.3 in master** (the update is expected from
r-ryantm): this package takes `base91`'s source and vendored crates, and
the crate it builds first exists in 0.3.3.  Until then, this branch also
carries a `base91: 0.3.2 -> 0.3.3` commit, so that it builds; **only the
second commit, `libbase91: init at 0.3.3`, belongs to this PR**.  I will
rebase onto master once `base91` is at 0.3.3, and mark the PR ready.

Add `libbase91`, the C library of base91-rs (the Rust library behind the
`base91` command).  base91 encodes binary data as printable ASCII with at
most 23% overhead (base64: 33%).  The library offers two APIs, in
`base91.h`:

- **Henke's basE91 API** (`struct basE91`, `basE91_init`, `basE91_encode`,
  `basE91_encode_end`, `basE91_decode`, `basE91_decode_end`), binary
  compatible with Joachim Henke's basE91 0.6.0: a program built against
  his library runs unchanged with this one.
- **A one-shot API for both formats** (`base91_encode`, `base91_decode`,
  …), Henke's and a fixed-width SIMD variant that SSE4.1, AVX2 and NEON
  encode and decode at several GiB/s; no allocation, sizes checked before
  anything is written, errors as return codes with their offset.

Homepage: https://github.com/douzebis/base91
Changelog: https://github.com/douzebis/base91/blob/v0.3.3/CHANGELOG.md

Packaging:

- `rustPlatform.buildRustPackage`; the library is the `rust/base91-capi`
  crate (`cdylib` and `staticlib`) of the workspace in `rust/`
  (`cargoRoot`, `buildAndTestSubdir`); its Rust tests run in the check
  phase.
- `inherit (base91) version src cargoDeps`: no second fetch or vendor
  hash; one `nix-update base91` moves both packages.
- Outputs `out` (`lib/libbase91.so.0`, SONAME `libbase91.so.0`, with the
  `libbase91.so` link; `lib/libbase91.a`) and `dev` (`include/base91.h`,
  `lib/pkgconfig/base91.pc`, with `Libs.private` for the static library);
  `meta.pkgConfigModules`; `fixDarwinDylibNames` on Darwin.
- `passthru.tests`: `pkg-config` (`testers.hasPkgConfigModules`); `c-api`,
  a C program calling every function through `pkg-config`, linked
  dynamically and statically; `henke-compat`, Henke's own basE91 0.6.0
  command-line tool built against this library, which must write exactly
  what `base91` writes, small buffers included (they overlap its input and
  output).
- `meta.license`: MIT, and BSD-3-Clause for the header, which declares
  Henke's API.

This PR previously proposed 0.2.2, built from Henke's C code with a
hand-written `buildPhase` (mdaniels5757 suggested the generic builder
with `make install`).  The library is now built from the Rust code with
cargo, so that suggestion no longer applies: the recipe has no build or
install phase of its own.

## Things done

- Built on platform:
  - [x] x86_64-linux
  - [ ] aarch64-linux
  - [ ] aarch64-darwin
- Tested, as applicable:
  - [ ] [NixOS tests] in [nixos/tests].
  - [x] [Package tests] at `passthru.tests`.
  - [ ] Tests in [lib/tests] or [pkgs/test] for functions and "core" functionality.
- [ ] Ran `nixpkgs-review` on this PR. See [nixpkgs-review usage].
  - (A new package with no dependents: built it and its `passthru.tests`
    directly.)
- [x] Tested basic functionality of all binary files, usually in `./result/bin/`.
  - (No binaries; the `passthru.tests` exercise the library from C.)
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
