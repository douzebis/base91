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
│   ├── Cargo.toml              #   workspace manifest (members: base91, base91-cli)
│   ├── Cargo.lock
│   ├── base91/                 #   library crate (published to crates.io)
│   │   ├── Cargo.toml
│   │   ├── build.rs            #     compiles C reference for c-compat-tests feature
│   │   └── src/
│   │       ├── lib.rs          #     public API + size hints + encode/decode helpers
│   │       ├── codec.rs        #     Encoder / Decoder streaming types
│   │       ├── io.rs           #     std::io adapters (EncoderWriter, DecoderReader)
│   │       ├── python.rs       #     PyO3 bindings (python feature)
│   │       └── bin/
│   │           └── post_build.rs  #  pyo3-stub-gen stub generator
│   └── base91-cli/             #   CLI binary crate
│       ├── Cargo.toml
│       ├── build.rs            #     generates man page + shell completions at build time
│       ├── src/
│       │   ├── main.rs         #     base91 / b91enc / b91dec binary
│       │   └── cli/            #     clap argument definitions
│       ├── tests/
│       │   └── cli.rs          #     subprocess integration tests
│       └── man/
│           └── man3/           #     committed man(3) pages for the C API
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
│       ├── rust.yml            #   fmt, clippy, test, MSRV
│       ├── go.yml              #   go test, go vet
│       ├── pypi.yml            #   maturin wheel build + PyPI publish on tag
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
└── base91.nix                  # Legacy Nix package (Henke's C CLI)
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
| `std`            | yes     | Enables `std`-dependent impls |
| `io`             | yes     | `std::io` adapters (`EncoderWriter`, `DecoderReader`) |
| `python`         | no      | PyO3 bindings for `pybase91` Python extension |
| `c-compat-tests` | no      | Compiles `src/base91.c` for cross-validation benches |

The `c-compat-tests` feature requires `gcc`, `objcopy`, and `ar` on `PATH`,
and is never enabled for published crate builds.

### Rust CLI crate (`rust/base91-cli`)

Builds the `base91` binary (symlinked as `b91enc` and `b91dec`).  `build.rs`
generates a man page (`base91.1`) and shell completions (bash, zsh, fish,
elvish) into `$OUT_DIR` at build time using `clap_mangen` and
`clap_complete`.  The man(3) pages for the C API live in
`rust/base91-cli/man/man3/` as committed source files.

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
nix-build -A base91          # CLI binary + man page + completions
nix-build -A base91-clib     # libbase91.{so,a} + base91.h (native C)
nix-build -A pybase91         # Python wheel
nix-build -A go-tests         # runs Go test suite
nix-build -A rust-tests       # runs Rust test suite
nix-build -A rust-fmt
nix-build -A rust-clippy
```

The `default` output is `base91` (the CLI).

---

## Publishing

### crates.io — `base91`

**Tag convention:** `v0.2.0` (repo root tag).

```sh
cargo publish --dry-run -p base91   # verify packaging
cargo publish           -p base91
```

Pending before first publish: see `docs/publishing.md` §1 for the
`c-compat-tests` path fix and README requirements.

### PyPI — `pybase91`

**Triggered automatically** by pushing a `v*` tag.  The
`.github/workflows/pypi.yml` workflow builds `manylinux` and macOS wheels
via `maturin-action` and uploads them using OIDC trusted publishing (no API
token required).

To publish manually:

```sh
cd rust/base91
maturin build   --features python --release
maturin publish --features python
```

### pkg.go.dev — `github.com/douzebis/base91/go`

No explicit publish step.  The Go module proxy discovers the module
automatically when anyone runs:

```sh
go get github.com/douzebis/base91/go@v0.2.0
```

**Tag convention for Go subdirectory modules:** `go/v0.2.0` (prefixed with
the subdirectory path).  This is required by the Go module proxy to resolve
the correct subtree of a multi-language monorepo.

### nixpkgs

See `docs/publishing.md` §3.  Blocked on the same `c-compat-tests` path fix
that unblocks crates.io.

---

## CI

| Workflow      | Triggers                              | Jobs |
|---------------|---------------------------------------|------|
| `rust.yml`    | push/PR touching `rust/**`            | fmt, clippy, test, test-msrv (1.74) |
| `go.yml`      | push/PR touching `go/**`              | test, vet |
| `pypi.yml`    | push of `v*` tag                      | build wheels, publish to PyPI |
| `reuse.yml`   | push/PR (all files)                   | reuse lint |

---

## License

- All Rust and Go sources: **MIT** (`LICENSES/MIT.txt`)
- C reference sources (`src/`): **BSD-3-Clause** (`LICENSES/BSD-3-Clause.txt`)

REUSE 3.3 compliance is enforced on every push via `reuse.yml`.
