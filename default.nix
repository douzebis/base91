# SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
#
# SPDX-License-Identifier: MIT

{ pkgs ? import (fetchTarball {
    url    = "https://github.com/NixOS/nixpkgs/archive/23d72dabcb3b12469f57b37170fcbc1789bd7457.tar.gz";
    sha256 = "sha256-z5NJPSBwsLf/OfD8WTmh79tlSU8XgIbwmk6qB1/TFzY=";
  }) {}
  # Seed for the randomized tests (spec 0009). CI passes the commit hash, so
  # reruns of a commit test the same cases; null keeps the suite's default.
, testSeed ? null
  # Seconds of libFuzzer per fuzz target in the rust-fuzz attribute.
, fuzzSeconds ? 60
}:

let
  # ---------------------------------------------------------------------------
  # CRANE (Rust build framework)
  # ---------------------------------------------------------------------------
  cranePath = pkgs.fetchgit {
    url    = "https://github.com/ipetkov/crane.git";
    rev    = "80ceeec0dc94ef967c371dcdc56adb280328f591";
    sha256 = "sha256-e1idZdpnnHWuosI3KsBgAgrhMR05T2oqskXCmNzGPq0=";
  };
  crane = pkgs.callPackage cranePath { inherit pkgs; };

  # A second crane instance built from the aarch64 cross package set: its
  # derivations target aarch64-unknown-linux-gnu by themselves and, when
  # running tests, use qemu user-mode as the cargo runner (spec 0009 §8).
  aarch64Pkgs  = pkgs.pkgsCross.aarch64-multiplatform;
  craneAarch64 = aarch64Pkgs.callPackage cranePath { pkgs = aarch64Pkgs; };

  # ---------------------------------------------------------------------------
  # PYTHON INTERPRETER (for PyO3 extension)
  # ---------------------------------------------------------------------------
  pythonBin        = pkgs.python313;
  pythonExecutable = "${pythonBin}/bin/python3";

  # Source for pure-Rust builds (scoped to rust/ so C sources don't affect
  # the hash).  The filter retains Cargo sources, binary test fixtures,
  # README.md files (maturin reads them from Cargo.toml's readme field), the
  # vendored upstream basE91 0.6.0 used as the legacy test oracle
  # (base91-testkit), and the fuzz corpus (spec 0009).
  rustSrc = pkgs.lib.cleanSourceWith {
    src    = pkgs.lib.cleanSource ./rust;
    filter = path: type:
      crane.filterCargoSources path type
      || builtins.match ".*/tests/fixtures/.*" path != null
      || builtins.match ".*/base91-testkit/reference(/.*)?" path != null
      || builtins.match ".*/base91/fuzz/corpus(/.*)?" path != null
      || builtins.match ".*/base91/python-tests(/.*)?" path != null
      || builtins.match ".*/base91/pybase91\\.pyi(\\.license)?" path != null
      || builtins.match ".*/README\\.md$" path != null;
  };

  # Test and lint derivations compile the upstream reference (needs the C
  # compiler of stdenv) so the legacy-oracle tests run in CI: the library
  # comparisons (base91-rs) and the CLI comparisons (base91-cli). Cargo
  # rejects features of packages a command does not select, so attributes
  # selecting only the library use the first variable.
  referenceTestsFeature    = "--features base91-rs/reference-tests";
  referenceTestsFeatureAll = "--features base91-rs/reference-tests,base91-cli/reference-tests,base91-fuzz/reference";

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
    version    = "0.3.1";
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
    cargoClippyExtraArgs = "${referenceTestsFeatureAll} -- --deny warnings";
  });

  # Tests run optimized (crane's release profile) but with debug assertions
  # and overflow checks on: the codec's debug_assert! bounds checks turn an
  # out-of-bounds write into a panic instead of silent memory corruption
  # (spec 0009 §7).
  testEnv = {
    CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS = "true";
    CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS  = "true";
  } // pkgs.lib.optionalAttrs (testSeed != null) {
    BASE91_TEST_SEED = testSeed;
  };

  rustTests = crane.cargoTest (rustCommon // testEnv // {
    pname              = "base91-tests";
    cargoArtifacts     = rustDeps;
    cargoTestExtraArgs = referenceTestsFeatureAll;
  });

  # The library tests for aarch64, run under qemu user-mode: the only way
  # the NEON kernels execute on x86_64 CI machines (spec 0009 §8). The CLI
  # tests are left out: they start child processes, which qemu user-mode
  # cannot run in the sandbox (no binfmt).
  #
  # The tests run in the *build* phase on purpose: stdenv skips the check
  # phase when it cannot execute the host platform, so crane.cargoTest
  # would succeed here without running a single test.
  rustTestsAarch64 = craneAarch64.mkCargoDerivation (rustCommon // testEnv // {
    pname          = "base91-tests-aarch64";
    nativeBuildInputs = [ ];
    cargoArtifacts = null;
    buildPhaseCargoCommand = ''
      cargoWithProfile test --locked -p base91-rs -p base91-testkit \
        ${referenceTestsFeature} 2>&1 | tee test-output.txt
      test "''${PIPESTATUS[0]}" -eq 0
    '';
    doCheck = false;
    installPhaseCommand = ''
      mkdir -p $out
      cp test-output.txt $out/
    '';
  });

  # Long fuzzing runs (spec 0009 §6): libFuzzer through cargo-fuzz on the
  # pinned stable rustc (-s none: no sanitizer, none is available), each
  # target for fuzzSeconds, starting from the committed corpus. A crash
  # fails the build; libFuzzer's log and any crash inputs are kept in $out.
  # Not part of the blocking CI (manual or scheduled). cargo-fuzz builds
  # with debug assertions and overflow checks on.
  rustFuzz = crane.mkCargoDerivation (rustCommon // {
    pname             = "base91-fuzz";
    cargoArtifacts    = null;
    nativeBuildInputs = rustCommon.nativeBuildInputs ++ [ pkgs.cargo-fuzz ];
    buildPhaseCargoCommand = ''
      cd base91
      status=0
      for target in fuzz_decode fuzz_roundtrip fuzz_unchecked fuzz_legacy; do
        mkdir -p ../corpus-run/$target
        cp fuzz/corpus/$target/* ../corpus-run/$target/
        cargo fuzz run -s none --features libfuzzer,reference $target \
          ../corpus-run/$target -- -max_total_time=${toString fuzzSeconds} \
          >> ../fuzz-log.txt 2>&1 || status=1
      done
      cd ..
      test "$status" -eq 0 || { tail -40 fuzz-log.txt; exit 1; }
    '';
    doCheck = false;
    installPhaseCommand = ''
      mkdir -p $out
      cp fuzz-log.txt $out/
      cp -r base91/fuzz/artifacts $out/ 2>/dev/null || true
    '';
  });

  # Relative performance sanity test (spec 0009 §10): plain release build,
  # without the debug assertions of the test attributes, which would distort
  # the timings. Asserts ratios between SIMD levels and layouts only.
  rustPerf = crane.cargoTest (rustCommon // {
    pname              = "base91-perf";
    cargoArtifacts     = rustDeps;
    cargoTestExtraArgs = "-p base91-rs --test perf_sanity -- --ignored --nocapture";
  });

  # Line coverage of the library sources (rust/base91/src) by the whole
  # workspace test suite (spec 0009 §11), with cargo-llvm-cov. Not part of
  # the blocking CI. Fails below 90 % of lines: the first report (2026-09-27)
  # measured 94.3 %, the gap being mostly code the pending list keeps tests
  # away from, CPU-dependent dispatch, trivial Default impls and assertion
  # messages. Re-baseline after spec 0010, which rewrites the driver layer.
  # $out holds the summary and an HTML report.
  #
  # The LLVM tools must match rustc's LLVM, hence rustc.llvm.
  rustCoverage = crane.mkCargoDerivation (rustCommon // {
    pname             = "base91-coverage";
    cargoArtifacts    = null;
    nativeBuildInputs = rustCommon.nativeBuildInputs ++ [ pkgs.cargo-llvm-cov ];
    LLVM_COV          = "${pkgs.rustc.llvm}/bin/llvm-cov";
    LLVM_PROFDATA     = "${pkgs.rustc.llvm}/bin/llvm-profdata";
    buildPhaseCargoCommand = ''
      ignore='(base91-testkit|/fuzz/|base91-cli|/tests/|/benches/|rustc-)'
      cargo llvm-cov --workspace ${referenceTestsFeatureAll} \
        --ignore-filename-regex "$ignore" --summary-only --show-missing-lines \
        --fail-under-lines 90 2>&1 | tee coverage-summary.txt
      test "''${PIPESTATUS[0]}" -eq 0
      cargo llvm-cov report --ignore-filename-regex "$ignore" \
        --html --output-dir coverage-html
    '';
    doCheck = false;
    installPhaseCommand = ''
      mkdir -p $out
      cp coverage-summary.txt $out/
      cp -r coverage-html $out/html
    '';
  });

  # no_std builds (spec 0004 §7): the library without std, with and without
  # alloc, for the host (the crate is #![no_std] there), for wasm32v1-none
  # (a target with no std at all, whose core/alloc nixpkgs' rustc ships),
  # and with the AVX2 compile-time branch; clippy and the library tests too
  # (without alloc they compile but run no test).
  rustNoStd = crane.mkCargoDerivation (rustCommon // {
    pname             = "base91-nostd";
    cargoArtifacts    = rustDeps;
    nativeBuildInputs = rustCommon.nativeBuildInputs ++ [ pkgs.clippy ];
    buildPhaseCargoCommand = ''
      P="--locked -p base91-rs --lib --no-default-features"
      for feat in "" "--features alloc"; do
        cargoWithProfile build  $P $feat
        cargoWithProfile build  $P $feat --target wasm32v1-none
        RUSTFLAGS="-C target-feature=+avx2" cargoWithProfile build $P $feat \
          --target-dir target/avx2
        cargoWithProfile clippy $P $feat -- --deny warnings
        cargoWithProfile test   $P $feat
      done
      # The unchecked API under compile-time detection (spec 0005 §3).
      cargoWithProfile test --locked -p base91-rs --no-default-features \
        --features alloc --test unchecked
    '';
    doCheck = false;
    installPhaseCommand = "mkdir -p $out";
  });

  # The same for aarch64, cross-compiled (spec 0004 §7): catches
  # aarch64-specific no_std breakage (NEON intrinsics) the other builds
  # cannot see.
  rustNoStdAarch64 = craneAarch64.mkCargoDerivation (rustCommon // {
    pname             = "base91-nostd-aarch64";
    nativeBuildInputs = [ ];
    cargoArtifacts    = null;
    buildPhaseCargoCommand = ''
      for feat in "" "--features alloc"; do
        cargoWithProfile build --locked -p base91-rs --lib --no-default-features $feat
      done
    '';
    doCheck = false;
    installPhaseCommand = "mkdir -p $out";
  });

  # The full differential matrix and property runs (spec 0009 §2, §11).
  rustTestsFull = crane.cargoTest (rustCommon // testEnv // {
    pname              = "base91-tests-full";
    cargoArtifacts     = rustDeps;
    cargoTestExtraArgs = "-p base91-rs ${referenceTestsFeature} --test matrix --test invalid_input --test properties";
    BASE91_TEST_FULL   = "1";
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
      # Search all build output dirs (crane may reuse artifacts from the deps phase).
      man1=$(find target -name "base91.1" 2>/dev/null | head -1)
      if [ -n "$man1" ]; then
        out_dir=$(dirname "$man1")
        install -Dm444 "$man1" $out/share/man/man1/base91.1

        installShellCompletion --cmd base91 \
          --bash "$out_dir/base91.bash" \
          --zsh  "$out_dir/_base91" \
          --fish "$out_dir/base91.fish"

        install -Dm444 "$out_dir/base91.elv" \
          $out/share/elvish/lib/completions/base91.elv 2>/dev/null || true
      else
        echo "WARNING: build.rs output not found — man page and completions not installed" >&2
      fi
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
    version = "0.3.1";
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
    version = "0.3.1";

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
  # Uses rustPlatform.maturinBuildHook (nixpkgs standard), not crane.
  # cargoSetupHook vendors deps; maturinBuildHook drives the maturin build.
  # ---------------------------------------------------------------------------
  pybase91 = pkgs.python313Packages.buildPythonPackage {
    pname    = "pybase91";
    version  = "0.3.1";
    pyproject = true;
    src      = rustSrc;

    cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
      pname   = "pybase91";
      version = "0.3.1";
      src     = rustSrc;
      hash    = "sha256-cCNAXpn5QiOtIBON8P7ADovYyfh3nS7GDGh5gqKD5zY=";
    };

    # maturin must be invoked from the crate subdirectory (not the workspace root)
    buildAndTestSubdir = "base91";

    nativeBuildInputs = [
      pkgs.cargo
      pkgs.rustc
      pkgs.rustPlatform.cargoSetupHook
      pkgs.rustPlatform.maturinBuildHook
    ];

    maturinBuildFlags = [ "--features" "python" ];

    # The type stub is committed (maturin ships it in the wheel, with
    # py.typed). Regenerate it and fail if it differs from the committed one,
    # so a stale stub can never be built or released.
    preBuild = ''
      cp base91/pybase91.pyi "$TMPDIR/pybase91.pyi.committed"
      cargo run --offline --locked -p base91-rs --bin post_build --features python
      if ! diff -u "$TMPDIR/pybase91.pyi.committed" base91/pybase91.pyi; then
        echo "error: rust/base91/pybase91.pyi is stale. Regenerate it with" >&2
        echo "  (cd rust && cargo run -p base91-rs --bin post_build --features python)" >&2
        echo "and commit the result." >&2
        exit 1
      fi
    '';

    # Importing the package loads the compiled extension: fails if the wheel
    # has none, which maturin produces silently when it misdetects the
    # bindings (spec 0004 §6, §7).
    pythonImportsCheck = [ "pybase91" ];

    # Python-side tests (spec 0006 §6: the streaming decoder, bug 5). The
    # check phase runs from the workspace root, next to no pybase91 source
    # directory, so the tests import the installed package.
    nativeCheckInputs = [ pkgs.python313Packages.pytestCheckHook ];
    enabledTestPaths  = [ "base91/python-tests" ];

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
      # Python + maturin (for PyO3 development builds and PyPI publishing)
      python313
      maturin
      pkg-config
      # REUSE / SPDX compliance
      reuse
      # GitHub CLI
      gh
      # Nix tooling
      nix-update
      # Test tooling (spec 0009): libFuzzer driver, aarch64 user-mode emulation,
      # coverage (with LLVM tools matching rustc's LLVM)
      cargo-fuzz
      qemu-user
      cargo-llvm-cov
      rustc.llvm
      # Man page viewer
      mandoc
      # Profiling — samply works on Linux (perf_event_open) and macOS (ktrace)
      samply
    ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
      binutils        # objcopy + ar fallback; keep for c-compat-tests build.rs
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
  rust-tests-full = rustTestsFull;
  rust-tests-aarch64 = rustTestsAarch64;
  rust-fuzz     = rustFuzz;
  rust-perf     = rustPerf;
  rust-coverage = rustCoverage;
  rust-nostd    = rustNoStd;
  rust-nostd-aarch64 = rustNoStdAarch64;
  go-tests      = goTests;
}
