// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! basE91 binary-to-text encoding.
//!
//! A pure-Rust, `no_std`-compatible implementation of the basE91 algorithm
//! invented by Joachim Henke.  Wire-format compatible with the C reference
//! implementation at <http://base91.sourceforge.net/>.
//!
//! # Quick start
//!
//! ```rust
//! let encoded = base91::encode(b"Hello, world!");
//! let decoded = base91::decode(&encoded);
//! assert_eq!(decoded, b"Hello, world!");
//! ```
//!
//! # Streaming (in-memory)
//!
//! For large or chunked inputs, pre-reserve with the size hints to avoid
//! reallocation:
//!
//! ```rust
//! use base91::{Encoder, encode_size_hint};
//!
//! let input = b"some large input";
//! let mut out = Vec::with_capacity(encode_size_hint(input.len()));
//! let mut enc = Encoder::new();
//! enc.encode(input, &mut out);
//! enc.finish(&mut out);
//! ```
//!
//! # The SIMD variant
//!
//! [`simd`] implements a fixed-width variant of basE91 (not compatible with
//! the C reference), designed for SIMD kernels. Its streams start with `-`;
//! Its decoders ([`simd::decode_slice`], `simd::decode`, …) recognize both formats. SIMD streams are decoded
//! strictly: damage is reported as an [`Error`], with its position.
//!
//! ```rust
//! use base91::simd::{self, SimdLevel};
//!
//! let encoded = simd::encode(b"Hello, world!", SimdLevel::default(), 64)?;
//! assert_eq!(simd::decode(&encoded, SimdLevel::default())?, b"Hello, world!");
//!
//! let damaged = [&encoded[..10], b" ", &encoded[10..]].concat();
//! assert_eq!(
//!     simd::decode(&damaged, SimdLevel::default()),
//!     Err(base91::Error::InvalidCharacter { offset: 10 })
//! );
//! # Ok::<(), base91::Error>(())
//! ```
//!
//! # Without an allocator
//!
//! The slice API writes into caller-provided buffers and works without the
//! `alloc` feature. Each call checks the buffer size before writing:
//!
//! ```rust
//! use base91::simd::{self, SimdLevel};
//!
//! let input = b"Hello, world!";
//! let mut encoded = [0u8; 32];
//! let n = simd::encode_slice(input, SimdLevel::default(), 0, &mut encoded)?;
//! let mut decoded = [0u8; 32];
//! let m = simd::decode_slice(&encoded[..n], SimdLevel::default(), &mut decoded)?;
//! assert_eq!(&decoded[..m], input);
//! # Ok::<(), base91::Error>(())
//! ```
//!
//! # `std::io` adapters
//!
//! With the `io` feature (default), `io::EncoderWriter` and
//! `io::DecoderReader` wrap any `std::io::Write` / `std::io::Read` and
//! handle buffering automatically:
//!
//! ```rust
//! use base91::io::{EncoderWriter, DecoderReader};
//! use std::io::{Write, Read};
//!
//! let mut enc = EncoderWriter::new(Vec::new());
//! enc.write_all(b"Hello, world!").unwrap();
//! let encoded = enc.finish().unwrap();
//!
//! let mut dec = DecoderReader::new(encoded.as_slice());
//! let mut decoded = Vec::new();
//! dec.read_to_end(&mut decoded).unwrap();
//! assert_eq!(decoded, b"Hello, world!");
//! ```

// no_std unless the `std` feature is on (spec 0004 §1). Test builds keep std
// for the test harness; the `std` *feature* stays off there, so they exercise
// the no_std code paths.
#![cfg_attr(not(any(feature = "std", test)), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::mem::MaybeUninit;

mod codec;
mod error;
#[cfg(feature = "io")]
pub mod io;
#[cfg(feature = "python")]
pub mod python;
pub mod simd;

pub use codec::{Decoder, Encoder};
pub use error::Error;

/// Return stub generation metadata for the `pybase91` Python extension.
///
/// Called by `src/bin/post_build.rs`; only available with `--features python`.
#[cfg(feature = "python")]
pub use python::stub_info;

