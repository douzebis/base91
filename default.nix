# SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
#
# SPDX-License-Identifier: MIT

{ pkgs ? import (fetchTarball {
    # Pinned to nixos-unstable @ e7439b6b14ad3cc35d05608ebca9bce01a25f5f8
    # (2026-10-08, the same as yb): close to nixpkgs master, against which the
    # staged nixpkgs recipes are submitted (spec 0012 §4).
    url    = "https://github.com/NixOS/nixpkgs/archive/e7439b6b14ad3cc35d05608ebca9bce01a25f5f8.tar.gz";
    sha256 = "19y4py973w62z4qb7gzn0chjs67ihxy8rbjb7g71iyk8yz7z5k8z";
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
  # Fetched at evaluation time: a fetcher derivation here would make every
  # evaluation build it first (import from derivation, spec 0012 §4).
  cranePath = builtins.fetchTarball {
    url    = "https://github.com/ipetkov/crane/archive/80ceeec0dc94ef967c371dcdc56adb280328f591.tar.gz";
    sha256 = "1b9yqvf9ihj5n8m6lkrr3lqy22h2c302ldy2lap7b737v9jrsn3v";
  };
  crane = pkgs.callPackage cranePath { inherit pkgs; };

  # The version, from the workspace manifest (spec 0012 §5).
  cargoVersion = (pkgs.lib.importTOML ./rust/Cargo.toml).workspace.package.version;

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
    version    = cargoVersion;
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
  # PACKAGES — the staged nixpkgs recipes, built from the local source
  # (spec 0012 §4)
  # ---------------------------------------------------------------------------
  # The layout of the GitHub tarball that nixpkgs fetches (rust/ under the
  # repository root), restricted to what the build reads.
  localSource = pkgs.lib.cleanSourceWith {
    src    = pkgs.lib.cleanSource ./.;
    filter = path: type:
      let rel = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
      in (rel == "rust" || pkgs.lib.hasPrefix "rust/" rel)
         && !(pkgs.lib.hasPrefix "rust/target" rel)
         && !(pkgs.lib.hasPrefix "rust/rust_out" rel);
  };

  stagedBase91   = ./nixpkgs/pkgs/by-name/ba/base91/package.nix;
  stagedPybase91 = ./nixpkgs/pkgs/development/python-modules/pybase91/default.nix;

  # nixpkgs with base91 replaced by the staged recipe built from the local
  # source, and pybase91 added: pybase91 takes base91's source and vendored
  # crates, as it does in nixpkgs.
  pkgsLocal = pkgs.extend (final: prev: {
    base91 = (final.callPackage stagedBase91 { }).overrideAttrs (old: {
      version   = cargoVersion;
      # tag, as fetchFromGitHub's sources carry it: the recipes'
      # meta.changelog links to it (a local build's changelog is main's).
      src       = localSource // { tag = "main"; };
      # From Cargo.lock: no hash to update when dependencies change (the
      # workspace has no git dependencies).
      cargoDeps = final.rustPlatform.importCargoLock {
        lockFile = ./rust/Cargo.lock;
      };
    });
    pythonPackagesExtensions = prev.pythonPackagesExtensions ++ [
      (pyFinal: pyPrev: {
        pybase91 = pyFinal.callPackage stagedPybase91 { };
      })
    ];
  });

  # The staged files must pass nixpkgs's formatter and carry the right
  # version (spec 0012 §6). Between releases, rust/Cargo.toml has the next
  # version with a -dev suffix and the staged recipe an older one; for a
  # release, both are equal.
  stagedVersion = (pkgs.callPackage stagedBase91 { }).version;
  nextRelease   = pkgs.lib.removeSuffix "-dev" cargoVersion;
  stagedVersionOk =
    if nextRelease != cargoVersion
    then builtins.compareVersions stagedVersion nextRelease < 0
    else stagedVersion == cargoVersion;
  nixpkgsStagingCheck = pkgs.runCommand "base91-nixpkgs-staging-check" {
    nativeBuildInputs = [ pkgs.nixfmt ];
  } (''
    nixfmt --check ${stagedBase91} ${stagedPybase91}
  '' + pkgs.lib.optionalString (!stagedVersionOk) ''
    echo "staged package.nix has version ${stagedVersion}, which does not fit" \
      "rust/Cargo.toml's ${cargoVersion} (equal for a release, older during development)" >&2
    exit 1
  '' + ''
    touch $out
  '');

  # The type stub is committed (maturin ships it in the wheel, with py.typed).
  # Regenerate it and fail if it differs from the committed one, so a stale
  # stub can never be released (spec 0012 §2).
  pybase91StubCheck = crane.mkCargoDerivation (rustCommon // {
    pname          = "pybase91-stub-check";
    cargoArtifacts = rustDeps;
    nativeBuildInputs = rustCommon.nativeBuildInputs ++ [ pythonBin ];
    env.PYO3_PYTHON = pythonExecutable;
    buildPhaseCargoCommand = ''
      cp base91/pybase91.pyi "$TMPDIR/pybase91.pyi.committed"
      cargo run --release --offline --locked -p base91-rs --bin post_build --features python
      if ! diff -u "$TMPDIR/pybase91.pyi.committed" base91/pybase91.pyi; then
        echo "error: rust/base91/pybase91.pyi is stale. Regenerate it with" >&2
        echo "  (cd rust && cargo run -p base91-rs --bin post_build --features python)" >&2
        echo "and commit the result." >&2
        exit 1
      fi
    '';
    doCheck = false;
    installPhaseCommand = "mkdir -p $out";
  });

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
    version = cargoVersion;
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
    version = cargoVersion;

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
  # RELEASE SHELL — packaging and publishing the crates (spec 0013 §4):
  # scripts/check-crates and scripts/publish-crates, which need the network
  # and so run outside a Nix build. The toolchain comes from the nixpkgs pin,
  # the dependencies from Cargo.lock; stdenv brings the C compiler the
  # testkit's build script may need.
  # ---------------------------------------------------------------------------
  release-shell = pkgs.mkShell {
    name = "base91-release";
    nativeBuildInputs = [ pkgs.cargo pkgs.rustc pkgs.curl ];
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
  default       = pkgsLocal.base91;
  base91        = pkgsLocal.base91;                  # staged nixpkgs recipe, local source
  pybase91      = pkgsLocal.python3Packages.pybase91; # staged nixpkgs recipe, local source
  pybase91-stub-check   = pybase91StubCheck;
  nixpkgs-staging-check = nixpkgsStagingCheck;      # nixfmt + version of nixpkgs/
  base91-clib   = base91CLib;
  dev-shell     = dev-shell;
  release-shell = release-shell;                    # scripts/check-crates, publish-crates
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
