// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Quick benchmark for performance work (spec 0011, §1): seconds, not
//! minutes, per round.
//!
//! Times the slice API on 512 KiB of seeded random data (input and output
//! fit a 2 MB L2 cache): one warm-up run, then the fastest of 101 runs, with
//! no allocation in the timed region. Pin it to a core and keep the machine
//! otherwise idle:
//!
//! ```text
//! cargo build --release --example quick_bench
//! taskset -c 8 target/release/examples/quick_bench [CASE...]
//! ```
//!
//! A case is `enc` or `dec`, a level (`scalar`, `128`, `256`) and a line
//! width (`w0`, `w64`), e.g. `dec256w64`; with no argument, all twelve.
//! Prints one line per case: the case and the fastest run in µs.

use base91::simd::{self, SimdLevel};
use std::hint::black_box;
use std::time::Instant;

const SIZE: usize = 512 << 10;
const RUNS: usize = 101;

fn parse(case: &str) -> Option<(bool, SimdLevel, usize)> {
    let (encode, rest) = if let Some(rest) = case.strip_prefix("enc") {
        (true, rest)
    } else {
        (false, case.strip_prefix("dec")?)
    };
    let (level, rest) = if let Some(rest) = rest.strip_prefix("scalar") {
        (SimdLevel::Scalar, rest)
    } else if let Some(rest) = rest.strip_prefix("128") {
        (SimdLevel::Simd128, rest)
    } else {
        (SimdLevel::Simd256, rest.strip_prefix("256")?)
    };
    let wrap = rest.strip_prefix('w')?.parse().ok()?;
    Some((encode, level, wrap))
}

/// Fastest of `RUNS` runs of `f`, after one warm-up run, in µs.
fn fastest(mut f: impl FnMut() -> usize) -> f64 {
    black_box(f());
    (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            black_box(f());
            start.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
        * 1e6
}

fn main() {
    let mut cases: Vec<String> = std::env::args().skip(1).collect();
    if cases.is_empty() {
        for dir in ["enc", "dec"] {
            for level in ["scalar", "128", "256"] {
                for wrap in ["w0", "w64"] {
                    cases.push(format!("{dir}{level}{wrap}"));
                }
            }
        }
    }
    // Deterministic data, as in `tests/perf_sanity.rs`.
    let mut state = 0x0009_0010_u64;
    let input: Vec<u8> = (0..SIZE)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    for case in cases {
        let Some((encode, level, wrap)) = parse(&case) else {
            eprintln!("unknown case: {case}");
            std::process::exit(2);
        };
        let stream = simd::encode(&input, level, wrap).unwrap();
        let mut enc = vec![0u8; stream.len()];
        let mut dec = vec![0u8; simd::decode_size_hint(stream.len())];
        let us = if encode {
            fastest(|| simd::encode_slice(black_box(&input), level, wrap, &mut enc).unwrap())
        } else {
            fastest(|| simd::decode_slice(black_box(&stream), level, &mut dec).unwrap())
        };
        println!("{case:12} {us:8.1}");
    }
}
