// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Tests of the C library's two APIs (spec 0014 §3, Tests).

use super::*;
use base91_testkit::rng::Rng;
use core::mem::{offset_of, size_of};

const SIZES: [usize; 12] = [0, 1, 2, 3, 12, 13, 14, 26, 100, 1000, 4097, 20_000];

fn new_state() -> BasE91 {
    let mut b = MaybeUninit::<BasE91>::uninit();
    // SAFETY: `b` is writable; `basE91_init` initializes it.
    unsafe {
        basE91_init(b.as_mut_ptr());
        b.assume_init()
    }
}

/// Henke's API: encode `data` split into `lengths`, one output per call.
fn c_encode_calls(data: &[u8], lengths: &[usize]) -> Vec<Vec<u8>> {
    let mut b = new_state();
    let mut calls = Vec::new();
    let mut pos = 0;
    for &n in lengths {
        let mut buf = vec![0u8; 2 * n + 2];
        // SAFETY: the input is readable for `n` bytes; `buf` holds the
        // most a call may write (2 characters per 13 bits, plus 2).
        let w = unsafe {
            basE91_encode(
                &mut b,
                data[pos..].as_ptr().cast(),
                n,
                buf.as_mut_ptr().cast(),
            )
        };
        calls.push(buf[..w].to_vec());
        pos += n;
    }
    let mut end = [0u8; 2];
    // SAFETY: `end` holds 2 bytes.
    let w = unsafe { basE91_encode_end(&mut b, end.as_mut_ptr().cast()) };
    calls.push(end[..w].to_vec());
    calls
}

/// Henke's API: decode `encoded` split into `lengths`, one output per call.
fn c_decode_calls(encoded: &[u8], lengths: &[usize]) -> Vec<Vec<u8>> {
    let mut b = new_state();
    let mut calls = Vec::new();
    let mut pos = 0;
    for &n in lengths {
        let mut buf = vec![0u8; n + 1];
        // SAFETY: as in `c_encode_calls`; a call writes at most 14/16 of
        // its input, plus 1.
        let w = unsafe {
            basE91_decode(
                &mut b,
                encoded[pos..].as_ptr().cast(),
                n,
                buf.as_mut_ptr().cast(),
            )
        };
        calls.push(buf[..w].to_vec());
        pos += n;
    }
    let mut end = [0u8; 1];
    // SAFETY: `end` holds 1 byte.
    let w = unsafe { basE91_decode_end(&mut b, end.as_mut_ptr().cast()) };
    calls.push(end[..w].to_vec());
    calls
}

/// `encoded` with junk bytes (outside Henke's alphabet) inserted.
fn with_junk(rng: &mut Rng, encoded: &[u8]) -> Vec<u8> {
    let junk = [b'\n', b' ', b'\r', b'\t', 0x00, 0x7f, 0xff, b'\''];
    let mut out = Vec::with_capacity(encoded.len() * 2);
    for &byte in encoded {
        while rng.below(8) == 0 {
            out.push(junk[rng.below(junk.len())]);
        }
        out.push(byte);
    }
    out
}

#[test]
fn struct_layout_is_henkes() {
    // Henke's struct on LP64 (Linux, macOS): unsigned long, unsigned int,
    // int, in that order, no padding but at the end.
    assert_eq!(offset_of!(BasE91, queue), 0);
    assert_eq!(offset_of!(BasE91, nbits), size_of::<c_ulong>());
    assert_eq!(
        offset_of!(BasE91, val),
        size_of::<c_ulong>() + size_of::<c_uint>()
    );
    assert_eq!(
        size_of::<BasE91>(),
        (size_of::<c_ulong>() + 2 * size_of::<c_int>())
            .next_multiple_of(core::mem::align_of::<c_ulong>())
    );
}

#[test]
fn init_is_henkes() {
    let b = new_state();
    assert_eq!((b.queue, b.nbits, b.val), (0, 0, -1));
}

#[test]
fn henke_encode_matches_rust_codec_on_any_split() {
    let mut rng = Rng::for_test("capi-henke-encode");
    for n in SIZES {
        let data = rng.bytes(n);
        let expected = base91::encode(&data);
        for _ in 0..8 {
            let lengths = rng.split_points(n);
            assert_eq!(
                c_encode_calls(&data, &lengths).concat(),
                expected,
                "n = {n}"
            );
        }
        assert_eq!(
            c_encode_calls(&data, &[n]).concat(),
            expected,
            "n = {n}, one call"
        );
    }
}

#[test]
fn henke_decode_matches_rust_codec_on_any_split() {
    let mut rng = Rng::for_test("capi-henke-decode");
    for n in SIZES {
        let data = rng.bytes(n);
        let encoded = with_junk(&mut rng, &base91::encode(&data));
        for _ in 0..8 {
            let lengths = rng.split_points(encoded.len());
            assert_eq!(c_decode_calls(&encoded, &lengths).concat(), data, "n = {n}");
        }
    }
}

