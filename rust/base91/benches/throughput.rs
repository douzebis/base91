// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::RngCore;

// competitor crate (base91 v0.1.0 by dnsl48)
use base91_dnsl48 as base91_other;

// ---------------------------------------------------------------------------
// C reference FFI — only available with --features c-compat-tests
// (build.rs compiles src/base91.c and renames symbols to c_basE91_*)
// ---------------------------------------------------------------------------

#[cfg(feature = "c-compat-tests")]
mod c_ref {
    #[repr(C)]
    pub struct BasE91State {
        pub queue: u64,
        pub nbits: u32,
        pub val: i32,
    }

    extern "C" {
        pub fn c_basE91_init(b: *mut BasE91State);
        pub fn c_basE91_encode(b: *mut BasE91State, i: *const u8, len: usize, o: *mut u8) -> usize;
        pub fn c_basE91_encode_end(b: *mut BasE91State, o: *mut u8) -> usize;
        pub fn c_basE91_decode(b: *mut BasE91State, i: *const u8, len: usize, o: *mut u8) -> usize;
        pub fn c_basE91_decode_end(b: *mut BasE91State, o: *mut u8) -> usize;
    }

    pub unsafe fn encode(input: &[u8], output: *mut u8) -> usize {
        let mut state = std::mem::MaybeUninit::<BasE91State>::uninit();
        c_basE91_init(state.as_mut_ptr());
        let mut state = state.assume_init();
        let n = c_basE91_encode(&mut state, input.as_ptr(), input.len(), output);
        let m = c_basE91_encode_end(&mut state, output.add(n));
        n + m
    }

    pub unsafe fn decode(input: &[u8], output: *mut u8) -> usize {
        let mut state = std::mem::MaybeUninit::<BasE91State>::uninit();
        c_basE91_init(state.as_mut_ptr());
        let mut state = state.assume_init();
        let n = c_basE91_decode(&mut state, input.as_ptr(), input.len(), output);
        let m = c_basE91_decode_end(&mut state, output.add(n));
        n + m
    }
}

// ---------------------------------------------------------------------------
// Benchmark setup
// ---------------------------------------------------------------------------

const SIZE: usize = 1024 * 1024; // 1 MiB

fn random_bytes(size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

fn bench_encode(c: &mut Criterion) {
    let input = random_bytes(SIZE);
    let mut enc_buf = vec![0u8; base91::encode_size_hint(input.len())];

    let mut g = c.benchmark_group("encode");
    g.throughput(Throughput::Bytes(input.len() as u64));

    g.bench_with_input(
        BenchmarkId::new("rust_unchecked", "1mib"),
        &input,
        |b, input| {
            b.iter(|| unsafe { base91::encode_unchecked(input, enc_buf.as_mut_ptr()) });
        },
    );

    g.bench_with_input(BenchmarkId::new("rust_safe", "1mib"), &input, |b, input| {
        b.iter(|| base91::encode(input));
    });

    g.bench_with_input(
        BenchmarkId::new("base91_dnsl48", "1mib"),
        &input,
        |b, input| {
            b.iter(|| base91_other::slice_encode(input));
        },
    );

    #[cfg(feature = "c-compat-tests")]
    g.bench_with_input(
        BenchmarkId::new("c_reference", "1mib"),
        &input,
        |b, input| {
            b.iter(|| unsafe { c_ref::encode(input, enc_buf.as_mut_ptr()) });
        },
    );

    g.finish();
}

fn bench_decode(c: &mut Criterion) {
    let input = random_bytes(SIZE);
    let encoded = {
        let mut buf = vec![0u8; base91::encode_size_hint(input.len())];
        let n = unsafe { base91::encode_unchecked(&input, buf.as_mut_ptr()) };
        buf.truncate(n);
        buf
    };
    let mut dec_buf = vec![0u8; base91::decode_size_hint(encoded.len())];

    let mut g = c.benchmark_group("decode");
    g.throughput(Throughput::Bytes(encoded.len() as u64));

    g.bench_with_input(
        BenchmarkId::new("rust_unchecked", "1mib"),
        &encoded,
        |b, encoded| {
            b.iter(|| unsafe { base91::decode_unchecked(encoded, dec_buf.as_mut_ptr()) });
        },
    );

    g.bench_with_input(
        BenchmarkId::new("rust_safe", "1mib"),
        &encoded,
        |b, encoded| b.iter(|| base91::decode(encoded)),
    );

    g.bench_with_input(
        BenchmarkId::new("base91_dnsl48", "1mib"),
        &encoded,
        |b, encoded| {
            b.iter(|| base91_other::slice_decode(encoded));
        },
    );

    #[cfg(feature = "c-compat-tests")]
    g.bench_with_input(
        BenchmarkId::new("c_reference", "1mib"),
        &encoded,
        |b, encoded| {
            b.iter(|| unsafe { c_ref::decode(encoded, dec_buf.as_mut_ptr()) });
        },
    );

    g.finish();
}

fn bench_encode_simd(c: &mut Criterion) {
    use base91::simd::SimdLevel;
    let input = random_bytes(SIZE);

    let mut g = c.benchmark_group("encode_simd");
    g.throughput(Throughput::Bytes(input.len() as u64));

    g.bench_with_input(BenchmarkId::new("scalar", "1mib"), &input, |b, input| {
        b.iter(|| base91::simd::encode(input, SimdLevel::Scalar, 0));
    });
    g.bench_with_input(BenchmarkId::new("simd128", "1mib"), &input, |b, input| {
        b.iter(|| base91::simd::encode(input, SimdLevel::Simd128, 0));
    });
    g.bench_with_input(BenchmarkId::new("simd256", "1mib"), &input, |b, input| {
        b.iter(|| base91::simd::encode(input, SimdLevel::Simd256, 0));
    });

    g.finish();
}

fn bench_decode_simd(c: &mut Criterion) {
    use base91::simd::SimdLevel;
    let input = random_bytes(SIZE);
    let encoded = base91::simd::encode(&input, SimdLevel::default(), 0);

    let mut g = c.benchmark_group("decode_simd");
    g.throughput(Throughput::Bytes(encoded.len() as u64));

    g.bench_with_input(
        BenchmarkId::new("scalar", "1mib"),
        &encoded,
        |b, encoded| {
            b.iter(|| base91::simd::decode(encoded, SimdLevel::Scalar));
        },
    );
    g.bench_with_input(
        BenchmarkId::new("simd128", "1mib"),
        &encoded,
        |b, encoded| {
            b.iter(|| base91::simd::decode(encoded, SimdLevel::Simd128));
        },
    );
    g.bench_with_input(
        BenchmarkId::new("simd256", "1mib"),
        &encoded,
        |b, encoded| {
            b.iter(|| base91::simd::decode(encoded, SimdLevel::Simd256));
        },
    );

    g.finish();
}

criterion_group!(
    benches,
    bench_encode,
    bench_decode,
    bench_encode_simd,
    bench_decode_simd
);
criterion_main!(benches);
