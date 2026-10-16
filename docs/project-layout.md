<!-- SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis) -->
<!-- SPDX-License-Identifier: MIT -->

# Project Layout

This document describes the directory structure of the `base91` repository,
how each component is built, and how it is published.

---

## Directory tree

```
base91/
├── src/                        # C reference implementation
│   ├── base91.c                #   encode/decode library (Henke + perf patch)
│   ├── base91.h                #   public header
│   └── cli.c                   #   original command-line front-end
│
├── rust/                       # Rust workspace
│   ├── Cargo.toml              #   workspace manifest: the version, the members
│   ├── Cargo.lock
│   ├── base91/                 #   base91-rs, the library (crates.io)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs          #     Henke's format, size hints, crate docs
│   │       ├── codec.rs        #     Encoder / Decoder streaming types
│   │       ├── simd/           #     the SIMD variant and its kernels
│   │       ├── io.rs           #     std::io adapters (EncoderWriter, DecoderReader)
│   │       ├── python.rs       #     PyO3 bindings (python feature)
│   │       ├── __capi.rs       #     hidden support for base91-capi
│   │       └── bin/
│   │           └── post_build.rs  #  pyo3-stub-gen stub generator
│   ├── base91-cli/             #   base91-cli, the command (crates.io)
│   │   ├── src/main.rs         #     base91 / b91enc / b91dec
│   │   └── tests/              #     subprocess tests (cli.rs, end_to_end.rs)
│   ├── base91-capi/            #   libbase91, the C library (not on crates.io)
│   │   ├── src/lib.rs          #     Henke's basE91 API and the base91_* API
│   │   ├── include/base91.h    #     the header
│   │   ├── base91.pc.in        #     the pkg-config template
│   │   └── tests/c/api.c       #     the C-side test
│   ├── base91-testkit/         #   test oracles (crates.io, internal)
│   ├── base91-bench/           #   the throughput benchmark (not published)
│   └── base91/fuzz/            #   fuzz targets (not published)
│
├── go/                         # Go module (published to pkg.go.dev)
│   ├── go.mod                  #   module github.com/douzebis/base91/go
│   ├── base91.go               #   Encoder / Decoder / Encode / Decode
│   └── base91_test.go          #   round-trip + reference vector tests
│
├── docs/                       # Project documentation
│   ├── project-layout.md       #   this file
│   ├── publishing.md           #   publishing checklist (crates.io, PyPI, nixpkgs)
│   ├── perf-analysis.md        #   performance notes
│   └── specs/                  #   feature specs
│
├── .github/
│   └── workflows/
│       ├── rust.yml            #   the Rust checks, the packages (see CI below)
│       ├── python.yml          #   pybase91, the stub check
│       ├── c.yml               #   libbase91 and its C-side tests
│       ├── go.yml              #   go test, go vet
│       ├── bench.yml           #   the throughput benchmark
│       ├── publish.yaml        #   crates.io and PyPI on a v* tag, on approval
│       └── reuse.yml           #   REUSE/SPDX compliance check
│
├── LICENSES/                   # Full license texts (REUSE requirement)
│   ├── MIT.txt
│   └── BSD-3-Clause.txt
│
├── REUSE.toml                  # REUSE annotations for files without inline headers
├── CHANGELOG.md
├── README.md
├── default.nix                 # Nix derivations (see below)
├── shell.nix                   # `nix-shell` entry point → dev-shell
└── nixpkgs/                    # nixpkgs recipes for base91 and pybase91, staged
                                #   (built by default.nix; see nixpkgs/README.md)
```

---

## Components

### C reference library (`src/`)

Joachim Henke's original basE91 implementation, with two modifications:
- `__restrict__` qualifiers on `basE91_encode` and `basE91_decode` pointer
  parameters, enabling the compiler to hoist `queue`/`nbits`/`val` struct
  fields into registers.
- State hoisting: the hot loops read struct fields into locals at entry and
  write them back on exit, matching the optimization already present in the
  Rust port.

`cli.c` is the original C command-line front-end; it is not part of the Rust
or Go builds.

### Rust library crate (`rust/base91`)

Pure-Rust, `no_std`-compatible implementation.  Features:

