// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Scalar (non-SIMD) fixed-width 13-bit encoder/decoder for the SIMD variant.
//!
//! This is the portable fallback used when no SIMD extensions are available,
//! and also the reference for correctness testing of the SIMD kernels.
//!
//! Wire format: one leading `-` byte, then fixed-width 13-bit groups encoded
//! two characters each using the SIMD alphabet (0x23–0x5B, 0x5D–0x7E).

// ---------------------------------------------------------------------------
// Alphabet helpers
// ---------------------------------------------------------------------------

/// Encode an index 0–90 to a SIMD-alphabet character.
///
/// Alphabet: 0x23–0x5B (indices 0–56) then 0x5D–0x7E (indices 57–90).
/// The gap at 0x5C (`\`) is bridged by adding 1 for the upper range.
#[inline(always)]
pub(crate) fn enc_char(idx: u8) -> u8 {
    debug_assert!(idx < 91);
    if idx < 57 {
        idx + 0x23
    } else {
        idx + 0x24 // skip 0x5C (`\`)
    }
}

/// Decode a SIMD-alphabet character to an index 0–90, or `None` if invalid.
#[cfg(test)]
fn dec_char(b: u8) -> Option<u8> {
    match b {
        0x23..=0x5B => Some(b - 0x23),
        0x5D..=0x7E => Some(b - 0x24),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Encoder state
// ---------------------------------------------------------------------------

/// Stateful scalar fixed-width encoder.
pub struct ScalarEncoder {
    queue: u32,
    nbits: u32,
}

impl ScalarEncoder {
    pub fn new() -> Self {
        Self { queue: 0, nbits: 0 }
    }

    /// Encode `input` bytes, appending SIMD-alphabet characters to `output`.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        // Upper bound: ceil(input_len * 16 / 13) + 2 for finish headroom.
        let max_out = (input.len() * 16).div_ceil(13) + 2;
        output.reserve(max_out);

        let mut queue = self.queue;
        let mut nbits = self.nbits;

        let base_len = output.len();
        let spare = output.spare_capacity_mut();
        let mut n = 0usize;

        // Macro: extract one 13-bit group from queue, emit 2 chars.
        macro_rules! emit_group {
            () => {{
                let val = queue & 0x1FFF;
                queue >>= 13;
                nbits -= 13;
                let hi = (val * 2881) >> 18;
                let lo = val - hi * 91;
                unsafe {
                    spare.get_unchecked_mut(n).write(enc_char(lo as u8));
                    spare.get_unchecked_mut(n + 1).write(enc_char(hi as u8));
                }
                n += 2;
            }};
        }

        // Main loop: consume 13 bytes at a time → 16 chars, no branch inside.
        // 13 bytes = 104 bits = 8 × 13-bit groups, so nbits is always in a
        // known state at block boundaries (carry from previous chunk included).
        let mut i = 0usize;
        while i + 13 <= input.len() {
            // Load 13 bytes into queue, emitting a group whenever we have ≥13 bits.
            // nbits starts at 0..12 (carry from previous block or chunk).
            macro_rules! ingest {
                ($b:expr) => {{
                    queue |= ($b as u32) << nbits;
                    nbits += 8;
                    if nbits >= 13 {
                        emit_group!();
                    }
                }};
            }
            ingest!(unsafe { *input.get_unchecked(i) });
            ingest!(unsafe { *input.get_unchecked(i + 1) });
            ingest!(unsafe { *input.get_unchecked(i + 2) });
            ingest!(unsafe { *input.get_unchecked(i + 3) });
            ingest!(unsafe { *input.get_unchecked(i + 4) });
            ingest!(unsafe { *input.get_unchecked(i + 5) });
            ingest!(unsafe { *input.get_unchecked(i + 6) });
            ingest!(unsafe { *input.get_unchecked(i + 7) });
            ingest!(unsafe { *input.get_unchecked(i + 8) });
            ingest!(unsafe { *input.get_unchecked(i + 9) });
            ingest!(unsafe { *input.get_unchecked(i + 10) });
            ingest!(unsafe { *input.get_unchecked(i + 11) });
            ingest!(unsafe { *input.get_unchecked(i + 12) });
            i += 13;
        }

        // Tail: fewer than 13 bytes remaining.
        while i < input.len() {
            queue |= (unsafe { *input.get_unchecked(i) } as u32) << nbits;
            nbits += 8;
            if nbits >= 13 {
                emit_group!();
            }
            i += 1;
        }

        unsafe { output.set_len(base_len + n) };

        self.queue = queue;
        self.nbits = nbits;
    }

    /// Flush remaining bits (0–2 chars).
    pub fn finish(self, output: &mut Vec<u8>) {
        if self.nbits > 0 {
            let val = self.queue & 0x1FFF;
            let hi = (val * 2881) >> 18;
            let lo = val - hi * 91;
            output.push(enc_char(lo as u8));
            if self.nbits > 6 || hi > 0 {
                output.push(enc_char(hi as u8));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder state
// ---------------------------------------------------------------------------

/// Stateful scalar fixed-width decoder.
pub struct ScalarDecoder {
    queue: u32,
    nbits: u32,
    /// Pending first character of a pair. `u32::MAX` = none.
    first: u32,
}

impl ScalarDecoder {
    pub fn new() -> Self {
        Self {
            queue: 0,
            nbits: 0,
            first: u32::MAX,
        }
    }

    /// Decode `input` SIMD-alphabet characters, appending bytes to `output`.
    ///
    /// Processes 16 alphabet characters at a time (8 pairs → up to 13 bytes).
    /// A single `\n` after each 16-char block is silently skipped (--wrap).
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        output.reserve(input.len());

        let mut queue = self.queue;
        let mut nbits = self.nbits;
        let carry = self.first; // u32::MAX = no pending char

        let base_len = output.len();
        let spare = output.spare_capacity_mut();
        let mut n = 0usize;
        let mut i = 0usize;

        // Macro: decode one raw byte to an index and emit the pair once we
        // have both halves.
        macro_rules! emit_pair {
            ($d0:expr, $d1:expr) => {{
                let val = $d0 + $d1 * 91;
                queue |= val << nbits;
                nbits += 13;
                unsafe { spare.get_unchecked_mut(n).write(queue as u8) };
                n += 1;
                queue >>= 8;
                nbits -= 8;
                if nbits >= 8 {
                    unsafe { spare.get_unchecked_mut(n).write(queue as u8) };
                    n += 1;
                    queue >>= 8;
                    nbits -= 8;
                }
            }};
        }

        macro_rules! idx {
            ($b:expr) => {{
                let b = $b as u32;
                b.wrapping_sub(0x23).wrapping_sub((b > 0x5C) as u32)
            }};
        }

        // Handle pending carry from previous chunk boundary.
        if carry != u32::MAX {
            if i >= input.len() {
                unsafe { output.set_len(base_len + n) };
                self.queue = queue;
                self.nbits = nbits;
                self.first = carry;
                return;
            }
            let d1 = idx!(unsafe { *input.get_unchecked(i) });
            i += 1;
            emit_pair!(carry, d1);
        }

        // Main loop: 16 alphabet chars per iteration, optional \n after.
        while i + 16 <= input.len() {
            // SAFETY: i+16 <= input.len() guarantees 16 readable bytes.
            let d: [u32; 16] =
                std::array::from_fn(|k| idx!(unsafe { *input.get_unchecked(i + k) }));
            i += 16;

            emit_pair!(d[0], d[1]);
            emit_pair!(d[2], d[3]);
            emit_pair!(d[4], d[5]);
            emit_pair!(d[6], d[7]);
            emit_pair!(d[8], d[9]);
            emit_pair!(d[10], d[11]);
            emit_pair!(d[12], d[13]);
            emit_pair!(d[14], d[15]);

            // Skip optional \n after each 16-char block (--wrap).
            if i < input.len() && unsafe { *input.get_unchecked(i) } == b'\n' {
                i += 1;
            }
        }

        // Tail: fewer than 16 chars remaining — process pairs one at a time.
        loop {
            if i >= input.len() {
                unsafe { output.set_len(base_len + n) };
                self.queue = queue;
                self.nbits = nbits;
                self.first = u32::MAX;
                return;
            }
            let d0 = idx!(unsafe { *input.get_unchecked(i) });
            i += 1;

            if i >= input.len() {
                unsafe { output.set_len(base_len + n) };
                self.queue = queue;
                self.nbits = nbits;
                self.first = d0;
                return;
            }
            let d1 = idx!(unsafe { *input.get_unchecked(i) });
            i += 1;

            emit_pair!(d0, d1);
        }
    }

    /// Flush any remaining state (0 or 1 byte).
    pub fn finish(self, output: &mut Vec<u8>) {
        if self.first != u32::MAX {
            // Odd trailing character — flush partial byte.
            let val = self.first;
            output.push((self.queue | (val << self.nbits)) as u8);
        }
    }
}

// ---------------------------------------------------------------------------
// One-shot helpers
// ---------------------------------------------------------------------------

/// Encode `input` to a new `Vec<u8>` using the SIMD variant (scalar path).
/// The returned bytes begin with `-` followed by SIMD-alphabet characters.
pub fn encode(input: &[u8]) -> Vec<u8> {
    // Upper bound: 1 (prefix) + ceil(input_len * 16 / 13) + 2 (finish)
    let cap = 1 + (input.len() * 16).div_ceil(13) + 2;
    let mut out = Vec::with_capacity(cap);
    out.push(b'-');
    let mut enc = ScalarEncoder::new();
    enc.encode(input, &mut out);
    enc.finish(&mut out);
    out
}

/// Decode `input` (SIMD-variant stream starting with `-`) to a new `Vec<u8>`.
///
/// Returns `None` if `input` does not start with `-`.
/// Non-alphabet bytes within the payload are silently ignored.
pub fn decode(input: &[u8]) -> Option<Vec<u8>> {
    let payload = input.strip_prefix(b"-")?;
    let mut out = Vec::with_capacity(payload.len() * 7 / 8 + 1);
    let mut dec = ScalarDecoder::new();
    dec.decode(payload, &mut out);
    dec.finish(&mut out);
    Some(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(input: &[u8]) -> Vec<u8> {
        let encoded = encode(input);
        assert_eq!(encoded[0], b'-', "missing '-' prefix");
        decode(&encoded).expect("decode failed")
    }

    #[test]
    fn empty() {
        assert_eq!(round_trip(b""), b"");
    }

    #[test]
    fn all_bytes() {
        let input: Vec<u8> = (0u8..=255).collect();
        assert_eq!(round_trip(&input), input);
    }

    #[test]
    fn all_zeros() {
        let input = vec![0u8; 256];
        assert_eq!(round_trip(&input), input);
    }

    #[test]
    fn all_ones() {
        let input = vec![0xffu8; 256];
        assert_eq!(round_trip(&input), input);
    }

    #[test]
    fn hello() {
        assert_eq!(round_trip(b"Hello, world!"), b"Hello, world!");
    }

    #[test]
    fn chunk_independence() {
        let input: Vec<u8> = (0u8..=255).collect();
        let reference = encode(&input);
        for chunk_size in [1, 2, 3, 7, 13, 64, 256] {
            let mut enc = ScalarEncoder::new();
            let mut out = vec![b'-'];
            for chunk in input.chunks(chunk_size) {
                enc.encode(chunk, &mut out);
            }
            enc.finish(&mut out);
            assert_eq!(out, reference, "chunk_size={chunk_size}");
        }
    }

    #[test]
    fn enc_char_coverage() {
        // Every index 0–90 maps to a unique valid character.
        let mut seen = std::collections::HashSet::new();
        for i in 0u8..91 {
            let c = enc_char(i);
            assert!(dec_char(c) == Some(i), "enc/dec mismatch at {i}");
            assert!(seen.insert(c), "duplicate char {c} at index {i}");
        }
    }

    #[test]
    fn backslash_not_in_alphabet() {
        assert_eq!(dec_char(b'\\'), None);
    }

    #[test]
    fn prefix_required_for_decode() {
        let encoded = encode(b"test");
        // Strip the '-' prefix — decode should fail.
        assert!(decode(&encoded[1..]).is_none());
    }

    #[test]
    fn decode_skips_newline_at_16_boundary() {
        // Use input large enough to produce at least one 16-char block.
        let input: Vec<u8> = (0u8..=255).cycle().take(100).collect();
        let encoded = encode(&input);
        // Insert \n after every 16 payload chars (after the '-' prefix).
        let mut with_nl = vec![b'-'];
        for (i, &b) in encoded[1..].iter().enumerate() {
            with_nl.push(b);
            if i % 16 == 15 {
                with_nl.push(b'\n');
            }
        }
        assert_eq!(decode(&with_nl).unwrap(), input);
    }

    #[test]
    fn large_round_trip() {
        let input: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        assert_eq!(round_trip(&input), input);
    }
}
