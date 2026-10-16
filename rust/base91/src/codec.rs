// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Core basE91 encoding/decoding algorithm.
//!
//! This is a clean-room Rust reimplementation of the algorithm invented by
//! Joachim Henke.  The alphabet and wire format are identical to the C
//! reference (<http://base91.sourceforge.net/>), ensuring byte-for-byte
//! interoperability.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::mem::MaybeUninit;

use crate::Error;

/// The 91-character encoding alphabet, in canonical order.
///
/// Index 0–90 maps to the printable ASCII character used to represent that
/// value.  Identical to `enctab[]` in the C reference.
pub(crate) const ENCTAB: &[u8; 91] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz\
                                        0123456789!#$%&()*+,./:;<=>?@[]^_`{|}~\"";

/// Reverse lookup: ASCII byte → alphabet value (0–90), or 91 for invalid.
///
/// Identical to `dectab[]` in the C reference.
pub(crate) const DECTAB: &[u8; 256] = &{
    let mut t = [91u8; 256];
    let mut i = 0u8;
    loop {
        // Find the position of byte i in ENCTAB.
        let mut j = 0usize;
        loop {
            if ENCTAB[j] == i {
                t[i as usize] = j as u8;
                break;
            }
            j += 1;
            if j == 91 {
                break; // i not in alphabet → stays 91
            }
        }
        if i == 255 {
            break;
        }
        i += 1;
    }
    t
};

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// Stateful basE91 encoder.
///
/// Feed input in chunks with `encode` (or
/// [`encode_slice`][Encoder::encode_slice]); call `finish`
/// (or [`finish_slice`][Encoder::finish_slice]) to flush the remaining bits.
///
/// ```
/// use base91::Encoder;
///
/// let mut enc = Encoder::new();
/// let mut out = Vec::new();
/// enc.encode(b"Hello, world!", &mut out);
/// enc.finish(&mut out);
/// ```
#[derive(Clone, Debug)]
pub struct Encoder {
    queue: u32,
    nbits: u32,
}

impl Encoder {
    /// Create a new encoder with empty state.
    #[inline]
    pub fn new() -> Self {
        Self { queue: 0, nbits: 0 }
    }

