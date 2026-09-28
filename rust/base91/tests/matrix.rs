// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Differential matrix (spec 0009 §2, §3): every public API of base91-rs
//! against the independent oracles of base91-testkit.
//!
//! By default a seeded subset runs, together with the exhaustive small cases
//! of §3; `BASE91_TEST_FULL=1` runs everything. Comparisons that a known
//! deviation would fail are skipped through `base91_testkit::pending`, and
//! the number of skipped comparisons is printed.

use base91::simd::{self, SimdLevel};
use base91_testkit::canary::Guarded;
use base91_testkit::oracle;
use base91_testkit::pending::{is_pending, Deviation};
use base91_testkit::reader::PieceReader;
use base91_testkit::rng::{full_matrix, Rng};
use std::io::Read;

const LEVELS: [SimdLevel; 3] = [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256];
const WIDTHS: [usize; 4] = [0, 32, 64, 96];
const CHUNKS: [usize; 14] = [1, 2, 3, 7, 13, 15, 16, 17, 31, 32, 33, 4096, 8191, 8192];

// ---------------------------------------------------------------------------
// Inputs, chunkings, skip accounting
// ---------------------------------------------------------------------------

/// Largest length covered exhaustively (§3): 3 * 13 * 8 bytes spans every
/// tail length, block boundary and 32/64-character line boundary.
const EXHAUSTIVE: usize = 312;

/// Test inputs as (description, bytes).
fn inputs(test: &str) -> Vec<(String, Vec<u8>)> {
    let mut rng = Rng::for_test(test);
    let full = full_matrix();
    let mut lengths: Vec<usize> = (0..=EXHAUSTIVE).collect();
    if full {
        lengths.extend(EXHAUSTIVE + 1..=3000);
        lengths.push(1 << 20);
    } else {
        for _ in 0..40 {
            lengths.push(EXHAUSTIVE + 1 + rng.below(3000 - EXHAUSTIVE));
        }
    }
    lengths.push(20_000);

    let mut out = Vec::new();
    for n in lengths {
        out.push((format!("random n={n}"), rng.bytes(n)));
        if n <= EXHAUSTIVE || full {
            out.push((format!("zeros n={n}"), vec![0x00; n]));
            // All-0xFF maximizes every 13-bit group (8191).
            out.push((format!("ones n={n}"), vec![0xFF; n]));
            let counting = (0..n).map(|i| i as u8).collect();
            out.push((format!("counting n={n}"), counting));
        }
    }
    out
}

/// Chunk-length sequences covering `len` bytes, as (description, lengths).
fn chunkings(rng: &mut Rng, len: usize) -> Vec<(String, Vec<usize>)> {
    let sizes: Vec<usize> = if full_matrix() {
        CHUNKS.to_vec()
    } else {
        (0..3).map(|_| CHUNKS[rng.below(CHUNKS.len())]).collect()
    };
    let mut out: Vec<(String, Vec<usize>)> = sizes
        .into_iter()
        .map(|k| {
            let mut lengths = vec![k; len / k];
            if !len.is_multiple_of(k) {
                lengths.push(len % k);
            }
            (format!("chunks of {k}"), lengths)
        })
        .collect();
    out.push(("random split".to_string(), rng.split_points(len)));
    out
}

/// `data` cut into consecutive pieces of the given lengths.
fn pieces<'a>(data: &'a [u8], lengths: &[usize]) -> Vec<&'a [u8]> {
    let mut rest = data;
    let mut out = Vec::new();
    for &len in lengths {
        let (head, tail) = rest.split_at(len);
        out.push(head);
        rest = tail;
    }
    assert!(rest.is_empty());
    out
}

/// Counts comparisons skipped because of a pending deviation, and reports
/// them when dropped.
struct Skipped {
    test: &'static str,
    deviation: Deviation,
    count: usize,
}

impl Skipped {
    fn new(test: &'static str, deviation: Deviation) -> Self {
        Self {
            test,
            deviation,
            count: 0,
        }
    }

    /// True (and counted) if the comparison must be skipped.
    fn skip(&mut self) -> bool {
        let pending = is_pending(self.deviation);
        if pending {
            self.count += 1;
        }
        pending
    }
}

