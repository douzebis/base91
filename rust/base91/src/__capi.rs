// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Support for `base91-capi`, the C library (spec 0014 §3). Hidden: not part
//! of the documented API, and outside its semver promises.
//!
//! Henke's C functions keep the codec's state in the caller's `struct
//! basE91` between calls, and write into the caller's (uninitialized)
//! memory. These functions load and store that state, and run the codecs on
//! `MaybeUninit` output. They are safe: each checks the output's length
//! against the bound of what it may write, and panics if it is too short,
//! before writing anything.

use core::mem::MaybeUninit;

use crate::simd::SimdLevel;
use crate::{Decoder, Encoder, Error};

/// An encoder in Henke's state (`queue`, `nbits`).
pub fn encoder(queue: u32, nbits: u32) -> Encoder {
    Encoder::from_state(queue, nbits)
}

/// The encoder's state (`queue`, `nbits`).
pub fn encoder_state(encoder: &Encoder) -> (u32, u32) {
    encoder.state()
}

/// Upper bound on what [`encode`] writes for `n` input bytes.
pub fn encode_len(encoder: &Encoder, n: usize) -> usize {
    encoder.encode_len(n)
}

/// Encode `input` into `out`, which holds at least
/// [`encode_len`]`(encoder, input.len())` bytes; return the bytes written.
pub fn encode(encoder: &mut Encoder, input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
    assert!(out.len() >= encoder.encode_len(input.len()));
    encoder.encode_uninit(input, out)
}

/// What [`encode_finish`] writes: 0, 1 or 2 bytes.
pub fn encode_finish_len(encoder: &Encoder) -> usize {
    encoder.finish_len()
}

/// Flush the encoder into `out` (at least [`encode_finish_len`] bytes) and
/// reset it; return the bytes written.
pub fn encode_finish(encoder: &mut Encoder, out: &mut [MaybeUninit<u8>]) -> usize {
    assert!(out.len() >= encoder.finish_len());
    encoder.finish_uninit(out)
}

/// A decoder in Henke's state (`queue`, `nbits`, `val`; `u32::MAX` for -1).
pub fn decoder(queue: u32, nbits: u32, val: u32) -> Decoder {
    Decoder::from_state(queue, nbits, val)
}

/// The decoder's state (`queue`, `nbits`, `val`; `u32::MAX` for -1).
pub fn decoder_state(decoder: &Decoder) -> (u32, u32, u32) {
    decoder.state()
}

/// Upper bound on what [`decode`] writes for `n` input bytes.
pub fn decode_len(decoder: &Decoder, n: usize) -> usize {
    decoder.decode_len_exact(n)
}

/// Decode `input` into `out`, which holds at least
/// [`decode_len`]`(decoder, input.len())` bytes; return the bytes written.
pub fn decode(decoder: &mut Decoder, input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
    assert!(out.len() >= decoder.decode_len_exact(input.len()));
    decoder.decode_uninit(input, out)
}

/// What [`decode_finish`] writes: 0 or 1 byte.
pub fn decode_finish_len(decoder: &Decoder) -> usize {
    decoder.finish_len()
}

/// Flush the decoder into `out` (at least [`decode_finish_len`] bytes) and
/// reset it; return the bytes written.
pub fn decode_finish(decoder: &mut Decoder, out: &mut [MaybeUninit<u8>]) -> usize {
    assert!(out.len() >= decoder.finish_len());
    decoder.finish_uninit(out)
}

/// Henke's format, one shot: `out` holds at least
/// [`crate::encode_size_hint`]`(input.len())` bytes.
pub fn henke_encode(input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
    assert!(out.len() >= crate::encode_size_hint(input.len()));
    crate::encode_uninit(input, out)
}

/// Either format, detected, one shot: `out` holds at least
/// [`crate::decode_size_hint`]`(input.len())` bytes.
pub fn decode_any(input: &[u8], out: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
    assert!(out.len() >= crate::decode_size_hint(input.len()));
    crate::simd::decode_uninit(input, SimdLevel::default(), out)
}

/// The SIMD variant, one shot: `out` holds exactly
/// [`crate::simd::encoded_len`]`(input.len(), wrap)` bytes.
pub fn simd_encode(input: &[u8], wrap: usize, out: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
    let needed = crate::simd::encoded_len(input.len(), wrap)?;
    assert!(out.len() >= needed);
    Ok(crate::simd::encode_uninit(
        input,
        SimdLevel::default(),
        wrap,
        &mut out[..needed],
    ))
}
