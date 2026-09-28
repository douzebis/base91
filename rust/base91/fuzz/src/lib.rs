// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Check functions of the fuzz targets (spec 0009 §6).
//!
//! Each function takes raw fuzzer bytes, derives its parameters from the
//! first ones, and asserts what the differential matrix asserts, for that
//! single input. The libFuzzer targets (`fuzz_targets/`) call them, and so
//! does the engine-free corpus replay (`tests/corpus_replay.rs`), which runs
//! in every CI test run.
//!
//! Known deviations are skipped through `base91_testkit::pending`. Memory
//! safety is checked through the codec's debug assertions and through
//! canary-guarded buffers.
//!
//! Parameter bytes: bits 0–1 select the SIMD level (3 = `Simd256`), bits
//! 2–3 the line width (0, 32, 64, 96), bits 4–7 the chunk size of streaming
//! calls (1 to 16).

use base91::simd::{self, SimdLevel};
use base91_testkit::canary::Guarded;
use base91_testkit::oracle::{self, Classification, DecodeError};
use base91_testkit::pending::{is_pending, Deviation};
use base91_testkit::reader::PieceReader;
use std::io::Read;

/// Decoded parameters of a fuzz input.
struct Params {
    level: SimdLevel,
    width: usize,
    chunk: usize,
}

fn params(byte: u8) -> Params {
    Params {
        level: match byte & 3 {
            0 => SimdLevel::Scalar,
            1 => SimdLevel::Simd128,
            _ => SimdLevel::Simd256,
        },
        width: [0, 32, 64, 96][usize::from((byte >> 2) & 3)],
        chunk: 1 + usize::from(byte >> 4),
    }
}

/// Lengths of consecutive chunks of at most `chunk` bytes covering `len`.
fn chunk_lengths(len: usize, chunk: usize) -> Vec<usize> {
    let mut lengths = vec![chunk; len / chunk];
    if !len.is_multiple_of(chunk) {
        lengths.push(len % chunk);
    }
    lengths
}

fn simd_decode_in_chunks(stream: &[u8], p: &Params) -> Result<Vec<u8>, base91::Error> {
    let mut decoder = simd::Decoder::new(p.level);
    let mut out = Vec::new();
    for piece in stream.chunks(p.chunk) {
        decoder.decode(piece, &mut out)?;
    }
    decoder.finish(&mut out)?;
    Ok(out)
}

/// The oracle's result for a SIMD (or empty) stream, as the library
/// reports it.
fn expected(stream: &[u8]) -> Result<Vec<u8>, base91::Error> {
    oracle::decode(stream).map_err(|e| match e {
        DecodeError::InvalidCharacter { offset } => base91::Error::InvalidCharacter { offset },
        DecodeError::InvalidLayout { offset } => base91::Error::InvalidLayout { offset },
        DecodeError::InvalidLength => base91::Error::InvalidLength,
        DecodeError::InvalidValue { offset } => base91::Error::InvalidValue { offset },
    })
}

fn read_all(reader: impl Read) -> Vec<u8> {
    let mut out = Vec::new();
    base91::io::DecoderReader::new(reader)
        .read_to_end(&mut out)
        .unwrap();
    out
}

// ---------------------------------------------------------------------------
// fuzz_decode: arbitrary bytes into every decoder
// ---------------------------------------------------------------------------

/// Arbitrary bytes (after one parameter byte) into every decoder. SIMD
/// streams must decode as the oracle says (once the strict decoder exists;
/// until then only memory safety is checked); legacy streams as upstream
/// does (with feature `reference`).
pub fn check_decode(data: &[u8]) {
    let Some((&byte, stream)) = data.split_first() else {
        return;
    };
    let p = params(byte);
    match oracle::classify(stream) {
        Classification::Legacy => check_legacy_decode(stream, &p),
        Classification::Empty | Classification::Simd => check_simd_decode(stream, &p),
    }
}

fn check_simd_decode(stream: &[u8], p: &Params) {
    // Strict decoding (spec 0008 §3.5): the oracle's bytes, or its error
    // (class and offset), from every decoder and for any chunking.
    let want = expected(stream);
    assert_eq!(simd::decode(stream, p.level), want);
    let mut buf = vec![0u8; simd::decode_size_hint(stream.len())];
    let got = simd::decode_slice(stream, p.level, &mut buf).map(|n| buf[..n].to_vec());
    assert_eq!(got, want);
    assert_eq!(simd_decode_in_chunks(stream, p), want);
}

/// Whether the auto-detecting decoders must treat `stream` as legacy (spec
/// 0008 §2). Only then may they be compared with the legacy reference; the
/// pure legacy decoders (`base91::decode`, `base91::Decoder`) are compared
/// on any input.
fn classified_legacy(stream: &[u8]) -> bool {
    oracle::classify(stream) == Classification::Legacy
}

#[cfg(feature = "reference")]
fn check_legacy_decode(stream: &[u8], p: &Params) {
    let expected = base91_testkit::reference::decode(stream);
    assert_eq!(base91::decode(stream), expected);
    if classified_legacy(stream) {
        assert_eq!(simd::decode(stream, p.level), Ok(expected.clone()));
    }
    if !is_pending(Deviation::LegacyStreamingDecoder) {
        let mut decoder = base91::Decoder::new();
        let mut got = Vec::new();
        for piece in stream.chunks(p.chunk) {
            decoder.decode(piece, &mut got);
        }
        decoder.finish(&mut got);
        assert_eq!(got, expected);
    }
}

