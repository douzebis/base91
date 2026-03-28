// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::RngCore;

// ---------------------------------------------------------------------------
// C reference FFI — only available with --features c-compat-tests
// (build.rs compiles src/base91.c and renames symbols to c_basE91_*)
// ---------------------------------------------------------------------------

#[cfg(feature = "c-compat-tests")]
mod c_ref {
    use base91::c_api::basE91 as State;

    extern "C" {
        pub fn c_basE91_init(b: *mut State);
        pub fn c_basE91_encode(b: *mut State, i: *const u8, len: usize, o: *mut u8) -> usize;
        pub fn c_basE91_encode_end(b: *mut State, o: *mut u8) -> usize;
        pub fn c_basE91_decode(b: *mut State, i: *const u8, len: usize, o: *mut u8) -> usize;
        pub fn c_basE91_decode_end(b: *mut State, o: *mut u8) -> usize;
    }

    pub unsafe fn encode(input: &[u8], output: *mut u8) -> usize {
        let mut state = std::mem::MaybeUninit::<State>::uninit();
        c_basE91_init(state.as_mut_ptr());
        let mut state = state.assume_init();
        let n = c_basE91_encode(&mut state, input.as_ptr(), input.len(), output);
        let m = c_basE91_encode_end(&mut state, output.add(n));
        n + m
    }

    pub unsafe fn decode(input: &[u8], output: *mut u8) -> usize {
        let mut state = std::mem::MaybeUninit::<State>::uninit();
        c_basE91_init(state.as_mut_ptr());
        let mut state = state.assume_init();
        let n = c_basE91_decode(&mut state, input.as_ptr(), input.len(), output);
        let m = c_basE91_decode_end(&mut state, output.add(n));
        n + m
    }
}

// ---------------------------------------------------------------------------
// Rust C API
// ---------------------------------------------------------------------------

unsafe fn rust_c_api_encode(input: &[u8], output: *mut u8) -> usize {
    let mut state = std::mem::MaybeUninit::<base91::c_api::basE91>::uninit();
    base91::c_api::basE91_init(state.as_mut_ptr());
    let mut state = state.assume_init();
    let n = base91::c_api::basE91_encode(
        &mut state,
        input.as_ptr() as *const _,
        input.len(),
        output as *mut _,
    );
    let m = base91::c_api::basE91_encode_end(&mut state, output.add(n) as *mut _);
    n + m
}

unsafe fn rust_c_api_decode(input: &[u8], output: *mut u8) -> usize {
    let mut state = std::mem::MaybeUninit::<base91::c_api::basE91>::uninit();
    base91::c_api::basE91_init(state.as_mut_ptr());
    let mut state = state.assume_init();
    let n = base91::c_api::basE91_decode(
        &mut state,
        input.as_ptr() as *const _,
        input.len(),
        output as *mut _,
    );
    let m = base91::c_api::basE91_decode_end(&mut state, output.add(n) as *mut _);
    n + m
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

    g.bench_with_input(
        BenchmarkId::new("rust_c_api", "1mib"),
        &input,
        |b, input| {
            b.iter(|| unsafe { rust_c_api_encode(input, enc_buf.as_mut_ptr()) });
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
        BenchmarkId::new("rust_c_api", "1mib"),
        &encoded,
        |b, encoded| {
            b.iter(|| unsafe { rust_c_api_decode(encoded, dec_buf.as_mut_ptr()) });
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

criterion_group!(benches, bench_encode, bench_decode);
criterion_main!(benches);
