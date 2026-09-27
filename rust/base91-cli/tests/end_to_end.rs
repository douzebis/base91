// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! CLI end-to-end tests (spec 0009 §9): real processes, stdin pipes and file
//! arguments.
//!
//! - SIMD mode against the wire-format oracle (spec 0008), at the sizes
//!   around every known threshold (bug 3: multiples of 52 bytes; bug 1:
//!   about 16 kB; the 8 KiB I/O buffers).
//! - Legacy mode against upstream basE91 0.6.0's own CLI, byte for byte
//!   (feature `reference-tests`; skipped with a message without it).
//! - Named regression tests for the CLI-visible bugs, ignored until fixed.
//!
//! Known deviations are skipped through `base91_testkit::pending`, like in
//! the library tests.

use base91_testkit::oracle;
use base91_testkit::pending::{is_pending, Deviation};
use base91_testkit::rng::Rng;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

const SIZES: [usize; 17] = [
    0, 1, 12, 13, 14, 51, 52, 53, 104, 520, 8191, 8192, 8193, 12_000, 16_000, 100_000, 1_000_000,
];

fn ours() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_base91"))
}

/// Runs `exe` with `args` and `stdin`, returning its output.
fn run(exe: &Path, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start {}: {e}", exe.display()));
    let mut pipe = child.stdin.take().unwrap();
    let data = stdin.to_vec();
    // Write from another thread: a large input would otherwise fill the
    // pipe while the child's stdout fills up too.
    let writer = std::thread::spawn(move || pipe.write_all(&data));
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    output
}

/// Like [`run`], with `exe` invoked under the name `name` (both CLIs pick
/// their mode from it).
fn run_as(exe: &Path, name: &str, args: &[&str], stdin: &[u8]) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join(name);
    std::os::unix::fs::symlink(exe, &link).unwrap();
    run(&link, args, stdin)
}

fn ok(output: &Output, what: &str) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{what}: exit {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout.clone()
}

fn input(n: usize) -> Vec<u8> {
    Rng::for_test(&format!("cli n={n}")).bytes(n)
}

// ---------------------------------------------------------------------------
// SIMD mode
// ---------------------------------------------------------------------------

#[test]
fn simd_mode_matches_oracle_and_round_trips() {
    let bin = ours();
    let (mut skipped, mut checked) = (0usize, 0usize);
    for n in SIZES {
        let x = input(n);
        // `None`: the CLI's default width for --simd (64).
        for width in [None, Some(0), Some(32), Some(64), Some(96)] {
            let w = width.unwrap_or(64);
            let mut args = vec!["--simd".to_string()];
            if let Some(w) = width {
                args.extend(["-w".to_string(), w.to_string()]);
            }
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let what = format!("n={n}, {args:?}");
            let encoded = ok(&run(&bin, &args, &x), &what);

            // The encoded stream, against the oracle. Wrapped output lacks
            // its final LF until spec 0006 (bug 3).
            if w == 0 || !is_pending(Deviation::SimdWrappedEncoding) {
                assert!(encoded == oracle::encode(&x, w), "{what}: encoding");
                checked += 1;
            } else {
                skipped += 1;
            }

            // Decoding it back, from a pipe and from a file argument.
            let corrupt = w > 0
                && n > 0
                && oracle::payload_len(n).is_multiple_of(w)
                && is_pending(Deviation::SimdWrappedEncoding);
            let multi_read = encoded.len() > 8192 && is_pending(Deviation::SimdStreamingDecoder);
            if corrupt || multi_read {
                skipped += 1;
                continue;
            }
            assert!(
                ok(&run(&bin, &["-d"], &encoded), &what) == x,
                "{what}: -d, pipe"
            );
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("encoded.b91");
            std::fs::write(&file, &encoded).unwrap();
            let decoded = ok(&run(&bin, &["-d", file.to_str().unwrap()], b""), &what);
            assert!(decoded == x, "{what}: -d, file");
            checked += 1;
        }
    }
    eprintln!("[simd_mode] checked {checked}, skipped {skipped} (pending)");
}