impl Drop for Skipped {
    fn drop(&mut self) {
        if self.count > 0 {
            eprintln!(
                "[pending] {}: skipped {} comparisons ({:?})",
                self.test, self.count, self.deviation
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Which kernels the matrix actually exercises
// ---------------------------------------------------------------------------

/// The level the dispatcher detects, checked against the CPU, so that a
/// passing matrix really means the SIMD kernels ran (spec 0009 §8): on
/// aarch64 (NEON is mandatory) it must be `Simd128`; on x86_64 it follows
/// the CPU's AVX2 / SSE4.1 support. Printed for the CI log.
#[test]
fn detected_level_matches_the_cpu() {
    let detected = simd::detect();
    eprintln!("[matrix] simd::detect() = {detected:?}");
    if cfg!(feature = "force-scalar") {
        assert_eq!(detected, SimdLevel::Scalar);
        return;
    }
    #[cfg(target_arch = "aarch64")]
    assert_eq!(detected, SimdLevel::Simd128);
    #[cfg(target_arch = "x86_64")]
    {
        let expected = if std::is_x86_feature_detected!("avx2") {
            SimdLevel::Simd256
        } else if std::is_x86_feature_detected!("sse4.1") {
            SimdLevel::Simd128
        } else {
            SimdLevel::Scalar
        };
        assert_eq!(detected, expected);
    }
}

// ---------------------------------------------------------------------------
// SIMD format: encoders
// ---------------------------------------------------------------------------

#[test]
fn simd_encode_unwrapped_matches_oracle() {
    for (name, x) in inputs("simd_encode_unwrapped") {
        let expected = oracle::encode(&x, 0);
        assert_eq!(expected.len(), oracle::encoded_len(x.len(), 0));
        for level in LEVELS {
            let got = simd::encode(&x, level, 0).unwrap();
            assert!(got == expected, "{name}, {level:?}");
        }
    }
}

#[test]
fn simd_encode_wrapped_matches_oracle() {
    let mut skipped = Skipped::new("simd_encode_wrapped", Deviation::SimdWrappedEncoding);
    for (name, x) in inputs("simd_encode_wrapped") {
        for width in [32, 64, 96] {
            if skipped.skip() {
                continue;
            }
            let expected = oracle::encode(&x, width);
            for level in LEVELS {
                let got = simd::encode(&x, level, width).unwrap();
                assert!(got == expected, "{name}, W={width}, {level:?}");
            }
        }
    }
}

#[test]
fn simd_streaming_encoder_matches_oracle() {
    let mut rng = Rng::for_test("simd_streaming_encoder");
    let mut skipped = Skipped::new(
        "simd_streaming_encoder",
        Deviation::SimdStreamingEncoderEmpty,
    );
    for (name, x) in inputs("simd_streaming_encoder") {
        for (plan, lengths) in chunkings(&mut rng, x.len()) {
            for width in WIDTHS {
                let expected = oracle::encode(&x, width);
                for level in LEVELS {
                    let mut encoder = simd::Encoder::new(level, width).unwrap();
                    let mut got = Vec::new();
                    for piece in pieces(&x, &lengths) {
                        encoder.encode(piece, &mut got);
                    }
                    encoder.finish(&mut got);
                    assert!(got == expected, "{name}, {plan}, W={width}, {level:?}");
                }
            }
        }
        let expected = oracle::encode(&x, 0);
        // Empty calls interleaved: they must not change the output. With an
        // empty input this is bug 4 (spec 0006).
        if x.is_empty() && skipped.skip() {
            continue;
        }
        let mut encoder = simd::Encoder::new(SimdLevel::default(), 0).unwrap();
        let mut got = Vec::new();
        encoder.encode(b"", &mut got);
        encoder.encode(&x, &mut got);
        encoder.encode(b"", &mut got);
        encoder.finish(&mut got);
        assert!(got == expected, "{name}, with empty calls");
    }
}

/// The slice API (spec 0010 §2): into canary-guarded buffers of exactly the
/// required size, one-shot and streaming. Each streaming call is first given
/// a buffer one byte too small: it must fail with the exact size, write
/// nothing and keep its state (the final output proves it).
#[test]
fn simd_slice_api_matches_oracle() {
    let mut rng = Rng::for_test("simd_slice_api");
    for (name, x) in inputs("simd_slice_api") {
        for width in WIDTHS {
            let expected = oracle::encode(&x, width);
            for level in LEVELS {
                let what = format!("{name}, W={width}, {level:?}");
                let needed = simd::encoded_len(x.len(), width).unwrap();
                assert_eq!(needed, expected.len(), "{what}");
                let mut out = Guarded::new(needed);
                // SAFETY: `out` holds `needed` initialized bytes.
                let buf = unsafe { std::slice::from_raw_parts_mut(out.as_mut_ptr(), needed) };
                let n = simd::encode_slice(&x, level, width, buf).unwrap();
                out.check();
                assert!(out.written(n) == expected, "{what}: encode_slice");

                let hint = simd::decode_size_hint(expected.len());
                let mut back = Guarded::new(hint);
                // SAFETY: as above.
                let buf = unsafe { std::slice::from_raw_parts_mut(back.as_mut_ptr(), hint) };
                let n = simd::decode_slice(&expected, level, buf).unwrap();
                back.check();
                assert!(back.written(n) == x, "{what}: decode_slice");
            }

            let lengths = rng.split_points(x.len());
            let mut encoder = simd::Encoder::new(SimdLevel::default(), width).unwrap();
            let mut got = Vec::new();
            let mut encode_piece = |got: &mut Vec<u8>, piece: Option<&[u8]>| {
                let call = |enc: &mut simd::Encoder, buf: &mut [u8]| match piece {
                    Some(p) => enc.encode_slice(p, buf),
                    None => enc.finish_slice(buf),
                };
                let mut probe = [0u8; 0];
                let needed = match call(&mut encoder, &mut probe) {
                    Ok(0) => return,
                    Err(base91::Error::BufferTooSmall { needed }) => needed,
                    other => panic!("{name}, W={width}: {other:?}"),
                };
                let mut small = vec![0xA5u8; needed - 1];
                assert_eq!(
                    call(&mut encoder, &mut small),
                    Err(base91::Error::BufferTooSmall { needed })
                );
                assert!(small.iter().all(|&b| b == 0xA5));
                let mut buf = vec![0u8; needed];
                assert_eq!(call(&mut encoder, &mut buf), Ok(needed));
                got.extend_from_slice(&buf);
            };
            for piece in pieces(&x, &lengths) {
                encode_piece(&mut got, Some(piece));
            }
            encode_piece(&mut got, None);
            assert!(got == expected, "{name}, W={width}: streaming encode_slice");

            let lengths = rng.split_points(expected.len());
            let mut decoder = simd::Decoder::new(SimdLevel::default());
            let mut back = Vec::new();
            for piece in pieces(&expected, &lengths) {
                // A call needing no room succeeds on the empty probe, and
                // has then consumed its piece.
                let needed = match decoder.decode_slice(piece, &mut []) {
                    Ok(n) => {
                        assert_eq!(n, 0);
                        continue;
                    }
                    Err(base91::Error::BufferTooSmall { needed }) => needed,
                    Err(e) => panic!("{name}, W={width}: {e:?}"),
                };
                assert!(needed <= simd::decode_size_hint(piece.len() + 15));
                let mut small = vec![0xA5u8; needed - 1];
                assert_eq!(
                    decoder.decode_slice(piece, &mut small),
                    Err(base91::Error::BufferTooSmall { needed })
                );
                assert!(small.iter().all(|&b| b == 0xA5));
                let mut buf = vec![0u8; needed];
                let n = decoder.decode_slice(piece, &mut buf).unwrap();
                back.extend_from_slice(&buf[..n]);
            }
            let mut buf = [0u8; 12];
            let n = decoder.finish_slice(&mut buf).unwrap();
            back.extend_from_slice(&buf[..n]);
            assert!(back == x, "{name}, W={width}: streaming decode_slice");
        }
    }
}

// ---------------------------------------------------------------------------
// SIMD format: decoders, on oracle-produced (valid) streams
// ---------------------------------------------------------------------------

#[test]
fn simd_decode_valid_streams() {
    for (name, x) in inputs("simd_decode") {
        for width in WIDTHS {
            let stream = oracle::encode(&x, width);
            for level in LEVELS {
                let got = simd::decode(&stream, level);
                assert!(got.as_deref() == Ok(&x[..]), "{name}, W={width}, {level:?}");
            }
        }
    }
}

#[test]
fn simd_streaming_decoder_on_valid_streams() {
    let mut rng = Rng::for_test("simd_streaming_decoder");
    let mut skipped = Skipped::new("simd_streaming_decoder", Deviation::SimdStreamingDecoder);
    for (name, x) in inputs("simd_streaming_decoder") {
        for width in WIDTHS {
            let stream = oracle::encode(&x, width);
            let mut plans = vec![("one chunk".to_string(), vec![stream.len()])];
            plans.extend(chunkings(&mut rng, stream.len()));
            for (plan, lengths) in plans {
                // Bug 1 (spec 0006): only a single chunk decodes correctly.
                if lengths.len() > 1 && skipped.skip() {
                    continue;
                }
                for level in LEVELS {
                    let mut decoder = simd::Decoder::new(level);
                    let mut got = Vec::new();
                    for piece in pieces(&stream, &lengths) {
                        decoder.decode(piece, &mut got).unwrap();
                    }
                    decoder.finish(&mut got).unwrap();
                    assert!(got == x, "{name}, W={width}, {plan}, {level:?}");
                }
            }
        }
    }
}

#[test]
fn io_reader_on_simd_streams() {
    let mut rng = Rng::for_test("io_reader_simd");
    let mut skipped = Skipped::new("io_reader_simd", Deviation::SimdStreamingDecoder);
    let mut skipped_eof = Skipped::new("io_reader_simd", Deviation::IoReaderEarlyEof);
    for (name, x) in inputs("io_reader_simd") {
        for width in WIDTHS {
            let stream = oracle::encode(&x, width);
            // Bug 1 (spec 0006): streams longer than one 8 KiB read fail.
            if stream.len() <= 8192 || !skipped.skip() {
                let mut got = Vec::new();
                base91::io::DecoderReader::new(&stream[..])
                    .read_to_end(&mut got)
                    .unwrap();
                assert!(got == x, "{name}, W={width}");
            }
            // An inner reader handing out pieces, like a pipe. Needs both
            // bug 1 and bug 6 (spec 0006) fixed.
            for (plan, lengths) in chunkings(&mut rng, stream.len()) {
                if lengths.len() > 1 && (skipped.skip() || skipped_eof.skip()) {
                    continue;
                }
                let mut got = Vec::new();
                base91::io::DecoderReader::new(PieceReader::new(&stream, &lengths))
                    .read_to_end(&mut got)
                    .unwrap();
                assert!(got == x, "{name}, W={width}, inner reads: {plan}");
            }
        }
    }
}

// Calls the deprecated `*_unchecked` functions, public until 1.0 and so
// still tested (spec 0010 §7).
#[test]
#[allow(deprecated)]
fn simd_unchecked_matches_oracle() {
    let mut skipped = Skipped::new("simd_unchecked", Deviation::SimdUnchecked);
    let mut skipped_wrap = Skipped::new("simd_unchecked", Deviation::SimdWrappedEncoding);
    for (name, x) in inputs("simd_unchecked") {
        for width in WIDTHS {
            // Spec 0005: calling these corrupts memory; never call them
            // while pending.
            if skipped.skip() || (width > 0 && skipped_wrap.skip()) {
                continue;
            }
            let expected = oracle::encode(&x, width);
            for level in LEVELS {
                let mut out = Guarded::new(simd::encode_size_hint(x.len(), width));
                // SAFETY: `out` holds exactly the documented number of bytes.
                let n = unsafe { simd::encode_unchecked(&x, level, width, out.as_mut_ptr()) };
                out.check();
                assert!(out.written(n) == expected, "{name}, W={width}, {level:?}");

                let mut back = Guarded::new(simd::decode_size_hint(expected.len()));
                // SAFETY: as above.
                let n = unsafe { simd::decode_unchecked(&expected, level, back.as_mut_ptr()) };
                back.check();
                assert!(back.written(n) == x, "{name}, W={width}, {level:?}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy format, against the upstream C reference
// ---------------------------------------------------------------------------

#[cfg(feature = "reference-tests")]
mod legacy {
    use super::*;
    use base91_testkit::reference;
    use std::io::Write;

    #[test]
    fn one_shot_matches_reference() {
        for (name, x) in inputs("legacy_one_shot") {
            let expected = reference::encode(&x);
            assert!(base91::encode(&x) == expected, "{name}");
            assert!(base91::decode(&expected) == x, "{name}");
            // Auto-detection (spec 0008 §2): no `-`, so the legacy path.
            for level in LEVELS {
                let got = simd::decode(&expected, level);
                assert!(got.as_deref() == Ok(&x[..]), "{name}, {level:?}");
            }
        }
    }

    #[test]
    fn streaming_matches_reference() {
        let mut rng = Rng::for_test("legacy_streaming");
        let mut skipped = Skipped::new("legacy_streaming", Deviation::LegacyStreamingDecoder);
        for (name, x) in inputs("legacy_streaming") {
            let expected = reference::encode(&x);
            for (plan, lengths) in chunkings(&mut rng, x.len()) {
                let mut encoder = base91::Encoder::new();
                let mut got = Vec::new();
                for piece in pieces(&x, &lengths) {
                    encoder.encode(piece, &mut got);
                }
                encoder.finish(&mut got);
                assert!(got == expected, "{name}, encode {plan}");
            }
            for (plan, lengths) in chunkings(&mut rng, expected.len()) {
                // Bug 5 (spec 0006): any chunking can panic, whenever the
                // output Vec has no spare capacity left.
                if lengths.len() > 1 && skipped.skip() {
                    continue;
                }
                let mut decoder = base91::Decoder::new();
                let mut got = Vec::new();
                for piece in pieces(&expected, &lengths) {
                    decoder.decode(piece, &mut got);
                }
                decoder.finish(&mut got);
                assert!(got == x, "{name}, decode {plan}");
            }
        }
    }

    #[test]
    fn io_adapters_match_reference() {
        let mut rng = Rng::for_test("legacy_io");
        let mut skipped = Skipped::new("legacy_io", Deviation::IoReaderEarlyEof);
        for (name, x) in inputs("legacy_io") {
            let expected = reference::encode(&x);
            let mut writer = base91::io::EncoderWriter::new(Vec::new());
            for piece in pieces(&x, &rng.split_points(x.len())) {
                writer.write_all(piece).unwrap();
            }
            assert!(writer.finish().unwrap() == expected, "{name}");

            let mut got = Vec::new();
            base91::io::DecoderReader::new(&expected[..])
                .read_to_end(&mut got)
                .unwrap();
            assert!(got == x, "{name}");

            // An inner reader handing out pieces, like a pipe.
            for (plan, lengths) in chunkings(&mut rng, expected.len()) {
                // Bug 6 (spec 0006): a read that decodes to nothing ends
                // the stream early.
                if lengths.len() > 1 && skipped.skip() {
                    continue;
                }
                let mut got = Vec::new();
                base91::io::DecoderReader::new(PieceReader::new(&expected, &lengths))
                    .read_to_end(&mut got)
                    .unwrap();
                assert!(got == x, "{name}, inner reads: {plan}");
            }
        }
    }

    // Calls the deprecated `*_unchecked` functions, as above.
    #[test]
    #[allow(deprecated)]
    fn unchecked_matches_reference() {
        for (name, x) in inputs("legacy_unchecked") {
            let expected = reference::encode(&x);
            let mut out = Guarded::new(base91::encode_size_hint(x.len()));
            // SAFETY: `out` holds exactly the documented number of bytes.
            let n = unsafe { base91::encode_unchecked(&x, out.as_mut_ptr()) };
            out.check();
            assert!(out.written(n) == expected, "{name}");

            let mut back = Guarded::new(base91::decode_size_hint(expected.len()));
            // SAFETY: as above.
            let n = unsafe { base91::decode_unchecked(&expected, back.as_mut_ptr()) };
            back.check();
            assert!(back.written(n) == x, "{name}");
        }
    }

    /// The legacy slice API (spec 0010 §2), into canary-guarded buffers of
    /// exactly the size hints.
    #[test]
    fn slice_api_matches_reference() {
        for (name, x) in inputs("legacy_slice_api") {
            let expected = reference::encode(&x);
            let hint = base91::encode_size_hint(x.len());
            let mut out = Guarded::new(hint);
            // SAFETY: `out` holds `hint` initialized bytes.
            let buf = unsafe { std::slice::from_raw_parts_mut(out.as_mut_ptr(), hint) };
            let n = base91::encode_slice(&x, buf).unwrap();
            out.check();
            assert!(out.written(n) == expected, "{name}");

            let hint = base91::decode_size_hint(expected.len());
            let mut back = Guarded::new(hint);
            // SAFETY: as above.
            let buf = unsafe { std::slice::from_raw_parts_mut(back.as_mut_ptr(), hint) };
            let n = base91::decode_slice(&expected, buf).unwrap();
            back.check();
            assert!(back.written(n) == x, "{name}");
        }
    }
}
