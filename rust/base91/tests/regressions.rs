// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Named regression tests for the known bugs (spec 0009 §12).
//!
//! Each one reproduces a bug of 0.2.2 and is ignored until the spec that
//! fixes it is implemented; the fix removes the `#[ignore]`. Some of them
//! crash the process on 0.2.2 (double free, heap overflow), which is why
//! they cannot simply run as expected failures. To check one by hand:
//!
//! ```text
//! cargo test --test regressions -- --ignored --exact <name>
//! ```

use base91::simd::{self, scalar::ScalarDecoder, SimdLevel};
use base91_testkit::canary::Guarded;
use base91_testkit::oracle;
use base91_testkit::reader::PieceReader;
use base91_testkit::rng::Rng;
use std::io::Read;

const LEVELS: [SimdLevel; 3] = [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256];

fn random(n: usize) -> Vec<u8> {
    Rng::new(0x0005_0006_0009).bytes(n)
}

// ---------------------------------------------------------------------------
// Spec 0005
// ---------------------------------------------------------------------------

#[test]
fn unchecked_simd_encode_len_26_27_52() {
    for n in [26, 27, 52] {
        let x = random(n);
        for level in LEVELS {
            let mut out = Guarded::new(simd::encode_size_hint(n, 0));
            // SAFETY: `out` holds exactly the documented number of bytes.
            let written = unsafe { simd::encode_unchecked(&x, level, 0, out.as_mut_ptr()) };
            out.check();
            assert_eq!(
                out.written(written),
                oracle::encode(&x, 0),
                "n={n}, {level:?}"
            );
        }
    }
}

#[test]
fn unchecked_simd_decode_len_32_100() {
    for n in [32, 100] {
        let x = random(n);
        let stream = oracle::encode(&x, 0);
        for level in LEVELS {
            let mut out = Guarded::new(simd::decode_size_hint(stream.len()));
            // SAFETY: as above.
            let written = unsafe { simd::decode_unchecked(&stream, level, out.as_mut_ptr()) };
            out.check();
            assert_eq!(out.written(written), &x[..], "n={n}, {level:?}");
        }
    }
}

#[test]
fn unchecked_simd_encode_empty() {
    for level in LEVELS {
        let mut out = Guarded::new(simd::encode_size_hint(0, 0));
        // SAFETY: as above.
        let written = unsafe { simd::encode_unchecked(&[], level, 0, out.as_mut_ptr()) };
        out.check();
        assert_eq!(out.written(written), b"", "{level:?}");
    }
}

// ---------------------------------------------------------------------------
// Spec 0006
// ---------------------------------------------------------------------------

#[test]
fn simd_decoder_chunked_20k() {
    let x = random(20_000);
    let stream = oracle::encode(&x, 0);
    let mut decoder = simd::Decoder::new(SimdLevel::default());
    let mut got = Vec::new();
    for piece in stream.chunks(4096) {
        assert!(decoder.decode(piece, &mut got));
    }
    decoder.finish(&mut got);
    assert!(got == x);
}

#[test]
fn io_reader_simd_20k() {
    let x = random(20_000);
    let stream = oracle::encode(&x, 64);
    let mut got = Vec::new();
    base91::io::DecoderReader::new(&stream[..])
        .read_to_end(&mut got)
        .unwrap();
    assert!(got == x);
}

#[test]
fn scalar_decoder_chunk_size_1() {
    let x = random(20_000);
    let stream = oracle::encode(&x, 0);
    let mut decoder = ScalarDecoder::new();
    let mut got = Vec::new();
    for piece in stream[1..].chunks(1) {
        decoder.decode(piece, &mut got);
    }
    decoder.finish(&mut got);
    assert!(got == x);
}

#[test]
fn simd_wrap_payload_fills_last_line() {
    // P(26) = 32 = W; P(52) = 64 = W (the CLI default width).
    for (n, width) in [(26, 32), (52, 64)] {
        let x = random(n);
        for level in LEVELS {
            let got = simd::encode(&x, level, width);
            assert_eq!(
                got,
                oracle::encode(&x, width),
                "n={n}, W={width}, {level:?}"
            );
        }
    }
}

#[test]
fn simd_streaming_encoder_empty() {
    let mut encoder = simd::Encoder::new(SimdLevel::default());
    let mut got = Vec::new();
    encoder.encode(b"", &mut got);
    encoder.finish(&mut got);
    assert_eq!(got, b"");
}

#[test]
fn legacy_decoder_chunk_size_1() {
    // The input that crashes pybase91.Decoder.update() fed byte by byte.
    let x: Vec<u8> = (0..=255u8).cycle().take(256 * 40).collect();
    let stream = base91::encode(&x);
    let mut decoder = base91::Decoder::new();
    let mut got = Vec::new();
    for piece in stream.chunks(1) {
        decoder.decode(piece, &mut got);
    }
    decoder.finish(&mut got);
    assert!(got == x);
}

#[test]
fn io_reader_lone_newline_read() {
    // The pipe of bug 6: the encoding of "hello, world" arrives in three
    // reads, the middle one a lone newline.
    let encoded = base91::encode(b"hello, world");
    let data = [&encoded[..8], b"\n", &encoded[8..]].concat();
    let lengths = [8, 1, encoded.len() - 8];
    let mut got = Vec::new();
    base91::io::DecoderReader::new(PieceReader::new(&data, &lengths))
        .read_to_end(&mut got)
        .unwrap();
    assert_eq!(got, b"hello, world");
}

#[test]
fn io_reader_one_byte_reads() {
    for x in [random(1), random(1000)] {
        let encoded = base91::encode(&x);
        let lengths = vec![1; encoded.len()];
        let mut got = Vec::new();
        base91::io::DecoderReader::new(PieceReader::new(&encoded, &lengths))
            .read_to_end(&mut got)
            .unwrap();
        assert!(got == x, "n={}", x.len());
    }
}
