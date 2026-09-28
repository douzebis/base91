// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Property tests (spec 0009 §5): the invariants of the §2 matrix over
//! generated inputs, with shrinking to minimal counterexamples.
//!
//! Seeded from `BASE91_TEST_SEED` like the rest of the suite;
//! `BASE91_TEST_FULL=1` runs 16x more cases. Failure persistence is off
//! (the Nix sandbox's source tree is read-only); a failure prints its
//! shrunk input instead.

use base91::simd::{self, SimdLevel};
use base91_testkit::oracle::{self, DecodeError};
use base91_testkit::pending::{is_pending, Deviation};
use base91_testkit::rng::{full_matrix, seed};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};

fn config() -> Config {
    Config {
        cases: if full_matrix() { 4096 } else { 256 },
        rng_seed: RngSeed::Fixed(seed()),
        failure_persistence: None,
        // proptest's automatic limit is 4 x cases, and its vector shrinker
        // deletes one element per iteration, so a failing input of a few
        // thousand bytes would stop shrinking long before its minimal form.
        // Shrinking only runs after a failure, so this costs nothing on
        // passing runs.
        max_shrink_iters: 100_000,
        ..Config::default()
    }
}

fn level() -> impl Strategy<Value = SimdLevel> {
    prop_oneof![
        Just(SimdLevel::Scalar),
        Just(SimdLevel::Simd128),
        Just(SimdLevel::Simd256),
    ]
}

fn width() -> impl Strategy<Value = usize> {
    prop_oneof![Just(0), Just(32), Just(64), Just(96)]
}

/// Input bytes plus cut points, for splitting the input into chunks.
fn bytes_and_cuts(max: usize) -> impl Strategy<Value = (Vec<u8>, Vec<usize>)> {
    vec(any::<u8>(), 0..max).prop_flat_map(|x| {
        let n = x.len();
        (Just(x), vec(0..=n, 0..8))
    })
}

/// `data` split at the (unsorted, possibly repeated) `cuts`; empty pieces
/// are kept on purpose.
/// Streams that look like SIMD streams: `-`, then mostly alphabet
/// characters, with terminators, spaces and arbitrary bytes mixed in.
fn simd_like() -> impl Strategy<Value = Vec<u8>> {
    let byte = prop_oneof![
        40 => proptest::sample::select(&oracle::ALPHABET[..]),
        2 => Just(b'\n'),
        1 => Just(b'\r'),
        1 => Just(b' '),
        1 => any::<u8>(),
    ];
    vec(byte, 0..400).prop_map(|body| [&b"-"[..], &body].concat())
}

/// The oracle's result, as the library reports it.
fn expected(stream: &[u8]) -> Result<Vec<u8>, base91::Error> {
    oracle::decode(stream).map_err(|e| match e {
        DecodeError::InvalidCharacter { offset } => base91::Error::InvalidCharacter { offset },
        DecodeError::InvalidLayout { offset } => base91::Error::InvalidLayout { offset },
        DecodeError::InvalidLength => base91::Error::InvalidLength,
        DecodeError::InvalidValue { offset } => base91::Error::InvalidValue { offset },
    })
}

fn split<'a>(data: &'a [u8], cuts: &[usize]) -> Vec<&'a [u8]> {
    let mut cuts = cuts.to_vec();
    cuts.sort_unstable();
    let mut out = Vec::new();
    let mut start = 0;
    for cut in cuts {
        out.push(&data[start..cut]);
        start = cut;
    }
    out.push(&data[start..]);
    out
}