| Feature          | Default | Description |
|------------------|---------|-------------|
| `std`            | yes     | `std`-dependent impls; run-time SIMD detection |
| `alloc`          | (`std`) | the `Vec`-returning API |
| `io`             | yes     | `std::io` adapters (`EncoderWriter`, `DecoderReader`) |
| `python`         | no      | PyO3 bindings for `pybase91` (and the stub generator) |
| `extension-module` | no    | `python`, built as the extension module (maturin) |
| `reference-tests` | no     | tests against Henke's basE91 0.6.0 (needs a C compiler) |

The C reference benchmark (feature `c-compat-tests`) is in the unpublished
`rust/base91-bench`.

### Rust CLI crate (`rust/base91-cli`)

Builds the `base91` binary (symlinked as `b91enc` and `b91dec`).  The
binary prints its man page (`--man`, with `clap_mangen`) and its shell
completions (`--completions SHELL`, with `clap_complete`); packagers install
them from the built binary.

### Go module (`go/`)

Pure Go, zero cgo.  Module path `github.com/douzebis/base91/go`.  The
`Encoder` and `Decoder` types mirror the Rust streaming API.  Test fixtures
are shared with the Rust suite (`rust/base91/tests/fixtures/`).

---

## Building

### Development shell

```sh
nix-shell          # enters the dev-shell defined in default.nix
```

The shell provides: `cargo`, `rustc`, `rustfmt`, `clippy`, `go`, `gcc`,
`binutils`, `python312`, `maturin`, `reuse`, `gh`, `mandoc`.

On entry it runs `cargo build --release` for the Rust workspace and adds
`rust/target/release` to `PATH`.

### Rust workspace

```sh
cargo build --release --manifest-path rust/Cargo.toml
cargo test         --manifest-path rust/Cargo.toml
cargo clippy       --manifest-path rust/Cargo.toml --all-targets -- -D warnings
cargo fmt --check  --manifest-path rust/Cargo.toml
```

### Go module

```sh
cd go && go test ./...
cd go && go vet  ./...
```

### Python extension (development wheel)

```sh
cd rust/base91 && maturin develop --features python
```

### Nix derivations

```sh
nix-build -A base91          # CLI binary + man page + completions (staged nixpkgs recipe)
nix-build -A libbase91       # libbase91.{so,a}, base91.h, base91.pc (staged nixpkgs recipe)
nix-build -A pybase91         # Python package (staged nixpkgs recipe)
nix-build -A pybase91-stub-check    # committed pybase91.pyi is up to date
nix-build -A nixpkgs-staging-check  # nixfmt + version of nixpkgs/
nix-build -A go-tests         # runs Go test suite
nix-build -A rust-tests       # runs Rust test suite
nix-build -A rust-fmt
nix-build -A rust-clippy
```

The `default` output is `base91` (the CLI).

---

## Publishing

A `v*` tag publishes the crates (`base91-testkit`, `base91-rs`,
`base91-cli`) to crates.io and `pybase91` to PyPI, through
`.github/workflows/publish.yaml`, each upload after the maintainer's
approval (spec 0013). The release procedure, nixpkgs included, is in
`nixpkgs/README.md`; the registries are listed in `docs/publishing.md`.
The Go module proxy picks up `go/v*` tags by itself.

---

## CI

| Workflow      | Triggers                              | Jobs |
|---------------|---------------------------------------|------|
| `rust.yml`    | push/PR touching `rust/**`, `nixpkgs/**`, `scripts/**` | evaluation without IFD, fmt, clippy, tests (x86_64, aarch64 under qemu), no_std, `base91`, the staging check, `check-crates` |
| `python.yml`  | push/PR touching `rust/base91/**`     | `pybase91`, the stub check |
| `c.yml`       | push/PR touching `src/**`, `rust/**`  | `libbase91` and its C-side tests |
| `go.yml`      | push/PR touching `go/**`              | test, vet |
| `bench.yml`   | push/PR touching `rust/**`            | the throughput benchmark |
| `publish.yaml`| push of a `v*` tag                    | check, package, approve, publish (crates.io, PyPI) |
| `reuse.yml`   | push/PR (all files)                   | reuse lint |

---

## License

- All Rust and Go sources: **MIT** (`LICENSES/MIT.txt`)
- C reference sources (`src/`, `rust/base91-testkit/reference/`):
  **BSD-3-Clause** (`LICENSES/BSD-3-Clause.txt`); `libbase91`'s header
  declares Henke's API

REUSE 3.3 compliance is enforced on every push via `reuse.yml`.