    /// Encode `input`, appending base91 characters to `output`.
    ///
    /// Returns the number of bytes written.
    #[cfg(feature = "alloc")]
    #[inline]
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) -> usize {
        // Writing into the spare capacity instead of pushing gives LLVM a
        // fixed-length slice: no capacity checks inside the loop.
        output.reserve(crate::encode_size_hint(input.len()));
        let n = self.encode_uninit(input, output.spare_capacity_mut());
        // SAFETY: `encode_uninit` initialized the first `n` bytes of the
        // spare capacity.
        unsafe { output.set_len(output.len() + n) };
        n
    }

    /// Encode `input` into `output`, and return the number of bytes written.
    ///
    /// `output` must hold at least `encode_size_hint(input.len())` bytes;
    /// otherwise nothing is written and the error gives that size.
    pub fn encode_slice(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let needed = crate::encode_size_hint(input.len());
        if output.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        Ok(self.encode_uninit(input, crate::as_uninit(output)))
    }

    /// Flush remaining bits and return the number of bytes written (0–2).
    #[cfg(feature = "alloc")]
    #[inline]
    pub fn finish(mut self, output: &mut Vec<u8>) -> usize {
        output.reserve(2);
        let n = self.finish_uninit(output.spare_capacity_mut());
        // SAFETY: as in `encode`.
        unsafe { output.set_len(output.len() + n) };
        n
    }

    /// Flush remaining bits into `output`, return the number of bytes
    /// written (0–2), and reset the encoder.
    ///
    /// `output` must hold the bytes to write; otherwise nothing is written,
    /// the state is kept, and the error gives the size.
    pub fn finish_slice(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let needed = self.finish_len();
        if output.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        Ok(self.finish_uninit(crate::as_uninit(output)))
    }

    /// Encoding core: `out` holds at least `encode_size_hint(input.len())`
    /// bytes (the pending bits, at most 13, never make more than that).
    #[inline]
    pub(crate) fn encode_uninit(&mut self, input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
        // Hoist state to locals so LLVM can keep them in registers.
        let mut queue = self.queue;
        let mut nbits = self.nbits;
        let mut n: usize = 0;

        for &byte in input {
            queue |= (byte as u32) << nbits;
            nbits += 8;
            if nbits > 13 {
                let mut val = queue & 0x1fff; // peek 13 bits
                if val > 88 {
                    queue >>= 13;
                    nbits -= 13;
                    // Duplicate writes per arm: keeps each path independent so
                    // LLVM emits immediate-count shifts instead of setae+shr cl.
                    let q = val / 91;
                    let r = val - q * 91;
                    out[n].write(ENCTAB[r as usize]);
                    out[n + 1].write(ENCTAB[q as usize]);
                } else {
                    val = queue & 0x3fff; // take 14 bits
                    queue >>= 14;
                    nbits -= 14;
                    let q = val / 91;
                    let r = val - q * 91;
                    out[n].write(ENCTAB[r as usize]);
                    out[n + 1].write(ENCTAB[q as usize]);
                }
                n += 2;
            }
        }

        self.queue = queue;
        self.nbits = nbits;
        n
    }

    /// An encoder in the given state (Henke's `queue`, `nbits`).
    pub(crate) fn from_state(queue: u32, nbits: u32) -> Self {
        Self { queue, nbits }
    }

    /// The state: Henke's `queue`, `nbits`.
    pub(crate) fn state(&self) -> (u32, u32) {
        (self.queue, self.nbits)
    }

    /// Upper bound on what `encode_uninit` writes for `n` input bytes from
    /// this state: two characters per group of at least 13 bits.
    pub(crate) fn encode_len(&self, n: usize) -> usize {
        2 * ((self.nbits as usize + 8 * n) / 13)
    }

    /// Bytes `finish` writes: 0, 1 or 2.
    pub(crate) fn finish_len(&self) -> usize {
        match self.nbits {
            0 => 0,
            _ if self.nbits > 7 || self.queue > 90 => 2,
            _ => 1,
        }
    }

    pub(crate) fn finish_uninit(&mut self, out: &mut [MaybeUninit<u8>]) -> usize {
        let n = self.finish_len();
        if n > 0 {
            out[0].write(ENCTAB[(self.queue % 91) as usize]);
        }
        if n > 1 {
            out[1].write(ENCTAB[(self.queue / 91) as usize]);
        }
        *self = Self::new();
        n
    }
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Stateful basE91 decoder.
///
/// Feed encoded text in chunks with `decode` (or
/// [`decode_slice`][Decoder::decode_slice]); call `finish`
/// (or [`finish_slice`][Decoder::finish_slice]) to flush any remaining
/// partial value. Non-alphabet bytes (including newlines) are silently
/// ignored.
///
/// ```
/// use base91::Decoder;
///
/// let mut dec = Decoder::new();
/// let mut out = Vec::new();
/// dec.decode(b"PM{(8&~AA", &mut out);
/// dec.finish(&mut out);
/// ```
#[derive(Clone, Debug)]
pub struct Decoder {
    queue: u32,
    nbits: u32,
    /// Pending first character of a pair. `u32::MAX` means no pending char.
    val: u32,
}

/// Appends the 13- or 14-bit group of pair value `v` to the bit queue and
/// drains the whole bytes into `out` at `n`; returns the new `n`.
#[inline(always)]
fn push_pair(
    v: u32,
    queue: &mut u32,
    nbits: &mut u32,
    out: &mut [MaybeUninit<u8>],
    mut n: usize,
) -> usize {
    *queue |= v << *nbits;
    // Branchless 13/14-bit selection.
    *nbits += if v & 0x1fff > 88 { 13 } else { 14 };

    // Drain: at most 2 bytes (unrolled, no loop).
    out[n].write(*queue as u8);
    n += 1;
    *queue >>= 8;
    *nbits -= 8;
    if *nbits >= 8 {
        out[n].write(*queue as u8);
        n += 1;
        *queue >>= 8;
        *nbits -= 8;
    }
    n
}