proptest! {
    #![proptest_config(config())]

    /// Unwrapped SIMD encoding equals the oracle, at every level.
    #[test]
    fn simd_encode_matches_oracle(x in vec(any::<u8>(), 0..2000), level in level()) {
        prop_assert_eq!(simd::encode(&x, level, 0), Ok(oracle::encode(&x, 0)));
    }

    /// Wrapped SIMD encoding equals the oracle (spec 0006, bug 3).
    #[test]
    fn simd_wrapped_encode_matches_oracle(
        x in vec(any::<u8>(), 0..2000),
        level in level(),
        width in prop_oneof![Just(32usize), Just(64), Just(96)],
    ) {
        if !is_pending(Deviation::SimdWrappedEncoding) {
            prop_assert_eq!(simd::encode(&x, level, width), Ok(oracle::encode(&x, width)));
        }
    }

    /// Every valid stream decodes back, at every level and width.
    #[test]
    fn simd_decode_round_trips(
        x in vec(any::<u8>(), 0..2000),
        level in level(),
        width in width(),
    ) {
        let stream = oracle::encode(&x, width);
        prop_assert_eq!(simd::decode(&stream, level), Ok(x));
    }

    /// Streaming SIMD encoding with any split equals the one-shot result.
    #[test]
    fn simd_streaming_encode_any_split((x, cuts) in bytes_and_cuts(1000), level in level()) {
        // Bug 4 (spec 0006): an empty stream fed through empty calls.
        if !x.is_empty() || !is_pending(Deviation::SimdStreamingEncoderEmpty) {
            let mut encoder = simd::Encoder::new(level, 0).unwrap();
            let mut got = Vec::new();
            for piece in split(&x, &cuts) {
                encoder.encode(piece, &mut got);
            }
            encoder.finish(&mut got);
            prop_assert_eq!(got, oracle::encode(&x, 0));
        }
    }

    /// Streaming SIMD decoding with any split equals the input (bug 1).
    #[test]
    fn simd_streaming_decode_any_split(
        (x, cuts) in bytes_and_cuts(1000),
        level in level(),
        width in width(),
    ) {
        if !is_pending(Deviation::SimdStreamingDecoder) {
            let stream = oracle::encode(&x, width);
            let cuts: Vec<usize> = cuts.iter().map(|&c| c * stream.len() / x.len().max(1)).collect();
            let mut decoder = simd::Decoder::new(level);
            let mut got = Vec::new();
            for piece in split(&stream, &cuts) {
                prop_assert!(decoder.decode(piece, &mut got).is_ok());
            }
            prop_assert!(decoder.finish(&mut got).is_ok());
            prop_assert_eq!(got, x);
        }
    }

    /// Legacy streaming encoding with any split equals the one-shot result.
    #[test]
    fn legacy_streaming_encode_any_split((x, cuts) in bytes_and_cuts(1000)) {
        let mut encoder = base91::Encoder::new();
        let mut got = Vec::new();
        for piece in split(&x, &cuts) {
            encoder.encode(piece, &mut got);
        }
        encoder.finish(&mut got);
        prop_assert_eq!(got, base91::encode(&x));
    }

    /// Legacy streaming decoding with any split equals the input (bug 5).
    #[test]
    fn legacy_streaming_decode_any_split((x, cuts) in bytes_and_cuts(1000)) {
        if !is_pending(Deviation::LegacyStreamingDecoder) {
            let stream = base91::encode(&x);
            let cuts: Vec<usize> = cuts.iter().map(|&c| c * stream.len() / x.len().max(1)).collect();
            let mut decoder = base91::Decoder::new();
            let mut got = Vec::new();
            for piece in split(&stream, &cuts) {
                decoder.decode(piece, &mut got);
            }
            decoder.finish(&mut got);
            prop_assert_eq!(got, x);
        }
    }

    /// SIMD-like streams, valid or not: the decoders give the oracle's
    /// result, bytes or error (class and offset), one-shot and with any
    /// split (strict decoding, spec 0008 §3.5, annex A9–A13b).
    #[test]
    fn simd_decode_arbitrary_matches_oracle((s, cuts) in simd_like().prop_flat_map(|s| {
        let n = s.len();
        (Just(s), vec(0..=n, 0..8))
    }), level in level()) {
        let want = expected(&s);
        prop_assert_eq!(simd::decode(&s, level), want.clone());
        let mut decoder = simd::Decoder::new(level);
        let mut got = Vec::new();
        let result = split(&s, &cuts)
            .into_iter()
            .try_for_each(|piece| decoder.decode(piece, &mut got))
            .and_then(|()| decoder.finish(&mut got))
            .map(|()| got);
        prop_assert_eq!(result, want);
    }

    /// Wrapped streaming encoding with any split equals the oracle.
    #[test]
    fn simd_wrapped_streaming_encode_any_split(
        (x, cuts) in bytes_and_cuts(1000),
        level in level(),
        width in width(),
    ) {
        let mut encoder = simd::Encoder::new(level, width).unwrap();
        let mut got = Vec::new();
        for piece in split(&x, &cuts) {
            encoder.encode(piece, &mut got);
        }
        encoder.finish(&mut got);
        prop_assert_eq!(got, oracle::encode(&x, width));
    }

    /// The slice API writes what the Vec API returns, and the exact sizes
    /// it requires are the ones it writes.
    #[test]
    fn slice_api_matches_vec_api(x in vec(any::<u8>(), 0..2000), level in level(), width in width()) {
        let encoded = simd::encode(&x, level, width).unwrap();
        prop_assert_eq!(simd::encoded_len(x.len(), width), Ok(encoded.len()));
        let mut buf = vec![0u8; encoded.len()];
        prop_assert_eq!(simd::encode_slice(&x, level, width, &mut buf), Ok(encoded.len()));
        prop_assert_eq!(&buf, &encoded);

        let mut back = vec![0u8; simd::decode_size_hint(encoded.len())];
        let n = simd::decode_slice(&encoded, level, &mut back).unwrap();
        prop_assert_eq!(&back[..n], &x[..]);
    }
}

#[cfg(feature = "reference-tests")]
mod legacy {
    use super::*;
    use base91_testkit::reference;

    proptest! {
        #![proptest_config(config())]

        /// Legacy one-shot encoding and decoding equal upstream.
        #[test]
        fn legacy_matches_reference(x in vec(any::<u8>(), 0..2000)) {
            let expected = reference::encode(&x);
            prop_assert_eq!(base91::encode(&x), expected.clone());
            prop_assert_eq!(base91::decode(&expected), x);
        }

        /// Legacy decoding of arbitrary bytes equals upstream (junk ignored).
        #[test]
        fn legacy_decode_arbitrary_matches_reference(s in vec(any::<u8>(), 0..500)) {
            prop_assert_eq!(base91::decode(&s), reference::decode(&s));
        }
    }
}
