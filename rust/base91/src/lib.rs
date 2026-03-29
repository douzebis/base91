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
//! # `std::io` adapters
//!
//! With the `io` feature (default), [`io::EncoderWriter`] and
//! [`io::DecoderReader`] wrap any [`Write`][std::io::Write] /
//! [`Read`][std::io::Read] and handle buffering automatically:
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

#![cfg_attr(not(feature = "std"), no_std)]

mod codec;
#[cfg(feature = "io")]
pub mod io;
#[cfg(feature = "python")]
pub mod python;
pub mod simd;

pub use codec::{Decoder, Encoder};

/// Return stub generation metadata for the `pybase91` Python extension.
///
/// Called by `src/bin/post_build.rs`; only available with `--features python`.
#[cfg(feature = "python")]
pub use python::stub_info;

/// Conservative upper bound on the encoded length for `input_len` input bytes.
///
/// Encoding ratio is at most 16/13 bits, plus 2 bytes for `finish()`.
/// Pre-reserving this capacity before feeding chunks to [`Encoder::encode`]
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
/// Pre-reserving this capacity before feeding chunks to [`Decoder::decode`]
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
///
/// The output buffer is pre-reserved to the exact upper bound, so no
/// reallocation occurs and the Vec capacity check is eliminated from the
/// hot loop by the branch predictor.
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
/// The output buffer is pre-reserved to the exact upper bound, so no
/// reallocation occurs and the Vec capacity check is eliminated from the
/// hot loop by the branch predictor.
pub fn decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(decode_size_hint(input.len()));
    let mut dec = Decoder::new();
    dec.decode(input, &mut out);
    dec.finish(&mut out);
    out
}

/// Encode `input` into a caller-provided buffer, without bounds checking.
///
/// Writes encoded bytes to `output` and returns the number of bytes written.
///
/// # Safety
///
/// `output` must point to at least `encode_size_hint(input.len())` writable
/// bytes.  Writing beyond that bound is undefined behaviour.
///
/// # Example
///
/// ```rust
/// let input = b"Hello, world!";
/// let mut buf = vec![0u8; base91::encode_size_hint(input.len())];
/// let n = unsafe { base91::encode_unchecked(input, buf.as_mut_ptr()) };
/// let encoded = &buf[..n];
/// assert_eq!(base91::decode(encoded), input);
/// ```
pub unsafe fn encode_unchecked(input: &[u8], output: *mut u8) -> usize {
    codec::encode_unchecked(input, output)
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
///
/// # Example
///
/// ```rust
/// let input = b"Hello, world!";
/// let encoded = base91::encode(input);
/// let mut buf = vec![0u8; base91::decode_size_hint(encoded.len())];
/// let n = unsafe { base91::decode_unchecked(&encoded, buf.as_mut_ptr()) };
/// assert_eq!(&buf[..n], input);
/// ```
pub unsafe fn decode_unchecked(input: &[u8], output: *mut u8) -> usize {
    codec::decode_unchecked(input, output)
}

#[cfg(test)]
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

    #[test]
    fn unchecked_encode_matches_safe() {
        let input: Vec<u8> = (0u8..=255).collect();
        let safe = encode(&input);
        let mut buf = vec![0u8; encode_size_hint(input.len())];
        let n = unsafe { encode_unchecked(&input, buf.as_mut_ptr()) };
        assert_eq!(&buf[..n], safe.as_slice());
    }

    #[test]
    fn unchecked_decode_matches_safe() {
        let input: Vec<u8> = (0u8..=255).collect();
        let encoded = encode(&input);
        let safe = decode(&encoded);
        let mut buf = vec![0u8; decode_size_hint(encoded.len())];
        let n = unsafe { decode_unchecked(&encoded, buf.as_mut_ptr()) };
        assert_eq!(&buf[..n], safe.as_slice());
    }
}
