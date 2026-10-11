# Briefing: nixpkgs PR `base91: 0.1.0 -> 0.3.2, rewrite in Rust`

Preparation notes for the maintainer before marking NixOS/nixpkgs#504523
ready, so that reviewer questions can be answered first-hand (nixpkgs
automation/AI policy: the contributor must understand the change and answer
without relaying to an AI tool).

## What changes (1 file: `pkgs/by-name/ba/base91/package.nix`)

nixpkgs' `base91` 0.1.0 is the C tool: Joachim Henke's basE91 0.6.0
command-line program, built from this repository's `v0.1.0` tag with
`make -C src`.  0.3.2 is the Rust rewrite of the same tool, same format.

Line by line:

- **`rustPlatform.buildRustPackage (finalAttrs: …)`**: the standard Rust
  builder; `finalAttrs` lets `src.tag` and `meta.changelog` use the version.
- **`src = fetchFromGitHub { …; tag = "v${finalAttrs.version}"; hash }`**:
  `tag` instead of `rev` is the current nixpkgs convention for tags.  The
  hash was cross-checked with `nix-prefetch-url --unpack` on the tag's
  tarball.
- **`cargoRoot = "rust"`**: the Cargo workspace (`Cargo.toml`,
  `Cargo.lock`) is in `rust/`, not at the repository root (the root also
  holds the C reference in `src/` and a Go port in `go/`).  The cargo hooks
  look for `Cargo.lock` there.
- **`buildAndTestSubdir = "rust/base91-cli"`**: build and test only the
  CLI crate, not the whole workspace (which also has the library, the test
  oracles and a fuzzing crate).  Why not `sourceRoot = "source/rust"`: it
  depends on the source's store name, `cargoRoot` does not.
- **`cargoHash`**: hash of the vendored dependencies (`fetchCargoVendor`),
  computed by building `base91.cargoDeps` with a fake hash.  `nix-update`
  maintains both hashes.
- **`nativeBuildInputs = [ installShellFiles ]`**: for `installManPage`
  and `installShellCompletion`.
- **`postInstall`**:
  - `ln -s base91 $out/bin/b91enc` / `b91dec`: the binary picks its mode
    from its name (`b91enc`: encode, no wrapping; `b91dec`: decode), as
    the C tool's `make install` did.
  - The rest runs only when the build platform can execute the binary
    (`stdenv.buildPlatform.canExecute stdenv.hostPlatform`): the man page
    and completions are produced by running `$out/bin/base91` (hidden
    options `--man` and `--completions SHELL`).  Cross builds skip them
    rather than fail.  This replaced, upstream, a `build.rs` that wrote
    them to Cargo's hashed `OUT_DIR`, which the old PR version located with
    `find`; reviewers of a similar package (yb, #514826) objected to that.
  - `b91enc.1` / `b91dec.1` links to `base91.1`: the C package installed
    them too, so `man b91enc` keeps working.  The man-page compression
    hook turns them into links to `base91.1.gz`.
- **Check phase** (default `cargoCheckHook`): the CLI crate's tests, 29 in
  `tests/cli.rs` (options, round trips, errors, the closed-pipe behavior)
  and 8 in `tests/end_to_end.rs`.  The comparisons with the original C
  tool need its sources and a C compiler (feature `reference-tests`), so
  they run in upstream CI, not here.
- **`nativeInstallCheckInputs = [ versionCheckHook ]; doInstallCheck =
  true`**: runs `base91 --version` and checks the output contains the
  version.
- **`passthru.updateScript = nix-update-script { }`**: lets r-ryantm
  propose updates.
- **`__structuredAttrs = true`**: passes the derivation's attributes to
  the builder as structured data instead of environment variables; about
  2,560 `by-name` packages in master set it.  Nothing in this recipe
  depends on it (it can go if a reviewer prefers).
- **`meta`**: MIT (the crate's license; the Henke C sources are not part
  of what is built), `mainProgram = "base91"`, `platforms = unix`,
  `changelog` at the tag (GitHub releases have no notes).

## Likely questions

- **Is it a breaking change for users?**  No for normal use: same
  commands, options, defaults and byte-identical output (checked option by
  option against the 0.1.0 package, see the PR's list).  Differences: usage
  errors exit with 2 instead of 1 (clap's convention), the missing-file
  message does not name the file, different `--help`/`--version`/man page
  text.  That is why the release-notes box is unchecked.
- **Why "rewrite in Rust" in the title?**  Same wording as yb
  (#514826); it tells reviewers the derivation changed builder.
- **Who uses it?**  It is a command-line tool, already in nixpkgs, with no
  dependents.  The Rust crates are on crates.io (`base91-rs`,
  `base91-cli`), the Python binding on PyPI (`pybase91`).
- **Why did the PR say 0.2.2 / "init"?**  It was opened in March as an
  init by mistake (the package existed) and went stale; 0.2.2 had a
  soundness bug in two `unsafe`-backed library functions
  (`simd::encode_unchecked` / `decode_unchecked`), fixed in 0.2.3.  The CLI
  never used them.  The PR is updated in place rather than reopened, to
  keep its history.
- **What is `--simd`?**  An upstream format (not Henke's): fixed-width
  13-bit blocks, which SIMD kernels can process in parallel; streams start
  with `-`.  `base91 -d` detects either format.  Henke's format stays the
  default.
- **Why is aarch64 unchecked?**  Not built with this recipe on aarch64
  machines.  Upstream CI runs the library's tests for aarch64 (NEON
  kernels) under qemu, with a Crane build, not this recipe.  The recipe has
  no architecture-specific part.
- **Darwin?**  `platforms = unix`.  Not built on Darwin with this recipe;
  the Python wheels, built from the same crates by upstream CI, are built
  for macOS (x86_64 and aarch64).
- **Why both a man page and `--help`?**  The man page is generated from
  the same definition (`clap_mangen`), so they cannot drift apart.
- **`pybase91`?**  A second PR after this one: `buildPythonPackage` with
  `maturinBuildHook`, inheriting `version`, `src` and `cargoDeps` from
  `base91`, so one `nix-update base91` moves both.

## Before marking ready

- Read the diff and this file; be able to explain each line above.
- Check that the PR's CI (eval, nixpkgs-vet, build) is green.
