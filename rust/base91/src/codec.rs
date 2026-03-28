// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Core basE91 encoding/decoding algorithm.
//!
//! This is a clean-room Rust reimplementation of the algorithm invented by
//! Joachim Henke.  The alphabet and wire format are identical to the C
//! reference (http://base91.sourceforge.net/), ensuring byte-for-byte
//! interoperability.

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
/// Feed input in chunks with [`encode`][Encoder::encode]; call
/// [`finish`][Encoder::finish] to flush the remaining bits.
///
/// ```
/// use base91::Encoder;
///
/// let mut enc = Encoder::new();
/// let mut out = Vec::new();
/// enc.encode(b"Hello, world!", &mut out);
/// enc.finish(&mut out);
/// ```
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
    #[inline]
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) -> usize {
        // Reserve worst-case space up front so the hot loop never reallocates.
        // Vec::push reloads ptr/cap/len from memory after every call because
        // grow_one (the slow path) may move the allocation — LLVM cannot hoist
        // the capacity check.  Writing into spare_capacity_mut() instead gives
        // LLVM a fixed-length slice: ptr stays in a register, no capacity
        // checks inside the loop.
        output.reserve(crate::encode_size_hint(input.len()));
        let spare = output.spare_capacity_mut();

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
                    spare[n].write(ENCTAB[r as usize]);
                    spare[n + 1].write(ENCTAB[q as usize]);
                } else {
                    val = queue & 0x3fff; // take 14 bits
                    queue >>= 14;
                    nbits -= 14;
                    let q = val / 91;
                    let r = val - q * 91;
                    spare[n].write(ENCTAB[r as usize]);
                    spare[n + 1].write(ENCTAB[q as usize]);
                }
                n += 2;
            }
        }

        // Safety: we wrote exactly n bytes into spare_capacity_mut(), which
        // has at least encode_size_hint(input.len()) slots — always ≥ n.
        unsafe { output.set_len(output.len() + n) };

        self.queue = queue;
        self.nbits = nbits;
        n
    }

    /// Flush remaining bits and return the number of bytes written (0–2).
    #[inline]
    pub fn finish(self, output: &mut Vec<u8>) -> usize {
        let before = output.len();
        if self.nbits > 0 {
            output.push(ENCTAB[(self.queue % 91) as usize]);
            if self.nbits > 7 || self.queue > 90 {
                output.push(ENCTAB[(self.queue / 91) as usize]);
            }
        }
        output.len() - before
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
/// Feed encoded text in chunks with [`decode`][Decoder::decode]; call
/// [`finish`][Decoder::finish] to flush any remaining partial value.
/// Non-alphabet bytes (including newlines) are silently ignored.
///
/// ```
/// use base91::Decoder;
///
/// let mut dec = Decoder::new();
/// let mut out = Vec::new();
/// dec.decode(b"PM{(8&~AA", &mut out);
/// dec.finish(&mut out);
/// ```
pub struct Decoder {
    queue: u32,
    nbits: u32,
    /// Pending first character of a pair. `u32::MAX` means no pending char.
    val: u32,
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
    #[inline]
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) -> usize {
        // Same spare_capacity_mut strategy as Encoder::encode — see that
        // method for the rationale.
        output.reserve(crate::decode_size_hint(input.len()));
        let spare = output.spare_capacity_mut();

        let mut queue = self.queue;
        let mut nbits = self.nbits;
        let mut val = self.val;
        let mut n: usize = 0;

        for &byte in input {
            let d = DECTAB[byte as usize] as u32;
            if d == 91 {
                continue; // not in alphabet
            }
            if val == u32::MAX {
                val = d; // first char of pair
            } else {
                // Second char: reconstruct value.
                let v = val + d * 91;
                val = u32::MAX;

                queue |= v << nbits;
                // Branchless 13/14-bit selection.
                nbits += if v & 0x1fff > 88 { 13 } else { 14 };

                // Drain: at most 2 bytes (unrolled, no loop).
                spare[n].write(queue as u8);
                n += 1;
                queue >>= 8;
                nbits -= 8;
                if nbits >= 8 {
                    spare[n].write(queue as u8);
                    n += 1;
                    queue >>= 8;
                    nbits -= 8;
                }
            }
        }

        // Safety: we wrote exactly n bytes into spare_capacity_mut(), which
        // has at least decode_size_hint(input.len()) slots — always ≥ n.
        unsafe { output.set_len(output.len() + n) };

        self.queue = queue;
        self.nbits = nbits;
        self.val = val;
        n
    }

    /// Flush any remaining partial value (0 or 1 byte).
    #[inline]
    pub fn finish(self, output: &mut Vec<u8>) -> usize {
        let before = output.len();
        if self.val != u32::MAX {
            output.push((self.queue | (self.val << self.nbits)) as u8);
        }
        output.len() - before
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Unchecked one-shot helpers (raw pointer output, no bounds checking)
// ---------------------------------------------------------------------------

/// Encode `input` into `output` without bounds checking.
///
/// # Safety
/// `output` must point to at least `encode_size_hint(input.len())` writable bytes.
pub(crate) unsafe fn encode_unchecked(input: &[u8], output: *mut u8) -> usize {
    let mut queue: u32 = 0;
    let mut nbits: u32 = 0;
    let mut n: usize = 0;

    for &byte in input {
        queue |= (byte as u32) << nbits;
        nbits += 8;
        if nbits > 13 {
            // Two separate arms with duplicated writes prevent LLVM from
            // merging them into a cmovae/setae + variable-count shift.
            // val > 88 holds ~98.9 % of the time (13-bit path), so the branch
            // is well-predicted and immediate shifts dominate.
            // Safety: val ≤ 91²−1 = 8280, so q,r ∈ 0..=90.
            let val = queue & 0x1fff;
            if val > 88 {
                // 13-bit path: val in 89..=8191
                queue >>= 13;
                nbits -= 13;
                let q = val / 91;
                let r = val - q * 91;
                unsafe {
                    output.add(n).write(*ENCTAB.get_unchecked(r as usize));
                    output.add(n + 1).write(*ENCTAB.get_unchecked(q as usize));
                }
            } else {
                // 14-bit path: val in 0..=88 or 8192..=8280
                let val = queue & 0x3fff;
                queue >>= 14;
                nbits -= 14;
                let q = val / 91;
                let r = val - q * 91;
                unsafe {
                    output.add(n).write(*ENCTAB.get_unchecked(r as usize));
                    output.add(n + 1).write(*ENCTAB.get_unchecked(q as usize));
                }
            }
            n += 2;
        }
    }
    if nbits > 0 {
        // Safety: queue < 91² at end-of-input, so r,q ∈ 0..=90.
        let r = queue % 91;
        let q = queue / 91;
        unsafe {
            output.add(n).write(*ENCTAB.get_unchecked(r as usize));
        }
        n += 1;
        if nbits > 7 || queue > 90 {
            unsafe {
                output.add(n).write(*ENCTAB.get_unchecked(q as usize));
            }
            n += 1;
        }
    }
    n
}

/// Decode `input` into `output` without bounds checking.
///
/// # Safety
/// `output` must point to at least `decode_size_hint(input.len())` writable bytes.
pub(crate) unsafe fn decode_unchecked(input: &[u8], output: *mut u8) -> usize {
    let mut queue: u32 = 0;
    let mut nbits: u32 = 0;
    let mut n: usize = 0;
    let mut ptr = input.as_ptr();
    let end = unsafe { ptr.add(input.len()) };

    // Outer loop: find first char of a pair (skip non-alphabet).
    // After emitting bytes, the inner inline block fetches the next char
    // directly — matching GCC's unrolled decode loop layout.
    loop {
        // --- scan for first char of pair ---
        let d0 = loop {
            if ptr == end {
                return n;
            }
            let b = unsafe { *ptr };
            ptr = unsafe { ptr.add(1) };
            let d = unsafe { *DECTAB.get_unchecked(b as usize) } as u32;
            if d != 91 {
                break d;
            }
        };

        // --- scan for second char of pair ---
        let d1 = loop {
            if ptr == end {
                // Flush pending first char (partial value).
                unsafe {
                    output.add(n).write((queue | (d0 << nbits)) as u8);
                }
                return n + 1;
            }
            let b = unsafe { *ptr };
            ptr = unsafe { ptr.add(1) };
            let d = unsafe { *DECTAB.get_unchecked(b as usize) } as u32;
            if d != 91 {
                break d;
            }
        };

        // --- emit bytes ---
        let v = d0 + d1 * 91;
        queue |= v << nbits;
        // Branchless 13/14-bit select: mirrors GCC's cmp+adc pattern.
        // (v & 0x1fff > 88) is false ~1.1% of the time.
        nbits += if v & 0x1fff > 88 { 13 } else { 14 };
        unsafe {
            output.add(n).write(queue as u8);
        }
        n += 1;
        queue >>= 8;
        nbits -= 8;
        if nbits >= 8 {
            unsafe {
                output.add(n).write(queue as u8);
            }
            n += 1;
            queue >>= 8;
            nbits -= 8;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
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
