// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Profiling harness — run one path for many iterations so perf/callgrind
//! can collect samples.  Usage:
//!
//!   cargo build --release --example enc_timing
//!   perf record -e cpu-clock -g \
//!       ./target/release/examples/enc_timing <path>
//!
//! <path> is one of:
//!   henke_enc_unchecked  henke_enc_safe  henke_dec_unchecked  henke_dec_safe
//!   scalar_enc  scalar_dec
//!   simd128_enc  simd128_dec
//!   simd256_enc  simd256_dec

use base91::simd::SimdLevel;
use std::hint::black_box;

const SIZE: usize = 4 * 1024 * 1024;
const REPS: usize = 200;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: enc_timing <path>");
        std::process::exit(1);
    });

    let input: Vec<u8> = (0u8..=255).cycle().take(SIZE).collect();

    // Pre-build encoded buffers needed by decode paths.
    let henke_encoded = base91::encode(&input);
    let simd_encoded = base91::simd::encode(&input, SimdLevel::default(), 0);

    let enc_buf_len = base91::encode_size_hint(input.len());
    let dec_buf_len = base91::decode_size_hint(henke_encoded.len());

    match path.as_str() {
        "henke_enc_unchecked" => {
            let mut buf = vec![0u8; enc_buf_len];
            for _ in 0..REPS {
                unsafe { base91::encode_unchecked(black_box(&input), buf.as_mut_ptr()) };
                black_box(&buf);
            }
        }
        "henke_enc_safe" => {
            for _ in 0..REPS {
                black_box(base91::encode(black_box(&input)));
            }
        }
        "henke_dec_unchecked" => {
            let mut buf = vec![0u8; dec_buf_len];
            for _ in 0..REPS {
                unsafe { base91::decode_unchecked(black_box(&henke_encoded), buf.as_mut_ptr()) };
                black_box(&buf);
            }
        }
        "henke_dec_safe" => {
            for _ in 0..REPS {
                black_box(base91::decode(black_box(&henke_encoded)));
            }
        }
        "scalar_enc" => {
            for _ in 0..REPS {
                black_box(base91::simd::encode(
                    black_box(&input),
                    SimdLevel::Scalar,
                    0,
                ));
            }
        }
        "scalar_dec" => {
            for _ in 0..REPS {
                black_box(base91::simd::decode(
                    black_box(&simd_encoded),
                    SimdLevel::Scalar,
                ));
            }
        }
        "simd128_enc" => {
            for _ in 0..REPS {
                black_box(base91::simd::encode(
                    black_box(&input),
                    SimdLevel::Simd128,
                    0,
                ));
            }
        }
        "simd128_dec" => {
            for _ in 0..REPS {
                black_box(base91::simd::decode(
                    black_box(&simd_encoded),
                    SimdLevel::Simd128,
                ));
            }
        }
        "simd256_enc" => {
            for _ in 0..REPS {
                black_box(base91::simd::encode(
                    black_box(&input),
                    SimdLevel::Simd256,
                    0,
                ));
            }
        }
        "simd256_dec" => {
            for _ in 0..REPS {
                black_box(base91::simd::decode(
                    black_box(&simd_encoded),
                    SimdLevel::Simd256,
                ));
            }
        }
        other => {
            eprintln!("unknown path: {other}");
            std::process::exit(1);
        }
    }
}
