// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! The `*_unchecked` functions (spec 0005 §2, §3): with a buffer of exactly
//! the documented size, they write the same bytes as the `Vec` API, inside
//! the buffer, and the size hints bound that output.
//!
//! Uses no `std`-only API of base91-rs, so it also runs under
//! `--no-default-features --features alloc` (compile-time SIMD detection).

use base91::simd::{self, SimdLevel};
use base91_testkit::canary::Guarded;
use base91_testkit::rng::Rng;

const LEVELS: [SimdLevel; 3] = [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256];
const WRAPS: [usize; 3] = [0, 32, 64];
/// Covers every block boundary (13, 16, 26, 32 bytes) several times over.
const MAX_LEN: usize = 1100;

fn inputs() -> impl Iterator<Item = Vec<u8>> {
    let mut rng = Rng::for_test("unchecked");
    (0..=MAX_LEN).map(move |n| rng.bytes(n))
}

/// `simd::decode_unchecked` into a buffer of exactly the documented size.
fn simd_decode_unchecked(stream: &[u8], level: SimdLevel) -> Vec<u8> {
    let hint = simd::decode_size_hint(stream.len());
    let mut out = Guarded::new(hint);
    // SAFETY: `out` holds exactly `decode_size_hint(stream.len())` bytes.
    let n = unsafe { simd::decode_unchecked(stream, level, out.as_mut_ptr()) };
    out.check();
    out.written(n).to_vec()
}

#[test]
fn simd_unchecked_matches_vec_api() {
    for x in inputs() {
        let n = x.len();
        let henke = base91::encode(&x);
        for level in LEVELS {
            for wrap in WRAPS {
                let what = format!("n={n}, wrap={wrap}, {level:?}");
                let expected = simd::encode(&x, level, wrap);
                let hint = simd::encode_size_hint(n, wrap);
                assert!(expected.len() <= hint, "encode hint too small: {what}");

                let mut out = Guarded::new(hint);
                // SAFETY: `out` holds exactly `encode_size_hint(n, wrap)` bytes.
                let written = unsafe { simd::encode_unchecked(&x, level, wrap, out.as_mut_ptr()) };
                out.check();
                assert!(out.written(written) == expected, "encode: {what}");

                let decoded = simd::decode(&expected, level).unwrap();
                assert!(
                    decoded.len() <= simd::decode_size_hint(expected.len()),
                    "decode hint too small: {what}"
                );
                assert!(
                    simd_decode_unchecked(&expected, level) == decoded,
                    "decode: {what}"
                );
            }

            // The Henke fallback path.
            let what = format!("n={n}, Henke stream, {level:?}");
            let decoded = simd::decode(&henke, level).unwrap();
            assert!(
                decoded.len() <= simd::decode_size_hint(henke.len()),
                "decode hint too small: {what}"
            );
            assert!(
                simd_decode_unchecked(&henke, level) == decoded,
                "decode: {what}"
            );
        }
    }
}

#[test]
fn henke_unchecked_matches_vec_api() {
    for x in inputs() {
        let n = x.len();
        let expected = base91::encode(&x);
        let hint = base91::encode_size_hint(n);
        assert!(expected.len() <= hint, "encode hint too small: n={n}");

        let mut out = Guarded::new(hint);
        // SAFETY: `out` holds exactly `encode_size_hint(n)` bytes.
        let written = unsafe { base91::encode_unchecked(&x, out.as_mut_ptr()) };
        out.check();
        assert!(out.written(written) == expected, "encode: n={n}");

        let decoded = base91::decode(&expected);
        let hint = base91::decode_size_hint(expected.len());
        assert!(decoded.len() <= hint, "decode hint too small: n={n}");
        let mut back = Guarded::new(hint);
        // SAFETY: `back` holds exactly `decode_size_hint(expected.len())` bytes.
        let written = unsafe { base91::decode_unchecked(&expected, back.as_mut_ptr()) };
        back.check();
        assert!(back.written(written) == decoded, "decode: n={n}");
    }
}