/// Conservative upper bound on the encoded length for `input_len` input bytes.
///
/// Encoding ratio is at most 16/13 bits, plus 2 bytes for `finish()`.
/// Pre-reserving this capacity before feeding chunks to [`Encoder::encode_slice`] or `Encoder::encode`
/// ensures the `Vec` never reallocates during encoding.
///
/// ```rust
/// assert!(base91::encode_size_hint(1024) >= base91::encode(b"a".repeat(1024).as_slice()).len());
/// ```
#[inline]
pub fn encode_size_hint(input_len: usize) -> usize {
    // ceil(input_len * 16 / 13) + 2
    (input_len * 16).div_ceil(13) + 2
}

/// Conservative upper bound on the decoded length for `input_len` encoded bytes.
///
/// Each pair of base91 characters carries at most 14 bits = 1 full output byte
/// plus residual bits, plus 1 byte for `finish()`.
/// Pre-reserving this capacity before feeding chunks to [`Decoder::decode_slice`] or `Decoder::decode`
/// ensures the `Vec` never reallocates during decoding.
///
/// ```rust
/// let input = b"Hello, world!";
/// let encoded = base91::encode(input);
/// assert!(base91::decode_size_hint(encoded.len()) >= input.len());
/// ```
#[inline]
pub fn decode_size_hint(input_len: usize) -> usize {
    // floor(input_len * 14 / 16) + 1  =  input_len * 7 / 8 + 1
    input_len * 7 / 8 + 1
}

/// Encode `input` to a new `Vec<u8>` of base91 characters.
#[cfg(feature = "alloc")]
pub fn encode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(encode_size_hint(input.len()));
    let mut enc = Encoder::new();
    enc.encode(input, &mut out);
    enc.finish(&mut out);
    out
}

/// Decode `input` (base91 characters) to a new `Vec<u8>`.
///
/// Non-alphabet bytes are silently ignored.
#[cfg(feature = "alloc")]
pub fn decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(decode_size_hint(input.len()));
    let mut dec = Decoder::new();
    dec.decode(input, &mut out);
    dec.finish(&mut out);
    out
}

/// Encode `input` into `output`, and return the number of bytes written.
///
/// `output` must hold at least [`encode_size_hint`]`(input.len())` bytes;
/// otherwise nothing is written and the error gives that size. Needs no
/// allocator.
///
/// ```rust
/// let mut buf = [0u8; 32];
/// let n = base91::encode_slice(b"Hello, world!", &mut buf).unwrap();
/// assert_eq!(base91::decode(&buf[..n]), b"Hello, world!");
/// ```
pub fn encode_slice(input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
    let needed = encode_size_hint(input.len());
    if output.len() < needed {
        return Err(Error::BufferTooSmall { needed });
    }
    Ok(encode_uninit(input, as_uninit(output)))
}

/// Decode `input` (base91 characters) into `output`, and return the number
/// of bytes written. Non-alphabet bytes are silently ignored.
///
/// `output` must hold at least [`decode_size_hint`]`(input.len())` bytes;
/// otherwise nothing is written and the error gives that size. Needs no
/// allocator.
pub fn decode_slice(input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
    let needed = decode_size_hint(input.len());
    if output.len() < needed {
        return Err(Error::BufferTooSmall { needed });
    }
    Ok(decode_uninit(input, as_uninit(output)))
}

/// Encode `input` into a caller-provided buffer, without bounds checking.
///
/// Writes encoded bytes to `output` and returns the number of bytes written.
///
/// # Safety
///
/// `output` must point to at least `encode_size_hint(input.len())` writable
/// bytes.  Writing beyond that bound is undefined behaviour.
#[deprecated(since = "0.3.0", note = "use `encode_slice`")]
pub unsafe fn encode_unchecked(input: &[u8], output: *mut u8) -> usize {
    // SAFETY: the caller guarantees `encode_size_hint(input.len())` writable
    // bytes at `output`; they are only written.
    let out = unsafe {
        core::slice::from_raw_parts_mut(
            output.cast::<MaybeUninit<u8>>(),
            encode_size_hint(input.len()),
        )
    };
    encode_uninit(input, out)
}

