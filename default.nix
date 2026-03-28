# SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
#
# SPDX-License-Identifier: MIT

{ pkgs ? import (fetchTarball {
    url    = "https://github.com/NixOS/nixpkgs/archive/23d72dabcb3b12469f57b37170fcbc1789bd7457.tar.gz";
    sha256 = "sha256-z5NJPSBwsLf/OfD8WTmh79tlSU8XgIbwmk6qB1/TFzY=";
  }) {} }:

let
  # ---------------------------------------------------------------------------
  # CRANE (Rust build framework)
  # ---------------------------------------------------------------------------
  crane = pkgs.callPackage (pkgs.fetchgit {
    url    = "https://github.com/ipetkov/crane.git";
    rev    = "80ceeec0dc94ef967c371dcdc56adb280328f591";
    sha256 = "sha256-e1idZdpnnHWuosI3KsBgAgrhMR05T2oqskXCmNzGPq0=";
  }) { inherit pkgs; };

  # ---------------------------------------------------------------------------
  # PYTHON INTERPRETER (for PyO3 extension)
  # ---------------------------------------------------------------------------
  pythonBin        = pkgs.python313;
  pythonExecutable = "${pythonBin}/bin/python3";

  # Source for pure-Rust builds (scoped to rust/ so C sources don't affect
  # the hash).  The filter retains Cargo sources plus binary test fixtures.
  rustSrc = pkgs.lib.cleanSourceWith {
    src    = pkgs.lib.cleanSource ./rust;
    filter = path: type:
      crane.filterCargoSources path type
      || builtins.match ".*/tests/fixtures/.*" path != null;
  };

  # Source tree extended with src/base91.{c,h} for bench builds that enable
  # --features c-compat-tests.  The C files are placed at src/ relative to
  # the workspace root, matching the "../../src/base91.c" path in build.rs
  # (crate at base91/, workspace root two levels up = src/).
  rustSrcWithC = pkgs.runCommand "base91-src-with-c" {} ''
    cp -r ${rustSrc} $out
    chmod -R u+w $out
    mkdir -p $out/src
    cp ${./src/base91.c} $out/src/base91.c
    cp ${./src/base91.h} $out/src/base91.h
  '';

  rustCommon = {
    src        = rustSrc;
    pname      = "base91";
    version    = "0.2.1";
    strictDeps = true;
    nativeBuildInputs = [ pkgs.cargo pkgs.rustc ];
  };

  # Shared dependency cache — rebuilt only when Cargo.lock or dep sources change.
  rustDeps = crane.buildDepsOnly (rustCommon // {
    pname = "base91-deps";
  });

  # ---------------------------------------------------------------------------
  # LINT / TEST DERIVATIONS
  # ---------------------------------------------------------------------------
  rustFmt = crane.cargoFmt (rustCommon // {
    pname = "base91-fmt";
  });

  rustClippy = crane.cargoClippy (rustCommon // {
    pname                = "base91-clippy";
    cargoArtifacts       = rustDeps;
    cargoClippyExtraArgs = "-- --deny warnings";
  });

  rustTests = crane.cargoTest (rustCommon // {
    pname          = "base91-tests";
    cargoArtifacts = rustDeps;
  });

  # ---------------------------------------------------------------------------
  # RUST PACKAGE (CLI binary + man page + completions)
  # ---------------------------------------------------------------------------
  base91Rust = crane.buildPackage (rustCommon // {
    pname          = "base91-rust";
    cargoArtifacts = rustDeps;
    cargoExtraArgs = "-p base91-cli";

    nativeBuildInputs = rustCommon.nativeBuildInputs ++ [
      pkgs.installShellFiles
    ];

    checkPhase = ''
      echo "fmt:    ${rustFmt}"
      echo "clippy: ${rustClippy}"
      echo "tests:  ${rustTests}"
    '';

    postInstall = ''
      # Symlinks for b91enc / b91dec
      ln -s base91 $out/bin/b91enc
      ln -s base91 $out/bin/b91dec

      # Man page and shell completions are written to OUT_DIR by build.rs.
      out_dir=$(find target/release/build/base91-cli-*/out -maxdepth 0 2>/dev/null | head -1)
      if [ -n "$out_dir" ]; then
        install -Dm444 "$out_dir/base91.1" \
          $out/share/man/man1/base91.1

        installShellCompletion --cmd base91 \
          --bash "$out_dir/base91.bash" \
          --zsh  "$out_dir/_base91" \
          --fish "$out_dir/base91.fish"

        # Elvish completion (installShellFiles doesn't have a built-in flag for it)
        install -Dm444 "$out_dir/base91.elv" \
          $out/share/elvish/lib/completions/base91.elv 2>/dev/null || true
      else
        echo "WARNING: build.rs OUT_DIR not found — man page and completions not installed" >&2
      fi

      # Man section 3 (C API) — committed source files, installed directly
      for f in ${./rust/base91-cli/man/man3}/*.3; do
        install -Dm444 "$f" $out/share/man/man3/$(basename "$f")
      done
    '';

    meta = with pkgs.lib; {
      description = "basE91 binary-to-text encoder/decoder (Rust port)";
      homepage    = "https://github.com/douzebis/base91";
      license     = licenses.mit;
      maintainers = [ ];
      mainProgram = "base91";
      platforms   = platforms.unix;
    };
  });

  # ---------------------------------------------------------------------------
  # LEGACY C PACKAGE (preserved for reference / nixpkgs compat)
  # ---------------------------------------------------------------------------
  base91C = pkgs.callPackage ./base91.nix {};

  # ---------------------------------------------------------------------------
  # C LIBRARY — native C build from src/base91.{c,h}
  #
  # Installs:
  #   $out/lib/libbase91.so
  #   $out/lib/libbase91.a
  #   $out/include/base91.h
  # Drop-in replacement for / identical to Joachim Henke's reference.
  # ---------------------------------------------------------------------------
  base91CLib = pkgs.stdenv.mkDerivation {
    pname   = "libbase91";
    version = "0.2.1";
    src     = ./src;

    nativeBuildInputs = [ pkgs.clang pkgs.llvmPackages.bintools ];

    buildPhase = ''
      clang -O3 -fno-plt -fPIC -shared -o libbase91.so base91.c
      clang -O3 -fno-plt -c base91.c -o base91.o
      llvm-ar crs libbase91.a base91.o
    '';

    installPhase = ''
      mkdir -p $out/lib $out/include
      cp libbase91.so $out/lib/
      cp libbase91.a  $out/lib/
      cp base91.h     $out/include/
    '';

    meta = with pkgs.lib; {
      description = "basE91 C library (native C, drop-in replacement)";
      homepage    = "https://github.com/douzebis/base91";
      license     = licenses.bsd3;
      platforms   = platforms.unix;
    };
  };

  # ---------------------------------------------------------------------------
  # GO LIBRARY — pure Go, zero cgo
  # ---------------------------------------------------------------------------
  goTests = pkgs.stdenv.mkDerivation {
    pname   = "base91-go-tests";
    version = "0.2.1";

    # Include go/ sources and the shared test fixtures from the Rust crate.
    src = pkgs.lib.cleanSourceWith {
      src    = pkgs.lib.cleanSource ./.;
      filter = path: type:
        builtins.match ".*/go(/.*)?$" path != null
        || builtins.match ".*/(rust|rust/base91|rust/base91/tests|rust/base91/tests/fixtures)(/.*)?$" path != null;
    };

    nativeBuildInputs = [ pkgs.go ];

    # Point the Go module and build caches at writable locations in the sandbox.
    GOPATH  = "/tmp/gopath";
    GOCACHE = "/tmp/gocache";
    GOFLAGS = "-mod=mod";

    buildPhase = ''
      export BASE91_FIXTURES_DIR="$PWD/rust/base91/tests/fixtures"
      cd go
      go vet ./...
      go test ./...
    '';

    installPhase = ''
      touch $out
    '';

    meta = with pkgs.lib; {
      description = "basE91 Go library tests";
      homepage    = "https://github.com/douzebis/base91";
      license     = licenses.mit;
      platforms   = platforms.unix;
    };
  };

  # ---------------------------------------------------------------------------
  # PYTHON EXTENSION — PyO3 bindings via maturin
  #
  # maturin drives the full cargo build and packages the .so as a wheel.
  # maturinBuildHook (provided by nixpkgs) makes buildPythonPackage speak
  # maturin natively — no separate crane derivation needed.
  # ---------------------------------------------------------------------------
  pybase91 = pkgs.python313Packages.buildPythonPackage {
    pname   = "pybase91";
    version = "0.2.1";
    format  = "pyproject";
    src     = ./rust/base91;

    nativeBuildInputs = [
      pkgs.cargo
      pkgs.rustc
      pkgs.maturin
      pkgs.python313Packages.maturin  # provides maturinBuildHook
      pkgs.pkg-config
    ];

    buildInputs = [ pythonBin ];

    env.PYO3_PYTHON = pythonExecutable;

    meta = with pkgs.lib; {
      description = "basE91 Python extension (Rust/PyO3)";
      homepage    = "https://github.com/douzebis/base91";
      license     = licenses.mit;
      platforms   = platforms.unix;
    };
  };

  # ---------------------------------------------------------------------------
  # DEVELOPMENT SHELL
  # ---------------------------------------------------------------------------
  dev-shell = pkgs.mkShell {
    name = "base91-dev";

    # Allow cargo to write build artifacts to rust/target/ outside /nix/store.
    NIX_ENFORCE_PURITY = 0;

    # Detected by ~/.claude/hooks/claude-hook-post-edit-lint to confirm
    # that the active nix-shell belongs to this repo.
    NIXSHELL_REPO = toString ./.;

    nativeBuildInputs = with pkgs; [
      # Rust toolchain
      cargo
      rustc
      rustfmt
      clippy
      # Go toolchain
      go
      # C toolchain
      clang           # C compiler (LLVM backend, used for bench and libbase91)
      llvmPackages.bintools  # llvm-objcopy + llvm-ar (build.rs symbol rename)
      binutils        # objcopy + ar fallback; keep for c-compat-tests build.rs
      # Python + maturin (for PyO3 development builds and PyPI publishing)
      python313
      maturin
      pkg-config
      # REUSE / SPDX compliance
      reuse
      # GitHub CLI
      gh
      # Man page viewer
      mandoc
    ];

    # PyO3 needs to find the Python interpreter at compile time.
    PYO3_PYTHON = pythonExecutable;

    shellHook = ''
      old_opts=$(set +o)
      set -euo pipefail

      export PATH="$PWD/rust/target/release:$PATH"

      # Build the Rust workspace if not already built.
      cargo build --release --manifest-path rust/Cargo.toml

      echo "Development environment ready."
      echo "  Rust:  $(cargo --version)"
      echo "  rustc: $(rustc --version)"
      echo "  Go:    $(go version)"

      eval "$old_opts"
    '';
  };

in
{
  default       = base91Rust;
  base91        = base91Rust;
  base91-c      = base91C;
  base91-clib   = base91CLib;
  pybase91      = pybase91;
  dev-shell     = dev-shell;
  rust-fmt      = rustFmt;
  rust-clippy   = rustClippy;
  rust-tests    = rustTests;
  go-tests      = goTests;
}