#[test]
fn simd_mode_rejects_invalid_widths() {
    for w in ["16", "48", "50", "76"] {
        let output = run(&ours(), &["--simd", "-w", w], b"x");
        assert!(!output.status.success(), "-w {w} accepted");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("multiple of 32"), "-w {w}: {stderr}");
    }
}

// ---------------------------------------------------------------------------
// Legacy mode, against upstream's CLI
// ---------------------------------------------------------------------------

fn upstream() -> Option<PathBuf> {
    match base91_testkit::REFERENCE_CLI {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("[legacy] skipped: needs --features reference-tests");
            None
        }
    }
}

#[test]
fn legacy_encode_matches_upstream_cli() {
    let Some(reference) = upstream() else {
        return;
    };
    for n in SIZES {
        let x = input(n);
        // (invocation name, arguments): default width 76, explicit widths,
        // and the b91enc name (no wrapping).
        for (name, args) in [
            ("base91", &[][..]),
            ("base91", &["-w", "0"][..]),
            ("base91", &["-w", "1"][..]),
            ("base91", &["-w", "40"][..]),
            ("b91enc", &[][..]),
        ] {
            let what = format!("n={n}, {name} {args:?}");
            let expected = ok(&run_as(&reference, name, args, &x), &what);
            let got = ok(&run_as(&ours(), name, args, &x), &what);
            assert!(got == expected, "{what}");
        }
    }
}

#[test]
fn legacy_decode_matches_upstream_cli() {
    let Some(reference) = upstream() else {
        return;
    };
    for n in SIZES {
        let x = input(n);
        let encoded = ok(&run_as(&reference, "base91", &[], &x), "upstream encode");
        // The stream as upstream wraps it, plus junk that decoders ignore.
        let mut noisy = Vec::new();
        for (i, &c) in encoded.iter().enumerate() {
            noisy.push(c);
            if i % 7 == 3 {
                noisy.extend(b" \t\r\n");
            }
        }
        for stream in [&encoded, &noisy] {
            for (name, args) in [("base91", &["-d"][..]), ("b91dec", &[][..])] {
                let what = format!("n={n}, {name} {args:?}");
                let expected = ok(&run_as(&reference, name, args, stream), &what);
                let got = ok(&run_as(&ours(), name, args, stream), &what);
                assert!(got == expected, "{what}");
                assert!(got == x, "{what}: round trip");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Regressions (spec 0009 §12): ignored until fixed
// ---------------------------------------------------------------------------

#[test]
#[ignore = "fixed by spec 0006, bug 1 (on 0.2.2: base91 -d corrupts SIMD streams from ~16 kB)"]
fn simd_roundtrip_16000() {
    let x = input(16_000);
    let encoded = ok(&run(&ours(), &["--simd"], &x), "encode");
    assert!(ok(&run(&ours(), &["-d"], &encoded), "decode") == x);
}

#[test]
#[ignore = "fixed by spec 0006, bug 3 (on 0.2.2: corrupt output for sizes multiple of 52)"]
fn simd_roundtrip_52() {
    let x = input(52);
    let encoded = ok(&run(&ours(), &["--simd"], &x), "encode");
    assert_eq!(encoded, oracle::encode(&x, 64));
    assert!(ok(&run(&ours(), &["-d"], &encoded), "decode") == x);
}

#[test]
#[ignore = "fixed by spec 0006, bug 6 (on 0.2.2: base91 -d stops at a read that decodes to nothing)"]
fn decode_from_slow_pipe() {
    // Bug 6's reproduction: the encoding of "hello, world" written to the
    // decoder's stdin in three parts with pauses, the middle one a lone LF.
    let encoded = ok(&run(&ours(), &["-w", "0"], b"hello, world"), "encode");
    let mut child = Command::new(ours())
        .arg("-d")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    for part in [&encoded[..8], b"\n", &encoded[8..]] {
        // With bug 6 the decoder exits early and closes the pipe, so later
        // writes fail (EPIPE); ignore that and let the assertion below show
        // the truncated output.
        let _ = pipe.write_all(part).and_then(|()| pipe.flush());
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(pipe);
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&ok(&output, "decode")),
        "hello, world"
    );
}