#[test]
fn end_resets_the_state() {
    let mut b = new_state();
    let mut buf = [0u8; 16];
    // SAFETY: 5 readable bytes; `buf` holds the output.
    unsafe {
        basE91_encode(&mut b, b"hello".as_ptr().cast(), 5, buf.as_mut_ptr().cast());
        basE91_encode_end(&mut b, buf.as_mut_ptr().cast());
    }
    assert_eq!((b.queue, b.nbits, b.val), (0, 0, -1));
    // SAFETY: as above.
    unsafe {
        basE91_decode(&mut b, b"A".as_ptr().cast(), 1, buf.as_mut_ptr().cast());
        assert_eq!(b.val, 0, "'A' pending");
        basE91_decode_end(&mut b, buf.as_mut_ptr().cast());
    }
    assert_eq!((b.queue, b.nbits, b.val), (0, 0, -1));
}

/// Henke's command-line tool: one buffer of `buf_size` bytes, the input in
/// its last `ibuf_size` bytes, the output written from its start.
fn cli_layout(data: &[u8], buf_size: usize, decode: bool) -> Vec<u8> {
    let ibuf_size = if decode {
        (buf_size - 1) * 8 / 15
    } else {
        (buf_size - 2) * 16 / 29
    };
    let mut buf = vec![0u8; buf_size];
    let obuf = buf.as_mut_ptr();
    let mut b = new_state();
    let mut out = Vec::new();
    for chunk in data.chunks(ibuf_size) {
        // SAFETY: the chunk fits in the last `ibuf_size` bytes of `buf`;
        // the output stays behind the unread input, as in Henke's tool.
        let w = unsafe {
            let ibuf = obuf.add(buf_size - ibuf_size);
            core::ptr::copy_nonoverlapping(chunk.as_ptr(), ibuf, chunk.len());
            if decode {
                basE91_decode(&mut b, ibuf.cast(), chunk.len(), obuf.cast())
            } else {
                basE91_encode(&mut b, ibuf.cast(), chunk.len(), obuf.cast())
            }
        };
        out.extend_from_slice(&buf[..w]);
    }
    let mut end = [0u8; 2];
    // SAFETY: `end` holds 2 bytes.
    let w = unsafe {
        if decode {
            basE91_decode_end(&mut b, end.as_mut_ptr().cast())
        } else {
            basE91_encode_end(&mut b, end.as_mut_ptr().cast())
        }
    };
    out.extend_from_slice(&end[..w]);
    out
}

#[test]
fn overlapping_buffers_as_in_henkes_tool() {
    let mut rng = Rng::for_test("capi-overlap");
    let data = rng.bytes(50_000);
    let encoded = base91::encode(&data);
    for buf_size in [64, 1000, 65536] {
        assert_eq!(
            cli_layout(&data, buf_size, false),
            encoded,
            "encode, {buf_size}"
        );
        assert_eq!(
            cli_layout(&encoded, buf_size, true),
            data,
            "decode, {buf_size}"
        );
    }
}

#[cfg(feature = "reference-tests")]
#[test]
fn henke_api_matches_upstream_call_by_call() {
    use base91_testkit::reference;
    let mut rng = Rng::for_test("capi-reference");
    for n in SIZES {
        let data = rng.bytes(n);
        let lengths = rng.split_points(n);
        let mut upstream = reference::Encoder::new();
        let mut expected = Vec::new();
        let mut pos = 0;
        for &len in &lengths {
            let mut out = Vec::new();
            upstream.encode(&data[pos..pos + len], &mut out);
            expected.push(out);
            pos += len;
        }
        let mut out = Vec::new();
        upstream.finish(&mut out);
        expected.push(out);
        assert_eq!(c_encode_calls(&data, &lengths), expected, "encode, n = {n}");

        let encoded = with_junk(&mut rng, &reference::encode(&data));
        let lengths = rng.split_points(encoded.len());
        let mut upstream = reference::Decoder::new();
        let mut expected = Vec::new();
        let mut pos = 0;
        for &len in &lengths {
            let mut out = Vec::new();
            upstream.decode(&encoded[pos..pos + len], &mut out);
            expected.push(out);
            pos += len;
        }
        let mut out = Vec::new();
        upstream.finish(&mut out);
        expected.push(out);
        assert_eq!(
            c_decode_calls(&encoded, &lengths),
            expected,
            "decode, n = {n}"
        );
    }
}

// ---------------------------------------------------------------------------
// The base91_* API
// ---------------------------------------------------------------------------

fn encode(format: c_int, data: &[u8], wrap: usize) -> Result<Vec<u8>, c_int> {
    let cap = base91_encoded_len_max(format, data.len(), wrap);
    let mut out = vec![0u8; cap];
    let mut written = 0;
    // SAFETY: valid buffers of the sizes passed.
    let code = unsafe {
        base91_encode(
            format,
            data.as_ptr().cast(),
            data.len(),
            wrap,
            out.as_mut_ptr().cast(),
            cap,
            &mut written,
        )
    };
    if code != BASE91_OK {
        return Err(code);
    }
    out.truncate(written);
    Ok(out)
}