impl Decoder {
    /// Create a new decoder with empty state.
    #[inline]
    pub fn new() -> Self {
        Self {
            queue: 0,
            nbits: 0,
            val: u32::MAX,
        }
    }

    /// Decode `input`, appending binary bytes to `output`.
    ///
    /// Non-alphabet bytes are silently skipped.
    /// Returns the number of bytes written.
    #[cfg(feature = "alloc")]
    #[inline]
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) -> usize {
        output.reserve(self.decode_len(input.len()));
        let n = self.decode_uninit(input, output.spare_capacity_mut());
        // SAFETY: `decode_uninit` initialized the first `n` bytes of the
        // spare capacity.
        unsafe { output.set_len(output.len() + n) };
        n
    }

    /// Decode `input` into `output`, and return the number of bytes written.
    ///
    /// `output` must hold at least `decode_size_hint(input.len())` bytes, or
    /// `decode_size_hint(input.len() + 1)` when a character is pending from
    /// the previous call; otherwise nothing is written and the error gives
    /// that size.
    pub fn decode_slice(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let needed = self.decode_len(input.len());
        if output.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        Ok(self.decode_uninit(input, crate::as_uninit(output)))
    }

    /// Flush any remaining partial value (0 or 1 byte).
    #[cfg(feature = "alloc")]
    #[inline]
    pub fn finish(mut self, output: &mut Vec<u8>) -> usize {
        output.reserve(1);
        let n = self.finish_uninit(output.spare_capacity_mut());
        // SAFETY: as in `decode`.
        unsafe { output.set_len(output.len() + n) };
        n
    }

    /// Flush any remaining partial value into `output`, return the number of
    /// bytes written (0 or 1), and reset the decoder.
    ///
    /// `output` must hold the bytes to write; otherwise nothing is written,
    /// the state is kept, and the error gives the size.
    pub fn finish_slice(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let needed = self.finish_len();
        if output.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        Ok(self.finish_uninit(crate::as_uninit(output)))
    }

    /// A decoder in the given state (Henke's `queue`, `nbits`, and `val`,
    /// `u32::MAX` for Henke's -1: no pending character).
    pub(crate) fn from_state(queue: u32, nbits: u32, val: u32) -> Self {
        Self { queue, nbits, val }
    }

    /// The state: Henke's `queue`, `nbits`, `val` (`u32::MAX` for -1).
    pub(crate) fn state(&self) -> (u32, u32, u32) {
        (self.queue, self.nbits, self.val)
    }

    /// Exact upper bound on what `decode_uninit` writes for `n` input
    /// bytes from this state: at most 14 bits per pair of characters, a
    /// pending character included.
    pub(crate) fn decode_len_exact(&self, n: usize) -> usize {
        let pairs = (n + usize::from(self.val != u32::MAX)) / 2;
        (self.nbits as usize + 14 * pairs) / 8
    }

    /// Upper bound on what one call decodes from `n` bytes: a character
    /// pending from the previous call can complete one more pair (spec 0006
    /// §5b).
    pub(crate) fn decode_len(&self, n: usize) -> usize {
        crate::decode_size_hint(n + usize::from(self.val != u32::MAX))
    }

    /// Decoding core: `out` holds at least `decode_len(input.len())` bytes.
    #[inline]
    pub(crate) fn decode_uninit(&mut self, input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
        let mut queue = self.queue;
        let mut nbits = self.nbits;
        let mut val = self.val;
        let mut n: usize = 0;

        let mut i = 0;
        while i < input.len() {
            // Fast path: while no character is pending, decode whole pairs of
            // alphabet bytes, one branch per pair.
            if val == u32::MAX {
                let mut rest = &input[i..];
                while let [b0, b1, tail @ ..] = rest {
                    let d0 = DECTAB[*b0 as usize] as u32;
                    let d1 = DECTAB[*b1 as usize] as u32;
                    if (d0 == 91) | (d1 == 91) {
                        break;
                    }
                    rest = tail;
                    n = push_pair(d0 + d1 * 91, &mut queue, &mut nbits, out, n);
                }
                i = input.len() - rest.len();
                if i == input.len() {
                    break;
                }
            }
            // Slow path: one byte (a junk byte, a pending character, or the
            // last byte).
            let d = DECTAB[input[i] as usize] as u32;
            i += 1;
            if d == 91 {
                continue; // not in alphabet
            }
            if val == u32::MAX {
                val = d; // first char of pair
            } else {
                // Second char: reconstruct value.
                n = push_pair(val + d * 91, &mut queue, &mut nbits, out, n);
                val = u32::MAX;
            }
        }

        self.queue = queue;
        self.nbits = nbits;
        self.val = val;
        n
    }

    /// Bytes `finish` writes: 0, or 1 for a pending character.
    pub(crate) fn finish_len(&self) -> usize {
        usize::from(self.val != u32::MAX)
    }

    pub(crate) fn finish_uninit(&mut self, out: &mut [MaybeUninit<u8>]) -> usize {
        let n = self.finish_len();
        if n > 0 {
            out[0].write((self.queue | (self.val << self.nbits)) as u8);
        }
        *self = Self::new();
        n
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    fn encode_all(input: &[u8]) -> Vec<u8> {
        let mut enc = Encoder::new();
        let mut out = Vec::new();
        enc.encode(input, &mut out);
        enc.finish(&mut out);
        out
    }

    fn decode_all(input: &[u8]) -> Vec<u8> {
        let mut dec = Decoder::new();
        let mut out = Vec::new();
        dec.decode(input, &mut out);
        dec.finish(&mut out);
        out
    }

    #[test]
    fn round_trip_empty() {
        let encoded = encode_all(b"");
        assert_eq!(encoded, b"");
        assert_eq!(decode_all(&encoded), b"");
    }

    #[test]
    fn round_trip_all_bytes() {
        let input: Vec<u8> = (0u8..=255).collect();
        let encoded = encode_all(&input);
        let decoded = decode_all(&encoded);
        assert_eq!(decoded, input);
    }

    #[test]
    fn round_trip_all_zeros() {
        let input = vec![0u8; 256];
        assert_eq!(decode_all(&encode_all(&input)), input);
    }

    #[test]
    fn round_trip_all_ones() {
        let input = vec![0xffu8; 256];
        assert_eq!(decode_all(&encode_all(&input)), input);
    }

    #[test]
    fn known_vector_hello() {
        // "Hello, world!" encoded with the C reference tool.
        let encoded = encode_all(b"Hello, world!");
        let decoded = decode_all(&encoded);
        assert_eq!(decoded, b"Hello, world!");
    }

    #[test]
    fn streaming_chunk_boundary_independence() {
        // Same output regardless of how input is chunked.
        let input: Vec<u8> = (0u8..=255).collect();
        let reference = encode_all(&input);

        for chunk_size in [1, 2, 3, 7, 13, 64, 128, 256] {
            let mut enc = Encoder::new();
            let mut out = Vec::new();
            for chunk in input.chunks(chunk_size) {
                enc.encode(chunk, &mut out);
            }
            enc.finish(&mut out);
            assert_eq!(out, reference, "chunk_size={chunk_size}");
        }
    }

    #[test]
    fn decode_ignores_non_alphabet() {
        // Newlines and spaces inserted into encoded data are ignored.
        let input = b"Hello, world!";
        let encoded = encode_all(input);

        // Insert whitespace every 4 chars.
        let mut with_ws = Vec::new();
        for (i, &b) in encoded.iter().enumerate() {
            if i > 0 && i % 4 == 0 {
                with_ws.push(b'\n');
            }
            with_ws.push(b);
        }
        assert_eq!(decode_all(&with_ws), input);
    }

    #[test]
    fn dectab_is_inverse_of_enctab() {
        for (i, &c) in ENCTAB.iter().enumerate() {
            assert_eq!(DECTAB[c as usize], i as u8, "char {c} at index {i}");
        }
    }

    #[test]
    fn dectab_invalid_chars_are_91() {
        for c in 0u8..=255 {
            if !ENCTAB.contains(&c) {
                assert_eq!(DECTAB[c as usize], 91, "char {c} should be invalid");
            }
        }
    }
}
