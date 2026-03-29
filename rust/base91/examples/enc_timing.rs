// SPDX-FileCopyrightText: 2026 2025 - 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

use std::time::Instant;

fn time_fn(name: &str, size: usize, reps: usize, mut f: impl FnMut()) {
    for _ in 0..3 {
        f();
    } // warm up
    let t = Instant::now();
    for _ in 0..reps {
        f();
    }
    let ns = t.elapsed().as_nanos() / reps as u128;
    let mbs = size as f64 / (ns as f64 / 1e9) / 1e6;
    println!("{name:30}: {mbs:7.0} MB/s  ({ns} ns/call)");
}

fn main() {
    const SIZE: usize = 4 * 1024 * 1024;
    const REPS: usize = 50;
    let input: Vec<u8> = (0u8..=255).cycle().take(SIZE).collect();

    time_fn("scalar simd encode", SIZE, REPS, || {
        let _ = base91::simd::encode(&input, base91::simd::SimdLevel::Scalar, 0);
    });
    time_fn("henke encode (safe)", SIZE, REPS, || {
        let _ = base91::encode(&input);
    });
    time_fn("henke encode_unchecked", SIZE, REPS, || {
        let mut buf = vec![0u8; base91::encode_size_hint(input.len())];
        unsafe {
            base91::encode_unchecked(&input, buf.as_mut_ptr());
        }
    });
}