/// Decode `input` into a caller-provided buffer, without bounds checking.
///
/// Non-alphabet bytes are silently ignored.
/// Writes decoded bytes to `output` and returns the number of bytes written.
///
/// # Safety
///
/// `output` must point to at least `decode_size_hint(input.len())` writable
/// bytes.  Writing beyond that bound is undefined behaviour.
#[deprecated(since = "0.3.0", note = "use `decode_slice`")]
pub unsafe fn decode_unchecked(input: &[u8], output: *mut u8) -> usize {
    // SAFETY: as in `encode_unchecked`, with `decode_size_hint`.
    let out = unsafe {
        core::slice::from_raw_parts_mut(
            output.cast::<MaybeUninit<u8>>(),
            decode_size_hint(input.len()),
        )
    };
    decode_uninit(input, out)
}

/// One-shot legacy encoding into `out`, which holds at least
/// `encode_size_hint(input.len())` bytes.
fn encode_uninit(input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
    let mut enc = Encoder::new();
    let n = enc.encode_uninit(input, out);
    n + enc.finish_uninit(&mut out[n..])
}

/// One-shot legacy decoding into `out`, which holds at least
/// `decode_size_hint(input.len())` bytes.
fn decode_uninit(input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
    let mut dec = Decoder::new();
    let n = dec.decode_uninit(input, out);
    n + dec.finish_uninit(&mut out[n..])
}

/// Views an initialized buffer as the cores' output type.
pub(crate) fn as_uninit(out: &mut [u8]) -> &mut [MaybeUninit<u8>] {
    // SAFETY: `MaybeUninit<u8>` has the layout of `u8`, and the cores only
    // ever write initialized bytes through the view, so `out` stays
    // initialized.
    unsafe { core::slice::from_raw_parts_mut(out.as_mut_ptr().cast(), out.len()) }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    #[test]
    fn size_hints_are_valid_upper_bounds() {
        // Verify encode_size_hint and decode_size_hint for all lengths 0..=1024.
        for len in 0..=1024usize {
            let input: Vec<u8> = (0u8..=255).cycle().take(len).collect();
            let encoded = encode(&input);
            assert!(
                encoded.len() <= encode_size_hint(len),
                "encode_size_hint({len}) = {} < actual {}",
                encode_size_hint(len),
                encoded.len()
            );
            let decoded = decode(&encoded);
            assert!(
                decoded.len() <= decode_size_hint(encoded.len()),
                "decode_size_hint({}) = {} < actual {}",
                encoded.len(),
                decode_size_hint(encoded.len()),
                decoded.len()
            );
        }
    }

    // Tests of the deprecated functions: they stay public until 1.0, so they
    // stay tested; calling them is the point (spec 0010 §7).
    #[test]
    #[allow(deprecated)]
    fn unchecked_encode_matches_safe() {
        let input: Vec<u8> = (0u8..=255).collect();
        let safe = encode(&input);
        let mut buf = vec![0u8; encode_size_hint(input.len())];
        let n = unsafe { encode_unchecked(&input, buf.as_mut_ptr()) };
        assert_eq!(&buf[..n], safe.as_slice());
    }

    #[test]
    #[allow(deprecated)]
    fn unchecked_decode_matches_safe() {
        let input: Vec<u8> = (0u8..=255).collect();
        let encoded = encode(&input);
        let safe = decode(&encoded);
        let mut buf = vec![0u8; decode_size_hint(encoded.len())];
        let n = unsafe { decode_unchecked(&encoded, buf.as_mut_ptr()) };
        assert_eq!(&buf[..n], safe.as_slice());
    }

    #[test]
    fn slice_api_matches_vec_api() {
        for len in 0..=300usize {
            let input: Vec<u8> = (0u8..=255).cycle().take(len).collect();
            let encoded = encode(&input);
            let mut buf = vec![0u8; encode_size_hint(len)];
            let n = encode_slice(&input, &mut buf).unwrap();
            assert_eq!(&buf[..n], &encoded[..]);

            let mut back = vec![0u8; decode_size_hint(encoded.len())];
            let n = decode_slice(&encoded, &mut back).unwrap();
            assert_eq!(&back[..n], &input[..]);
        }
    }

    #[test]
    fn slice_api_buffer_too_small_writes_nothing() {
        let input = b"Hello, world!";
        let needed = encode_size_hint(input.len());
        let mut buf = vec![0xA5u8; needed - 1];
        assert_eq!(
            encode_slice(input, &mut buf),
            Err(Error::BufferTooSmall { needed })
        );
        assert!(buf.iter().all(|&b| b == 0xA5));

        let encoded = encode(input);
        let needed = decode_size_hint(encoded.len());
        let mut buf = vec![0xA5u8; needed - 1];
        assert_eq!(
            decode_slice(&encoded, &mut buf),
            Err(Error::BufferTooSmall { needed })
        );
        assert!(buf.iter().all(|&b| b == 0xA5));
    }

    #[test]
    fn streaming_slice_api_matches_vec_api() {
        let input: Vec<u8> = (0u8..=255).cycle().take(2000).collect();
        let encoded = encode(&input);
        for chunk in [1, 2, 3, 7, 64] {
            let mut enc = Encoder::new();
            let mut out = Vec::new();
            for piece in input.chunks(chunk) {
                let mut buf = vec![0u8; encode_size_hint(piece.len())];
                let n = enc.encode_slice(piece, &mut buf).unwrap();
                out.extend_from_slice(&buf[..n]);
            }
            let mut buf = [0u8; 2];
            let n = enc.finish_slice(&mut buf).unwrap();
            out.extend_from_slice(&buf[..n]);
            assert_eq!(out, encoded, "chunk={chunk}");

            let mut dec = Decoder::new();
            let mut back = Vec::new();
            for piece in encoded.chunks(chunk) {
                let mut buf = vec![0u8; decode_size_hint(piece.len() + 1)];
                let n = dec.decode_slice(piece, &mut buf).unwrap();
                back.extend_from_slice(&buf[..n]);
            }
            let mut buf = [0u8; 1];
            let n = dec.finish_slice(&mut buf).unwrap();
            back.extend_from_slice(&buf[..n]);
            assert_eq!(back, input, "chunk={chunk}");
        }
    }
}

