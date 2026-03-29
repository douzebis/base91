// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! CLI subprocess integration tests.
//!
//! Tests invoke the compiled `base91` binary as a child process via
//! `std::process::Command`.  The binary path is resolved through
//! `CARGO_BIN_EXE_base91` which Cargo sets automatically when running
//! integration tests.

use std::io::Write;
use std::os::unix::fs::symlink;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_base91"))
}

/// Run `base91` with the given args and stdin bytes.
/// Returns `(stdout, stderr, status)`.
fn run(args: &[&str], stdin_data: &[u8]) -> (Vec<u8>, Vec<u8>, std::process::ExitStatus) {
    let mut child = Command::new(bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn base91");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_data)
        .expect("failed to write stdin");

    let out = child.wait_with_output().expect("failed to wait for base91");
    (out.stdout, out.stderr, out.status)
}

/// Like `run`, but uses argv[0] override via a temporary symlink.
/// Creates a symlink `<tmpdir>/<name> -> base91`, invokes it, and cleans up.
fn run_as(
    name: &str,
    args: &[&str],
    stdin_data: &[u8],
) -> (Vec<u8>, Vec<u8>, std::process::ExitStatus) {
    let tmp = tempfile::tempdir().expect("cannot create tempdir");
    let link = tmp.path().join(name);
    symlink(bin(), &link).expect("cannot create symlink");

    let mut child = Command::new(&link)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn symlink binary");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_data)
        .expect("failed to write stdin");

    let out = child.wait_with_output().expect("failed to wait");
    (out.stdout, out.stderr, out.status)
}

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("base91/tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
}

// ---------------------------------------------------------------------------
// Round-trip via pipes
// ---------------------------------------------------------------------------

#[test]
fn round_trip_stdin_stdout() {
    let input = b"Hello, world!";
    let (encoded, _, status) = run(&["-w", "0"], input);
    assert!(status.success());
    let (decoded, _, status2) = run(&["-d"], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

#[test]
fn round_trip_binary_data() {
    let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let (encoded, _, status) = run(&["-w", "0"], &input);
    assert!(status.success());
    let (decoded, _, status2) = run(&["-d"], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

#[test]
fn round_trip_fixture_rnd0() {
    let input = fixture("rnd0.dat");
    let (encoded, _, status) = run(&["-w", "0"], &input);
    assert!(status.success());
    let (decoded, _, status2) = run(&["-d"], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

// ---------------------------------------------------------------------------
// Wrap flag
// ---------------------------------------------------------------------------

#[test]
fn wrap_default_76_produces_wrapped_lines() {
    let input = b"Hello, world!";
    // Default for `base91` is wrap=76.
    let (encoded, _, status) = run(&[], input);
    assert!(status.success());
    for line in encoded.split(|&b| b == b'\n') {
        assert!(
            line.len() <= 76,
            "line longer than 76 chars: {}",
            line.len()
        );
    }
}

#[test]
fn wrap_0_produces_no_newlines() {
    let input: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (encoded, _, status) = run(&["-w", "0"], &input);
    assert!(status.success());
    assert!(!encoded.contains(&b'\n'));
}

#[test]
fn wrap_40_produces_lines_at_most_40() {
    let input: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (encoded, _, status) = run(&["-w", "40"], &input);
    assert!(status.success());
    for line in encoded.split(|&b| b == b'\n') {
        assert!(line.len() <= 40, "line len {} > 40", line.len());
    }
}

#[test]
fn decoder_ignores_newlines_in_input() {
    let input = b"Hello, world!";
    let (encoded_wrapped, _, _) = run(&["-w", "4"], input);
    let (decoded, _, status) = run(&["-d"], &encoded_wrapped);
    assert!(status.success());
    assert_eq!(decoded, input);
}

// ---------------------------------------------------------------------------
// Symlink invocation (b91enc / b91dec)
// ---------------------------------------------------------------------------

#[test]
fn b91enc_defaults_to_no_wrap() {
    let input: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (encoded, _, status) = run_as("b91enc", &[], &input);
    assert!(status.success());
    // b91enc default is no wrap → no newlines.
    assert!(!encoded.contains(&b'\n'));
}

#[test]
fn b91dec_defaults_to_decode() {
    let input = b"Hello, world!";
    let (encoded, _, _) = run(&["-w", "0"], input);
    let (decoded, _, status) = run_as("b91dec", &[], &encoded);
    assert!(status.success());
    assert_eq!(decoded, input);
}

#[test]
fn b91enc_b91dec_round_trip() {
    let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let (encoded, _, status) = run_as("b91enc", &[], &input);
    assert!(status.success());
    let (decoded, _, status2) = run_as("b91dec", &[], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

// ---------------------------------------------------------------------------
// Output file (-o)
// ---------------------------------------------------------------------------

#[test]
fn output_file_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let out_path = tmp.path().join("out.b91");
    let input = b"Hello, world!";

    let (stdout, _, status) = run(&["-w", "0", "-o", out_path.to_str().unwrap()], input);
    assert!(status.success());
    assert!(stdout.is_empty(), "stdout should be empty when -o is used");

    let encoded = std::fs::read(&out_path).unwrap();
    let (decoded, _, status2) = run(&["-d"], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

// ---------------------------------------------------------------------------
// Verbose flag (-v)
// ---------------------------------------------------------------------------

#[test]
fn verbose_writes_to_stderr() {
    let input = b"Hello, world!";
    let (_, stderr, status) = run(&["-v", "-w", "0"], input);
    assert!(status.success());
    let stderr_str = String::from_utf8_lossy(&stderr);
    assert!(
        stderr_str.contains("encoding"),
        "expected 'encoding' in stderr: {stderr_str:?}"
    );
}

#[test]
fn verbose_decode_writes_to_stderr() {
    let input = b"Hello, world!";
    let (encoded, _, _) = run(&["-w", "0"], input);
    let (_, stderr, status) = run(&["-v", "-d"], &encoded);
    assert!(status.success());
    let stderr_str = String::from_utf8_lossy(&stderr);
    assert!(
        stderr_str.contains("decoding"),
        "expected 'decoding' in stderr: {stderr_str:?}"
    );
}

// ---------------------------------------------------------------------------
// --simd flag
// ---------------------------------------------------------------------------

#[test]
fn simd_round_trip() {
    let input = b"Hello, world!";
    let (encoded, _, status) = run(&["--simd"], input);
    assert!(status.success());
    let (decoded, _, status2) = run(&["--simd", "-d"], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

#[test]
fn simd_round_trip_binary() {
    let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let (encoded, _, status) = run(&["--simd"], &input);
    assert!(status.success());
    let (decoded, _, status2) = run(&["--simd", "-d"], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

#[test]
fn simd_output_starts_with_dash() {
    let input = b"Hello, world!";
    let (encoded, _, status) = run(&["--simd", "-w", "0"], input);
    assert!(status.success());
    assert!(
        encoded.first() == Some(&b'-'),
        "SIMD output must start with '-', got: {:?}",
        &encoded[..encoded.len().min(4)]
    );
}

#[test]
fn simd_output_contains_no_single_quote() {
    let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let (encoded, _, status) = run(&["--simd", "-w", "0"], &input);
    assert!(status.success());
    assert!(
        !encoded.contains(&b'\''),
        "SIMD output must not contain single-quote characters"
    );
}

#[test]
fn simd_output_contains_no_double_quote() {
    let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let (encoded, _, status) = run(&["--simd", "-w", "0"], &input);
    assert!(status.success());
    assert!(
        !encoded.contains(&b'"'),
        "SIMD output must not contain double-quote characters"
    );
}

#[test]
fn simd_decode_requires_simd_flag() {
    // --simd -d decodes SIMD streams; plain -d does not (Henke decoder).
    let input: Vec<u8> = (0u8..=255).cycle().take(256).collect();
    let (encoded, _, status) = run(&["--simd", "-w", "0"], &input);
    assert!(status.success());
    // SIMD-encoded stream decodes correctly with --simd -d.
    let (decoded, _, s) = run(&["--simd", "-d"], &encoded);
    assert!(s.success(), "--simd -d must succeed on a SIMD stream");
    assert_eq!(decoded, input);
    // Plain -d (Henke decoder) rejects a SIMD stream (missing '-' prefix check
    // is in Henke decoder, which just treats '-' as a non-alphabet byte and
    // silently produces wrong or empty output — not an error per se, but the
    // round-trip must not equal the original input).
    let (henke_decoded, _, _) = run(&["-d"], &encoded);
    assert_ne!(
        henke_decoded, input,
        "Henke -d must not correctly decode a SIMD stream"
    );
}

#[test]
fn simd_wrap_multiple_of_32_accepted() {
    let input: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (_, _, status) = run(&["--simd", "-w", "32"], &input);
    assert!(status.success());
}

#[test]
fn simd_wrap_non_multiple_of_32_rejected() {
    let input = b"Hello, world!";
    // 16 is a multiple of 16 but not 32 — must be rejected.
    let (_, stderr, status) = run(&["--simd", "-w", "16"], input);
    assert!(
        !status.success(),
        "expected failure for non-multiple-of-32 wrap"
    );
    let stderr_str = String::from_utf8_lossy(&stderr);
    assert!(
        stderr_str.contains("multiple of 32"),
        "expected 'multiple of 32' in error: {stderr_str:?}"
    );
}

#[test]
fn wrap_non_multiple_of_32_without_simd_accepted() {
    // Without --simd, any positive wrap value is fine (including 16 or 17).
    let input: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    let (_, _, status) = run(&["-w", "17"], &input);
    assert!(status.success());
}

#[test]
fn simd_verbose_mentions_simd() {
    let input = b"Hello, world!";
    let (_, stderr, status) = run(&["--simd", "-v"], input);
    assert!(status.success());
    let stderr_str = String::from_utf8_lossy(&stderr);
    assert!(
        stderr_str.to_lowercase().contains("simd"),
        "expected 'SIMD' in stderr: {stderr_str:?}"
    );
}

#[test]
fn simd_round_trip_with_wrap() {
    let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let (encoded, _, status) = run(&["--simd", "-w", "32"], &input);
    assert!(status.success());
    // The output starts with '-' followed by wrapped payload.
    // The first line is '-' + up to 32 payload chars (len <= 33).
    // All subsequent lines are at most 32 chars.
    let mut lines = encoded.split(|&b| b == b'\n');
    if let Some(first) = lines.next() {
        // Strip leading '-' before checking width.
        let payload = first.strip_prefix(b"-").unwrap_or(first);
        assert!(
            payload.len() <= 32,
            "first line payload len {} > 32",
            payload.len()
        );
    }
    for line in lines {
        assert!(line.len() <= 32, "line len {} > 32", line.len());
    }
    let (decoded, _, status2) = run(&["--simd", "-d"], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

#[test]
fn simd_fixture_round_trip() {
    let input = fixture("rnd0.dat");
    let (encoded, _, status) = run(&["--simd", "-w", "0"], &input);
    assert!(status.success());
    let (decoded, _, status2) = run(&["--simd", "-d"], &encoded);
    assert!(status2.success());
    assert_eq!(decoded, input);
}

// ---------------------------------------------------------------------------
// Small-input round-trips (sizes 0..=32) — Henke and SIMD
// ---------------------------------------------------------------------------

#[test]
fn henke_round_trip_sizes_0_to_32() {
    // Use a fixed pattern so failures are reproducible.
    let pattern: Vec<u8> = (0u8..=255).cycle().take(33).collect();
    for len in 0usize..=32 {
        let input = &pattern[..len];
        let (encoded, _, s1) = run(&["-w", "0"], input);
        assert!(s1.success(), "encode failed at len={len}");
        let (decoded, _, s2) = run(&["-d"], &encoded);
        assert!(s2.success(), "decode failed at len={len}");
        assert_eq!(decoded, input, "round-trip mismatch at len={len}");
    }
}

#[test]
fn simd_round_trip_sizes_0_to_32() {
    let pattern: Vec<u8> = (0u8..=255).cycle().take(33).collect();
    for len in 0usize..=32 {
        let input = &pattern[..len];
        let (encoded, _, s1) = run(&["--simd", "-w", "0"], input);
        assert!(s1.success(), "simd encode failed at len={len}");
        let (decoded, _, s2) = run(&["--simd", "-d"], &encoded);
        assert!(s2.success(), "simd decode failed at len={len}");
        assert_eq!(decoded, input, "simd round-trip mismatch at len={len}");
    }
}