#[cfg(not(feature = "reference"))]
fn check_legacy_decode(stream: &[u8], p: &Params) {
    // Without upstream, the decoders must at least agree with each other.
    if classified_legacy(stream) {
        let expected = base91::decode(stream);
        assert_eq!(simd::decode(stream, p.level), Some(expected));
    }
}

// ---------------------------------------------------------------------------
// fuzz_roundtrip: arbitrary data through every encoder and decoder
// ---------------------------------------------------------------------------

/// Arbitrary data (after one parameter byte) encoded and decoded through
/// every API, in both formats, against the oracles.
pub fn check_roundtrip(data: &[u8]) {
    let Some((&byte, x)) = data.split_first() else {
        return;
    };
    let p = params(byte);

    // SIMD format.
    let stream = oracle::encode(x, p.width);
    assert_eq!(stream.len(), oracle::encoded_len(x.len(), p.width));
    if p.width == 0 || !is_pending(Deviation::SimdWrappedEncoding) {
        assert_eq!(simd::encode(x, p.level, p.width), Ok(stream.clone()));
    }
    assert_eq!(simd::decode(&stream, p.level).as_deref(), Ok(x));

    if !x.is_empty() || !is_pending(Deviation::SimdStreamingEncoderEmpty) {
        // Streaming, wrapping included (spec 0010 §2).
        let mut encoder = simd::Encoder::new(p.level, p.width).unwrap();
        let mut got = Vec::new();
        for piece in x.chunks(p.chunk) {
            encoder.encode(piece, &mut got);
        }
        encoder.finish(&mut got);
        assert_eq!(got, stream);
    }
    if !is_pending(Deviation::SimdStreamingDecoder) {
        assert_eq!(simd_decode_in_chunks(&stream, &p).as_deref(), Ok(x));
        if !is_pending(Deviation::IoReaderEarlyEof) {
            let lengths = chunk_lengths(stream.len(), p.chunk);
            assert_eq!(read_all(PieceReader::new(&stream, &lengths)), x);
        }
    }

    // Legacy format.
    let legacy = base91::encode(x);
    #[cfg(feature = "reference")]
    assert_eq!(legacy, base91_testkit::reference::encode(x));
    assert_eq!(base91::decode(&legacy), x);
    if !is_pending(Deviation::LegacyStreamingDecoder) {
        let mut decoder = base91::Decoder::new();
        let mut got = Vec::new();
        for piece in legacy.chunks(p.chunk) {
            decoder.decode(piece, &mut got);
        }
        decoder.finish(&mut got);
        assert_eq!(got, x);
    }
    if !is_pending(Deviation::IoReaderEarlyEof) {
        let lengths = chunk_lengths(legacy.len(), p.chunk);
        assert_eq!(read_all(PieceReader::new(&legacy, &lengths)), x);
    }
}

// ---------------------------------------------------------------------------
// fuzz_unchecked: the raw-pointer APIs with exactly sized, guarded buffers
// ---------------------------------------------------------------------------

/// The `*_unchecked` functions, writing into buffers of exactly the
/// documented size between canaries.
// Exercises the deprecated `*_unchecked` functions: public until 1.0, so
// still fuzzed (spec 0010 §7).
#[allow(deprecated)]
pub fn check_unchecked(data: &[u8]) {
    let Some((&byte, x)) = data.split_first() else {
        return;
    };
    let p = params(byte);

    // Legacy: encode arbitrary data, and decode arbitrary bytes.
    let mut out = Guarded::new(base91::encode_size_hint(x.len()));
    // SAFETY: `out` holds exactly the documented number of bytes.
    let n = unsafe { base91::encode_unchecked(x, out.as_mut_ptr()) };
    out.check();
    assert_eq!(out.written(n), base91::encode(x));

    let mut back = Guarded::new(base91::decode_size_hint(x.len()));
    // SAFETY: as above; any input is allowed.
    let n = unsafe { base91::decode_unchecked(x, back.as_mut_ptr()) };
    back.check();
    assert_eq!(back.written(n), base91::decode(x));

    // SIMD (spec 0005): never called while pending, since it corrupts
    // memory.
    if !is_pending(Deviation::SimdUnchecked) {
        let expected = oracle::encode(x, p.width);
        let mut out = Guarded::new(simd::encode_size_hint(x.len(), p.width));
        // SAFETY: as above.
        let n = unsafe { simd::encode_unchecked(x, p.level, p.width, out.as_mut_ptr()) };
        out.check();
        assert_eq!(out.written(n), expected);

        let mut back = Guarded::new(simd::decode_size_hint(expected.len()));
        // SAFETY: as above.
        let n = unsafe { simd::decode_unchecked(&expected, p.level, back.as_mut_ptr()) };
        back.check();
        assert_eq!(back.written(n), x);
    }
}

// ---------------------------------------------------------------------------
// fuzz_legacy: arbitrary bytes into the legacy decoder, against upstream
// ---------------------------------------------------------------------------

/// Arbitrary bytes into the legacy decoder, which must match upstream
/// exactly (junk ignored), in one call and in chunks.
pub fn check_legacy(data: &[u8]) {
    let Some((&byte, stream)) = data.split_first() else {
        return;
    };
    check_legacy_decode(stream, &params(byte));
}
