// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Invalid and unusual input (spec 0009 §4): mutated streams fed to every
//! decoder, with the expected outcome taken from the oracles.
//!
//! - Legacy streams (spec 0008 §2 classification): decoders must match the
//!   upstream reference exactly, including on junk bytes, which upstream
//!   ignores (spec 0008 annex A15). Enforced.
//! - SIMD streams: until the strict decoder exists (pending
//!   `SimdStrictDecoding`), decoders are only required to be memory-safe.
//!   Panics are caught and counted; memory corruption aborts the process.
//!   Comparisons with the oracle are counted as skipped.

use base91::simd::{self, SimdLevel};
use base91_testkit::mutate::mutations;
use base91_testkit::oracle::{self, Classification};
use base91_testkit::pending::{is_pending, quiet_expected_panics, Deviation};
use base91_testkit::rng::{full_matrix, Rng};
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};

const LEVELS: [SimdLevel; 3] = [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256];

/// Sizes of the original inputs whose encodings get mutated.
fn sizes(rng: &mut Rng) -> Vec<usize> {
    let mut sizes: Vec<usize> = (0..=40).collect();
    let extra = if full_matrix() { 40 } else { 8 };
    for _ in 0..extra {
        sizes.push(41 + rng.below(3000));
    }
    sizes
}

fn edits() -> usize {
    if full_matrix() {
        32
    } else {
        6
    }
}

/// Decodes `stream` with every SIMD-module decoder (all levels, one-shot,
/// streaming in one chunk, io reader when it fits in one read) and returns
/// the one-shot result at the default level.
fn decode_all_ways(stream: &[u8]) -> Option<Vec<u8>> {
    for level in LEVELS {
        let _ = simd::decode(stream, level);
        let mut decoder = simd::Decoder::new(level);
        let mut out = Vec::new();
        let _ = decoder.decode(stream, &mut out);
        decoder.finish(&mut out);
    }
    if stream.len() <= 8192 || !is_pending(Deviation::SimdStreamingDecoder) {
        let mut out = Vec::new();
        let _ = base91::io::DecoderReader::new(stream).read_to_end(&mut out);
    }
    simd::decode(stream, SimdLevel::default())
}

#[test]
fn mutated_simd_streams() {
    quiet_expected_panics();
    let strict_pending = is_pending(Deviation::SimdStrictDecoding);
    let mut rng = Rng::for_test("mutated_simd_streams");
    let (mut compared, mut skipped, mut panicked) = (0usize, 0usize, 0usize);
    for n in sizes(&mut rng) {
        let input = rng.bytes(n);
        for width in [0, 32, 64] {
            let stream = oracle::encode(&input, width);
            for m in mutations(&stream, &mut rng, edits()) {
                if oracle::classify(&m.stream) == Classification::Legacy {
                    // Covered by `mutated_legacy_streams`.
                    continue;
                }
                let got = if strict_pending {
                    match panic::catch_unwind(AssertUnwindSafe(|| decode_all_ways(&m.stream))) {
                        Ok(got) => got,
                        Err(_) => {
                            panicked += 1;
                            continue;
                        }
                    }
                } else {
                    decode_all_ways(&m.stream)
                };
                if strict_pending {
                    skipped += 1;
                    continue;
                }
                // Until the strict decoder defines its error API, only the
                // accepted streams can be compared.
                if let Ok(expected) = oracle::decode(&m.stream) {
                    assert!(
                        got.as_deref() == Some(&expected[..]),
                        "n={n}, W={width}, {}",
                        m.name
                    );
                    compared += 1;
                }
            }
        }
    }
    eprintln!(
        "[mutated_simd_streams] compared {compared}; skipped {skipped}, \
         panicked {panicked} (SimdStrictDecoding pending: {strict_pending})"
    );
}

#[cfg(feature = "reference-tests")]
#[test]
fn mutated_legacy_streams() {
    use base91_testkit::reference;

    let mut rng = Rng::for_test("mutated_legacy_streams");
    let (mut compared, mut skipped) = (0usize, 0usize);
    for n in sizes(&mut rng) {
        let input = rng.bytes(n);
        let stream = reference::encode(&input);
        for m in mutations(&stream, &mut rng, edits()) {
            let expected = reference::decode(&m.stream);

            // The legacy codec on its own: no classification, junk ignored.
            assert!(base91::decode(&m.stream) == expected, "n={n}, {}", m.name);
            let mut decoder = base91::Decoder::new();
            let mut got = Vec::new();
            decoder.decode(&m.stream, &mut got);
            decoder.finish(&mut got);
            assert!(got == expected, "n={n}, {} (streaming)", m.name);

            // Auto-detecting decoders, when the stream still classifies as
            // legacy (spec 0008 §2).
            if oracle::classify(&m.stream) == Classification::Legacy {
                for level in LEVELS {
                    let got = simd::decode(&m.stream, level);
                    assert!(
                        got.as_deref() == Some(&expected[..]),
                        "n={n}, {}, {level:?}",
                        m.name
                    );
                }
                // Bug 6 (spec 0006): a read that decodes to nothing ends the
                // stream early.
                if is_pending(Deviation::IoReaderEarlyEof) {
                    skipped += 1;
                } else {
                    let mut got = Vec::new();
                    base91::io::DecoderReader::new(&m.stream[..])
                        .read_to_end(&mut got)
                        .unwrap();
                    assert!(got == expected, "n={n}, {} (io reader)", m.name);
                }
            }
            compared += 1;
        }
    }
    eprintln!("[mutated_legacy_streams] compared {compared}; io reader skipped {skipped} (IoReaderEarlyEof)");
}
