// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! The upstream basE91 0.6.0 C implementation, the legacy-format oracle
//! (spec 0008 §4).
//!
//! The vendored sources in `reference/basE91-0.6.0/` are compiled by
//! `build.rs` with their symbols renamed to `ref_*`. This module is a thin,
//! safe wrapper around them.

use core::ffi::{c_int, c_uint, c_ulong, c_void};

/// Mirror of `struct basE91` in upstream `base91.h`.
#[repr(C)]
struct State {
    queue: c_ulong,
    nbits: c_uint,
    val: c_int,
}

extern "C" {
    fn ref_basE91_init(b: *mut State);
    fn ref_basE91_encode(b: *mut State, i: *const c_void, len: usize, o: *mut c_void) -> usize;
    fn ref_basE91_encode_end(b: *mut State, o: *mut c_void) -> usize;
    fn ref_basE91_decode(b: *mut State, i: *const c_void, len: usize, o: *mut c_void) -> usize;
    fn ref_basE91_decode_end(b: *mut State, o: *mut c_void) -> usize;
}

fn new_state() -> State {
    let mut state = State {
        queue: 0,
        nbits: 0,
        val: 0,
    };
    // SAFETY: `state` is a valid, exclusively borrowed `struct basE91`.
    unsafe { ref_basE91_init(&mut state) };
    state
}

/// Streaming upstream encoder: output of any sequence of [`Encoder::encode`]
/// calls followed by [`Encoder::finish`], exactly as upstream produces it.
pub struct Encoder {
    state: State,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub fn new() -> Self {
        Self { state: new_state() }
    }

    /// Appends the encoding of `input` to `output`.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        // Upstream writes at most 2 characters per 13 input bits, so
        // 2 * len + 2 bytes is a comfortable bound.
        let mut buf = vec![0u8; 2 * input.len() + 2];
        // SAFETY: `input` is readable for its length; `buf` has room for
        // the worst case (see above).
        let n = unsafe {
            ref_basE91_encode(
                &mut self.state,
                input.as_ptr().cast(),
                input.len(),
                buf.as_mut_ptr().cast(),
            )
        };
        output.extend_from_slice(&buf[..n]);
    }

    /// Appends the final 0..=2 characters to `output`.
    pub fn finish(mut self, output: &mut Vec<u8>) {
        let mut buf = [0u8; 2];
        // SAFETY: upstream writes at most 2 bytes here.
        let n = unsafe { ref_basE91_encode_end(&mut self.state, buf.as_mut_ptr().cast()) };
        output.extend_from_slice(&buf[..n]);
    }
}

/// Streaming upstream decoder, same contract as [`Encoder`].
pub struct Decoder {
    state: State,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self { state: new_state() }
    }

    /// Appends the decoding of `input` to `output`. Bytes outside the
    /// alphabet are ignored, as upstream does.
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        // Every 2 input characters give at most 2 bytes.
        let mut buf = vec![0u8; input.len() + 2];
        // SAFETY: `input` is readable for its length; `buf` has room for
        // the worst case (see above).
        let n = unsafe {
            ref_basE91_decode(
                &mut self.state,
                input.as_ptr().cast(),
                input.len(),
                buf.as_mut_ptr().cast(),
            )
        };
        output.extend_from_slice(&buf[..n]);
    }

    /// Appends the final 0..=1 byte to `output`.
    pub fn finish(mut self, output: &mut Vec<u8>) {
        let mut buf = [0u8; 1];
        // SAFETY: upstream writes at most 1 byte here.
        let n = unsafe { ref_basE91_decode_end(&mut self.state, buf.as_mut_ptr().cast()) };
        output.extend_from_slice(&buf[..n]);
    }
}

/// One-shot upstream encoding (no line wrapping, like `b91enc`).
pub fn encode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = Encoder::new();
    encoder.encode(input, &mut out);
    encoder.finish(&mut out);
    out
}

/// One-shot upstream decoding.
pub fn decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut decoder = Decoder::new();
    decoder.decode(input, &mut out);
    decoder.finish(&mut out);
    out
}
