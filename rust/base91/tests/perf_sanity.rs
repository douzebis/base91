// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Relative performance sanity test (spec 0009 §10).
//!
//! Ignored by default; run in release mode, without debug assertions:
//!
//! ```text
//! cargo test --release -p base91-rs --test perf_sanity -- --ignored --nocapture
//! ```
//!
//! Measures legacy and SIMD encode/decode throughput on 4 MB of seeded random
//! data (median of 7 runs), prints a table, and asserts only **ratios**
//! between levels and layouts, which hold across machines far better than
//! absolute numbers. Levels the CPU lacks are skipped and reported.

use base91::simd::{self, SimdLevel};
use base91_testkit::rng::Rng;
use std::hint::black_box;
use std::time::Instant;

const SIZE: usize = 4 << 20;
const RUNS: usize = 7;

/// Minimum ratios (encode, decode), calibrated on the dev machine at 70 % of
/// the lowest ratio measured over three runs (spec 0009, decision 6). Dev
/// machine, 2026-09-27, x86_64 with AVX2, MB/s encode / decode:
///
/// | | encode | decode |
/// |---|---|---|
/// | legacy | ~550 | ~490 |
/// | Scalar, W=0 | ~1020 | ~1110 |
/// | Simd128, W=0 | ~2390 | ~3100 |
/// | Simd256, W=0 | 2090–2520 | ~3370 |
/// | Simd256, W=64 | ~860 | ~550 |
///
/// Measured ratios: Simd128/Scalar 2.34 / 2.65–2.98; Simd256/Simd128
/// 0.87–1.06 / 1.08–1.09; wrapped/unwrapped 0.34–0.41 / 0.16–0.17. The
/// wrapping ratios are low because 0.2.2 wraps with a separate insertion
/// pass and unwraps with a copy; raise them once spec 0008's in-place
/// layout is implemented.
const MIN_SIMD128_OVER_SCALAR: (f64, f64) = (1.64, 1.86);
const MIN_SIMD256_OVER_SIMD128: (f64, f64) = (0.61, 0.76);
const MIN_WRAPPED_OVER_UNWRAPPED: (f64, f64) = (0.24, 0.11);

/// Median throughput of `f` over `RUNS` runs, in MB/s of original data.
fn throughput(mut f: impl FnMut() -> usize) -> f64 {
    let mut rates: Vec<f64> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            black_box(f());
            SIZE as f64 / start.elapsed().as_secs_f64() / 1e6
        })
        .collect();
    rates.sort_by(f64::total_cmp);
    rates[RUNS / 2]
}

struct Rates {
    encode: f64,
    decode: f64,
}

fn simd_rates(x: &[u8], level: SimdLevel, width: usize) -> Rates {
    let stream = simd::encode(x, level, width).unwrap();
    Rates {
        encode: throughput(|| simd::encode(black_box(x), level, width).unwrap().len()),
        decode: throughput(|| simd::decode(black_box(&stream), level).unwrap().len()),
    }
}

fn check_ratio(what: &str, ratio: f64, min: f64, failures: &mut Vec<String>) {
    let verdict = if ratio >= min { "ok" } else { "TOO LOW" };
    eprintln!("  {what:<44} {ratio:6.2}  (min {min:.2})  {verdict}");
    if ratio < min {
        failures.push(format!("{what}: {ratio:.2} < {min:.2}"));
    }
}

#[test]
#[ignore = "performance test: run with --release -- --ignored"]
fn simd_levels_keep_their_relative_speed() {
    let x = Rng::new(0x0009_0010).bytes(SIZE);
    let best = simd::detect();

    let legacy_stream = base91::encode(&x);
    let legacy = Rates {
        encode: throughput(|| base91::encode(black_box(&x)).len()),
        decode: throughput(|| base91::decode(black_box(&legacy_stream)).len()),
    };

    eprintln!("simd::detect() = {best:?}; MB/s of original data, median of {RUNS}");
    eprintln!("  {:<24} {:>10} {:>10}", "", "encode", "decode");
    eprintln!(
        "  {:<24} {:>10.0} {:>10.0}",
        "legacy", legacy.encode, legacy.decode
    );

    let mut table = Vec::new();
    for level in [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256] {
        if level > best {
            eprintln!("  {level:?}: skipped (CPU lacks it)");
            continue;
        }
        for width in [0, 64] {
            let r = simd_rates(&x, level, width);
            eprintln!(
                "  {:<24} {:>10.0} {:>10.0}",
                format!("{level:?}, W={width}"),
                r.encode,
                r.decode
            );
            table.push((level, width, r));
        }
    }
    let get = |level, width| {
        table
            .iter()
            .find(|(l, w, _)| *l == level && *w == width)
            .map(|(_, _, r)| r)
    };

    let mut failures = Vec::new();
    eprintln!("ratios:");
    if let (Some(s), Some(m)) = (get(SimdLevel::Scalar, 0), get(SimdLevel::Simd128, 0)) {
        let (e, d) = (m.encode / s.encode, m.decode / s.decode);
        check_ratio(
            "encode Simd128 / Scalar",
            e,
            MIN_SIMD128_OVER_SCALAR.0,
            &mut failures,
        );
        check_ratio(
            "decode Simd128 / Scalar",
            d,
            MIN_SIMD128_OVER_SCALAR.1,
            &mut failures,
        );
    }
    if let (Some(m), Some(l)) = (get(SimdLevel::Simd128, 0), get(SimdLevel::Simd256, 0)) {
        let (e, d) = (l.encode / m.encode, l.decode / m.decode);
        check_ratio(
            "encode Simd256 / Simd128",
            e,
            MIN_SIMD256_OVER_SIMD128.0,
            &mut failures,
        );
        check_ratio(
            "decode Simd256 / Simd128",
            d,
            MIN_SIMD256_OVER_SIMD128.1,
            &mut failures,
        );
    }
    if let (Some(u), Some(w)) = (get(best, 0), get(best, 64)) {
        let (e, d) = (w.encode / u.encode, w.decode / u.decode);
        check_ratio(
            "encode wrapped (W=64) / unwrapped, best",
            e,
            MIN_WRAPPED_OVER_UNWRAPPED.0,
            &mut failures,
        );
        check_ratio(
            "decode wrapped (W=64) / unwrapped, best",
            d,
            MIN_WRAPPED_OVER_UNWRAPPED.1,
            &mut failures,
        );
    }
    eprintln!(
        "  (reported only) SIMD Scalar / legacy: encode {:.2}, decode {:.2}",
        get(SimdLevel::Scalar, 0).map_or(f64::NAN, |s| s.encode / legacy.encode),
        get(SimdLevel::Scalar, 0).map_or(f64::NAN, |s| s.decode / legacy.decode),
    );
    assert!(failures.is_empty(), "ratios below threshold: {failures:#?}");
}