/// The slice API without an allocator (spec 0010 §2): these tests use stack
/// buffers only, and also run in the `no_std` builds without `alloc`.
#[cfg(test)]
mod slice_tests {
    use crate::simd::{self, SimdLevel};

    const INPUT: &[u8] = b"The quick brown fox jumps over the lazy dog, twice: \
                           the quick brown fox jumps over the lazy dog.";

    #[test]
    fn legacy_round_trip() {
        let mut enc = [0u8; 256];
        let n = crate::encode_slice(INPUT, &mut enc).unwrap();
        let mut dec = [0u8; 256];
        let m = crate::decode_slice(&enc[..n], &mut dec).unwrap();
        assert_eq!(&dec[..m], INPUT);
    }

    #[test]
    fn simd_round_trip() {
        for level in [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256] {
            for wrap in [0, 32] {
                let mut enc = [0u8; 256];
                let n = simd::encode_slice(INPUT, level, wrap, &mut enc).unwrap();
                assert_eq!(Ok(n), simd::encoded_len(INPUT.len(), wrap));
                let mut dec = [0u8; 256];
                let m = simd::decode_slice(&enc[..n], level, &mut dec).unwrap();
                assert_eq!(&dec[..m], INPUT);
            }
        }
    }

    #[test]
    fn simd_streaming_round_trip() {
        let mut enc = [0u8; 256];
        let mut n = 0;
        let mut encoder = simd::Encoder::new(SimdLevel::default(), 32).unwrap();
        for piece in INPUT.chunks(7) {
            n += encoder.encode_slice(piece, &mut enc[n..]).unwrap();
        }
        n += encoder.finish_slice(&mut enc[n..]).unwrap();
        assert_eq!(Ok(n), simd::encoded_len(INPUT.len(), 32));

        let mut dec = [0u8; 256];
        let mut m = 0;
        let mut decoder = simd::Decoder::new(SimdLevel::default());
        for piece in enc[..n].chunks(5) {
            m += decoder.decode_slice(piece, &mut dec[m..]).unwrap();
        }
        m += decoder.finish_slice(&mut dec[m..]).unwrap();
        assert_eq!(&dec[..m], INPUT);
    }

    #[test]
    fn simd_errors_without_alloc() {
        let mut dec = [0u8; 16];
        assert_eq!(
            simd::decode_slice(b"-##\"##", SimdLevel::default(), &mut dec),
            Err(crate::Error::InvalidCharacter { offset: 3 })
        );
        assert_eq!(
            simd::decode_slice(b"-#", SimdLevel::default(), &mut dec),
            Err(crate::Error::InvalidLength)
        );
    }
}
