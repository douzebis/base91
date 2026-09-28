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
use base91_testkit::oracle;
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

    /// Any byte string: SIMD decoders agree with the oracle (strict
    /// decoder, spec 0008 annex A9–A13b).
    #[test]
    fn simd_decode_arbitrary_matches_oracle(s in vec(any::<u8>(), 0..300)) {
        if !is_pending(Deviation::SimdStrictDecoding)
            && oracle::classify(&s) != oracle::Classification::Legacy
        {
            if let Ok(expected) = oracle::decode(&s) {
                prop_assert_eq!(simd::decode(&s, SimdLevel::default()), Ok(expected));
            }
        }
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
