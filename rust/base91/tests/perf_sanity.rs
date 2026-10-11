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
//! Measures legacy and SIMD encode/decode throughput on 512 KiB of seeded
//! random data (fastest of 51 runs, after a warm-up run, into buffers
//! allocated beforehand; input and output fit a 2 MB L2 cache), prints a
//! table, and asserts only **ratios**
//! between levels and layouts, which hold across machines far better than
//! absolute numbers. Levels the CPU lacks are skipped and reported.

use base91::simd::{self, SimdLevel};
use base91_testkit::rng::Rng;
use std::hint::black_box;
use std::time::Instant;

const SIZE: usize = 512 << 10;
const RUNS: usize = 51;

/// Minimum ratios (encode, decode), calibrated on the dev machine at 70 % of
/// the lowest ratio measured (spec 0009, decision 6), rounded down. Dev
/// machine, 2026-10-11: Intel Core Ultra 7 165U, measured on a performance
/// core (`taskset -c 8` in the dev VM), rustc 1.98.1, kernels of spec 0011
/// with the pack-free interleave of spec 0012, MB/s encode / decode, six
/// runs:
///
/// | | encode | decode |
/// |---|---|---|
/// | legacy | 1011–1077 | 1159–1186 |
/// | Scalar, W=0 | 3047–3118 | 2246–2282 |
/// | Simd128, W=0 | 9682–9694 | 10163–10370 |
/// | Simd256, W=0 | 19689–21059 | 19098–21227 |
/// | Simd256, W=64 | 16934–18162 | 16929–17746 |
///
/// Ratios, lowest–highest over six runs: Simd128/Scalar 3.11–3.18 /
/// 4.45–4.55; Simd256/Simd128 2.03–2.17 / 1.88–2.09; wrapped/unwrapped
/// 0.80–0.90 / 0.80–0.93. On an efficiency core (`taskset -c 4`):
/// 4.26–4.79 / 5.51–6.16; 1.02–1.03 / 1.04–1.16; 0.94–0.95 / 0.92–0.93.
///
/// The Simd256/Simd128 thresholds are capped at 0.90 instead of 70 %:
/// machines whose AVX2 units are half-width (efficiency cores, older AMD)
/// run AVX2 at about the SSE4.1 speed, and the guard is only meant to catch
/// AVX2 falling behind SSE4.1.
///
/// With these measurement conditions the ratios vary by a few hundredths
/// between runs, so the guards catch a code-generation regression like the
/// ones recorded in spec 0011 ("Code-generation sensitivity") when it moves
/// a ratio by more than about a third. The one of rustc 1.98 (spec 0012:
/// `_mm_packus_epi16` became a clamp and a pack, 10–14 % on SIMD encode)
/// stayed under the 1.91 thresholds.
const MIN_SIMD128_OVER_SCALAR: (f64, f64) = (2.17, 3.11);
const MIN_SIMD256_OVER_SIMD128: (f64, f64) = (0.90, 0.90);
const MIN_WRAPPED_OVER_UNWRAPPED: (f64, f64) = (0.56, 0.56);

/// Throughput of `f`, in MB/s of original data: the fastest of `RUNS`
/// runs, after one warm-up run.
///
/// The minimum, not the median: for CPU-bound code it is the most
/// repeatable figure (every disturbance only adds time). The functions
/// timed write into buffers allocated and touched beforehand, so page
/// faults on fresh allocations stay out of the measurement.
fn throughput(mut f: impl FnMut() -> usize) -> f64 {
    black_box(f());
    let best = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            black_box(f());
            start.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min);
    SIZE as f64 / best / 1e6
}

struct Rates {
    encode: f64,
    decode: f64,
}

fn simd_rates(x: &[u8], level: SimdLevel, width: usize) -> Rates {
    let stream = simd::encode(x, level, width).unwrap();
    let mut enc = vec![0u8; stream.len()];
    let mut dec = vec![0u8; simd::decode_size_hint(stream.len())];
    Rates {
        encode: throughput(|| simd::encode_slice(black_box(x), level, width, &mut enc).unwrap()),
        decode: throughput(|| simd::decode_slice(black_box(&stream), level, &mut dec).unwrap()),
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
    let mut enc = vec![0u8; base91::encode_size_hint(x.len())];
    let mut dec = vec![0u8; base91::decode_size_hint(legacy_stream.len())];
    let legacy = Rates {
        encode: throughput(|| base91::encode_slice(black_box(&x), &mut enc).unwrap()),
        decode: throughput(|| base91::decode_slice(black_box(&legacy_stream), &mut dec).unwrap()),
    };

    eprintln!("simd::detect() = {best:?}; MB/s of original data, fastest of {RUNS}");
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
