// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Invalid and unusual input (spec 0009 §4): mutated streams fed to every
//! decoder, with the expected outcome taken from the oracles.
//!
//! - Legacy streams (spec 0008 §2 classification): decoders must match the
//!   upstream reference exactly, including on junk bytes, which upstream
//!   ignores (spec 0008 annex A15). Enforced.
//! - SIMD streams (strict decoding, spec 0008 §3.5): every decoder gives
//!   exactly the oracle's result, the bytes or the error, class and offset
//!   (spec 0010 §1), whatever the level and the chunking.

use base91::simd::{self, SimdLevel};
use base91_testkit::mutate::mutations;
use base91_testkit::oracle::{self, Classification, DecodeError};
use base91_testkit::reader::PieceReader;
use base91_testkit::rng::{full_matrix, Rng};
use std::io::Read;

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

/// The oracle's result for a SIMD (or empty) stream, as the library reports
/// it.
fn expected(stream: &[u8]) -> Result<Vec<u8>, base91::Error> {
    oracle::decode(stream).map_err(|e| match e {
        DecodeError::InvalidCharacter { offset } => base91::Error::InvalidCharacter { offset },
        DecodeError::InvalidLayout { offset } => base91::Error::InvalidLayout { offset },
        DecodeError::InvalidLength => base91::Error::InvalidLength,
        DecodeError::InvalidValue { offset } => base91::Error::InvalidValue { offset },
    })
}

/// Chunk lengths covering `len` bytes: fixed sizes, then random.
fn chunkings(rng: &mut Rng, len: usize) -> Vec<Vec<usize>> {
    let mut all: Vec<Vec<usize>> = [1, 7, 16]
        .iter()
        .map(|&size| vec![size; len.div_ceil(size)])
        .collect();
    all.push(rng.split_points(len));
    all
}

/// Streaming decode of `stream` in pieces of the given lengths.
fn decode_streaming(
    stream: &[u8],
    level: SimdLevel,
    lengths: &[usize],
) -> Result<Vec<u8>, base91::Error> {
    let mut decoder = simd::Decoder::new(level);
    let mut out = Vec::new();
    let mut rest = stream;
    for &n in lengths {
        let (piece, tail) = rest.split_at(n.min(rest.len()));
        decoder.decode(piece, &mut out)?;
        rest = tail;
    }
    decoder.finish(&mut out)?;
    Ok(out)
}

/// Every SIMD-module decoder on `stream`, each compared with `want`.
fn check_all_decoders(
    stream: &[u8],
    want: &Result<Vec<u8>, base91::Error>,
    rng: &mut Rng,
    what: &str,
) {
    for level in LEVELS {
        assert!(
            simd::decode(stream, level) == *want,
            "{what}, one-shot, {level:?}"
        );

        let mut buf = vec![0u8; simd::decode_size_hint(stream.len())];
        let got = simd::decode_slice(stream, level, &mut buf).map(|n| buf[..n].to_vec());
        assert!(got == *want, "{what}, slice, {level:?}");

        for lengths in chunkings(rng, stream.len()) {
            let got = decode_streaming(stream, level, &lengths);
            assert!(
                got == *want,
                "{what}, streaming {:?}…, {level:?}",
                &lengths[..lengths.len().min(3)]
            );
        }
    }

    // The io reader, fed in pieces like a pipe: the bytes, or InvalidData
    // wrapping the same error.
    let lengths = rng.split_points(stream.len());
    let mut out = Vec::new();
    let result =
        base91::io::DecoderReader::new(PieceReader::new(stream, &lengths)).read_to_end(&mut out);
    match (result, want) {
        (Ok(_), Ok(bytes)) => assert!(out == *bytes, "{what}, io"),
        (Err(e), Err(error)) => {
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "{what}, io");
            let inner = e
                .into_inner()
                .and_then(|e| e.downcast::<base91::Error>().ok());
            assert_eq!(inner.as_deref(), Some(error), "{what}, io");
        }
        (got, _) => panic!("{what}, io: got {got:?}, want {want:?}"),
    }
}

#[test]
fn mutated_simd_streams() {
    let mut rng = Rng::for_test("mutated_simd_streams");
    let (mut accepted, mut rejected) = (0usize, 0usize);
    for n in sizes(&mut rng) {
        let input = rng.bytes(n);
        for width in [0, 32, 64] {
            let stream = oracle::encode(&input, width);
            for m in mutations(&stream, &mut rng, edits()) {
                if oracle::classify(&m.stream) == Classification::Legacy {
                    // Covered by `mutated_legacy_streams`.
                    continue;
                }
                let want = expected(&m.stream);
                match want {
                    Ok(_) => accepted += 1,
                    Err(_) => rejected += 1,
                }
                let what = format!("n={n}, W={width}, {}", m.name);
                check_all_decoders(&m.stream, &want, &mut rng, &what);
            }
        }
    }
    assert!(accepted > 0 && rejected > 0);
    eprintln!("[mutated_simd_streams] accepted {accepted}, rejected {rejected}");
}

#[cfg(feature = "reference-tests")]
#[test]
fn mutated_legacy_streams() {
    use base91_testkit::pending::{is_pending, Deviation};
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
                        got.as_deref() == Ok(&expected[..]),
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