fn decode(encoded: &[u8]) -> Result<Vec<u8>, (c_int, usize)> {
    let cap = base91_decoded_len_max(encoded.len());
    let mut out = vec![0u8; cap];
    let (mut written, mut offset) = (0, usize::MAX);
    // SAFETY: valid buffers of the sizes passed.
    let code = unsafe {
        base91_decode(
            encoded.as_ptr().cast(),
            encoded.len(),
            out.as_mut_ptr().cast(),
            cap,
            &mut written,
            &mut offset,
        )
    };
    if code != BASE91_OK {
        return Err((code, offset));
    }
    out.truncate(written);
    Ok(out)
}

#[test]
fn new_api_matches_base91_rs() {
    let mut rng = Rng::for_test("capi-new-api");
    for n in SIZES {
        let data = rng.bytes(n);
        let henke = encode(BASE91_HENKE, &data, 0).unwrap();
        assert_eq!(henke, base91::encode(&data));
        assert_eq!(decode(&henke).unwrap(), data);
        for wrap in [0, 32, 64] {
            let simd = encode(BASE91_SIMD, &data, wrap).unwrap();
            assert_eq!(
                simd,
                base91::simd::encode(&data, base91::simd::SimdLevel::default(), wrap).unwrap()
            );
            assert_eq!(
                simd.len(),
                base91_encoded_len_max(BASE91_SIMD, n, wrap),
                "exact"
            );
            assert_eq!(decode(&simd).unwrap(), data, "n = {n}, wrap = {wrap}");
        }
    }
}

#[test]
fn new_api_errors() {
    assert_eq!(
        encode(BASE91_SIMD, b"data", 5),
        Err(BASE91_ERR_INVALID_WIDTH)
    );
    assert_eq!(
        encode(BASE91_HENKE, b"data", 32),
        Err(BASE91_ERR_INVALID_WIDTH)
    );
    assert_eq!(encode(7, b"data", 0), Err(BASE91_ERR_INVALID_FORMAT));
    assert_eq!(base91_encoded_len_max(BASE91_SIMD, 10, 5), 0);
    assert_eq!(base91_encoded_len_max(7, 10, 0), 0);

    // A buffer too small: nothing written but the size needed.
    let mut out = [0x55u8; 4];
    let mut written = 0;
    // SAFETY: valid buffers of the sizes passed.
    let code = unsafe {
        base91_encode(
            BASE91_SIMD,
            b"data".as_ptr().cast(),
            4,
            0,
            out.as_mut_ptr().cast(),
            4,
            &mut written,
        )
    };
    assert_eq!(code, BASE91_ERR_BUFFER_TOO_SMALL);
    assert_eq!(written, base91_encoded_len_max(BASE91_SIMD, 4, 0));
    assert_eq!(out, [0x55; 4]);

    // Null pointers with a non-zero length.
    // SAFETY: the null pointers are rejected before use.
    unsafe {
        assert_eq!(
            base91_encode(
                BASE91_HENKE,
                core::ptr::null(),
                4,
                0,
                out.as_mut_ptr().cast(),
                4,
                &mut written
            ),
            BASE91_ERR_NULL_POINTER
        );
        assert_eq!(
            base91_decode(
                b"ab".as_ptr().cast(),
                2,
                out.as_mut_ptr().cast(),
                4,
                core::ptr::null_mut(),
                core::ptr::null_mut()
            ),
            BASE91_ERR_NULL_POINTER
        );
    }

    // An empty input needs no buffer.
    assert_eq!(encode(BASE91_SIMD, b"", 0), Ok(Vec::new()));

    // A corrupted SIMD stream: the offset of the offending byte.
    let mut rng = Rng::for_test("capi-errors");
    let mut simd = encode(BASE91_SIMD, &rng.bytes(1000), 0).unwrap();
    simd[10] = b'\'';
    assert_eq!(decode(&simd), Err((BASE91_ERR_INVALID_CHARACTER, 10)));
}

#[test]
fn strings() {
    // SAFETY: the functions return static NUL-terminated strings.
    unsafe {
        assert_eq!(
            CStr::from_ptr(base91_version()).to_str(),
            Ok(env!("CARGO_PKG_VERSION"))
        );
        let level = CStr::from_ptr(base91_simd_level()).to_str().unwrap();
        assert!(["avx2", "sse4.1", "neon", "scalar"].contains(&level));
        for code in 0..=BASE91_ERR_INVALID_FORMAT {
            assert_ne!(
                CStr::from_ptr(base91_strerror(code)).to_bytes(),
                b"unknown error",
                "{code}"
            );
        }
        assert_eq!(
            CStr::from_ptr(base91_strerror(99)).to_bytes(),
            b"unknown error"
        );
    }
}
