// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! ARM NEON kernel for the fixed-width basE91 variant.
//!
//! NEON is mandatory on aarch64 — no runtime feature detection needed.
//! Processes one 13-byte block (→ 16 chars) per call using 128-bit
//! NEON registers.
//!
//! # Bit packing
//!
//! As in `x86.rs` (spec 0011): groups 2k and 2k+1 form a 26-bit field at
//! bit 2k of byte 3k, one field per 32-bit lane once aligned by a per-lane
//! shift. The encoder gathers bytes 3k..3k+3 into lane k with one
//! `vqtbl1q_u8` and splits the fields with variable shifts (`vshlq`); the
//! decoder joins value pairs with a shift-and-insert (`vsliq_n_u32`) and
//! places the fields with two `vqtbl1q_u8`.
//!
//! # Division by 91
//!
//! hi = (v * 2881) >> 18, lo = v - hi * 91 (the constants of `x86.rs`),
//! with a doubling multiply-high: `vqdmulhq_n_s16(v, 2881)` is
//! (2 v 2881) >> 16, then a shift by 3.

#![allow(non_snake_case)]

#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

const MAGIC91: i16 = 2881;

/// Encode 13 input bytes into 16 SIMD-alphabet output characters.
///
/// # Safety
/// `input` must be readable for at least 16 bytes (padded); `output` for 16.
/// Only safe to call on aarch64.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn encode_block_neon(input: *const u8, output: *mut u8) {
    let data = vld1q_u8(input);

    // -----------------------------------------------------------------------
    // Step 1: unpack the 13 bytes into eight 13-bit values.
    //
    // Lane k gathers bytes 3k..3k+3: its field (groups 2k, 2k+1) starts at
    // bit 2k. A shift by 3 - 2k (negative: right) puts every field on bits
    // 3..28: in 16-bit halves, group 2k at bits 3..15 of the low half and
    // group 2k+1 at bits 0..12 of the high half. A 16-bit shift of the low
    // halves by 3 and an AND finish the split.
    // -----------------------------------------------------------------------
    let gather: [u8; 16] = [0, 1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 9, 9, 10, 11, 12];
    let gathered = vreinterpretq_u32_u8(vqtbl1q_u8(data, vld1q_u8(gather.as_ptr())));
    let field_shift: [i32; 4] = [3, 1, -1, -3];
    let aligned = vshlq_u32(gathered, vld1q_s32(field_shift.as_ptr()));
    let half_shift: [i16; 8] = [-3, 0, -3, 0, -3, 0, -3, 0];
    let vals = vandq_u16(
        vshlq_u16(
            vreinterpretq_u16_u32(aligned),
            vld1q_s16(half_shift.as_ptr()),
        ),
        vdupq_n_u16(0x1FFF),
    );
    // vals = [g0,g1,g2,g3, g4,g5,g6,g7] as u16x8.

    // -----------------------------------------------------------------------
    // Step 2: divide by 91: hi = (v * 2881) >> 18, as (2 v 2881) >> 16 (no
    // saturation: 2 * 8191 * 2881 < 2^31), then >> 3.
    // -----------------------------------------------------------------------
    let hi = vshrq_n_s16(vqdmulhq_n_s16(vreinterpretq_s16_u16(vals), MAGIC91), 3);

    // -----------------------------------------------------------------------
    // Step 3: the output byte pairs [lo, hi] as one 16-bit lane each:
    // lo + 256 hi = v + 165 hi.
    // -----------------------------------------------------------------------
    let pairs = vmlaq_n_u16(vals, vreinterpretq_u16_s16(hi), 165);

    // -----------------------------------------------------------------------
    // Step 4: Map indices 0–90 to SIMD-alphabet characters.
    //   indices 0–3   → 0x23 + index
    //   indices 4–90  → 0x24 + index  (skip 0x27 = '\'')
    // Add 0x23, then subtract cmpgt mask (0xFF where > 0x26) to add 1.
    // -----------------------------------------------------------------------
    let chars = vaddq_u8(vreinterpretq_u8_u16(pairs), vdupq_n_u8(0x23));
    let needs_bump = vcgtq_u8(chars, vdupq_n_u8(0x26)); // 0xFF where > 0x26
    vst1q_u8(output, vsubq_u8(chars, needs_bump)); // subtract 0xFF = add 1
}

/// Decode 16 SIMD-alphabet characters into 13 output bytes.
///
/// Returns what the batch validates (spec 0011 §4): the indices, `'` forced
/// to 0xFF, and the pair values. The output is written either way.
///
/// # Safety
/// `input` must be readable for 16 bytes; `output` writable for 13.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn decode_block_neon(input: *const u8, output: *mut u8) -> (uint8x16_t, uint16x8_t) {
    let chars = vld1q_u8(input);

    // -----------------------------------------------------------------------
    // Step 1: Reverse map characters to indices 0–90.
    //   chars 0x23–0x26: index = char - 0x23
    //   chars 0x28–0x7E: index = char - 0x24  (= (char - 0x23) - 1)
    // needs_bump = 0xFF where char > 0x26; subtracting 0xFF adds 1.
    // index = (char - 0x23) + needs_bump   (needs_bump is 0xFF = -1 in u8)
    // -----------------------------------------------------------------------
    let base = vdupq_n_u8(0x23);
    let threshold = vdupq_n_u8(0x26);
    let needs_bump = vcgtq_u8(chars, threshold); // 0xFF where > 0x26
    let raw = vsubq_u8(chars, base);
    let indices8 = vaddq_u8(raw, needs_bump); // subtracts 1 where > 0x26

    // Validity (spec 0010 §5, spec 0011 §4): every byte outside 0x23–0x7E
    // gets an index above 90; `'` (0x27) maps to the index of `&`, so it is
    // forced to 0xFF. The batch keeps a running maximum of these and of the
    // pair values, and compares them with 90 and 8191 once.
    let checked = vorrq_u8(indices8, vceqq_u8(chars, vdupq_n_u8(0x27)));

    // -----------------------------------------------------------------------
    // Step 2: pair values. A 16-bit lane of the indices is lo + 256 hi;
    // lo + 91 hi = that - 165 hi. At most 255 + 91 * 255 = 23460, exact
    // even for bytes outside the alphabet.
    // -----------------------------------------------------------------------
    let pairs = vreinterpretq_u16_u8(indices8);
    let vals = vmlsq_n_u16(pairs, vshrq_n_u16(pairs, 8), 165);

    // -----------------------------------------------------------------------
    // Step 3: pack the eight 13-bit values into 13 bytes, as in `x86.rs`.
    // A shift-and-insert joins value pairs into 26-bit fields (field k =
    // v[2k] + v[2k+1] << 13, in 32-bit lane k); a shift by 2k puts field
    // k's byte b at output byte 3k + b. Bytes 3, 6 and 9 get a contribution
    // from two fields: a second table lookup brings the upper field's byte 3
    // and an OR merges it.
    // -----------------------------------------------------------------------
    let v32 = vreinterpretq_u32_u16(vals);
    let fields = vsliq_n_u32(v32, vshrq_n_u32(v32, 16), 13);
    let field_shift: [i32; 4] = [0, 2, 4, 6];
    let placed = vreinterpretq_u8_u32(vshlq_u32(fields, vld1q_s32(field_shift.as_ptr())));
    let primary: [u8; 16] = [0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, 15, 0xFF, 0xFF, 0xFF];
    let secondary: [u8; 16] = [
        0xFF, 0xFF, 0xFF, 3, 0xFF, 0xFF, 7, 0xFF, 0xFF, 11, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    // Stores 16 bytes, of which 13 are data: the caller provides 3 bytes of
    // slack (see `decode_lines_neon`).
    vst1q_u8(
        output,
        vorrq_u8(
            vqtbl1q_u8(placed, vld1q_u8(primary.as_ptr())),
            vqtbl1q_u8(placed, vld1q_u8(secondary.as_ptr())),
        ),
    );

    (checked, vals)
}

// ---------------------------------------------------------------------------
// Batch entry points, used by the driver (spec 0010 §3, §5)
// ---------------------------------------------------------------------------
//
// As in `x86.rs`: a batch is `lines` lines of `n` blocks, the encoded side
// of line `l` at `l · stride`, the raw side contiguous.

/// Encodes `lines` × `n` blocks of 13 bytes; line `l`'s characters go to
/// `output + l · stride`, followed by an LF when `TERMINATE`.
///
/// # Safety
/// `input` readable for `13 n · lines + 3` bytes (the kernel loads 16);
/// every line's `16 n` output bytes writable, and the byte after them when
/// `TERMINATE`.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub(crate) unsafe fn encode_lines_neon<const TERMINATE: bool>(
    input: *const u8,
    output: *mut u8,
    stride: usize,
    n: usize,
    lines: usize,
) {
    let (mut src, mut dst) = (input, output);
    for _ in 0..lines {
        for i in 0..n {
            encode_block_neon(src.add(13 * i), dst.add(16 * i));
        }
        if TERMINATE {
            *dst.add(16 * n) = b'\n';
        }
        src = src.add(13 * n);
        dst = dst.wrapping_add(stride);
    }
}

/// Decodes `lines` × `n` blocks of 16 characters, line `l` read from
/// `input + l · stride`, into contiguous output, and returns whether every
/// block was valid. The output is written even for invalid blocks.
///
/// # Safety
/// Every line's `16 n` input bytes readable; `output` writable for
/// `13 n · lines + 3` (the kernel stores 16).
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub(crate) unsafe fn decode_lines_neon(
    input: *const u8,
    stride: usize,
    output: *mut u8,
    n: usize,
    lines: usize,
) -> bool {
    let (mut max_index, mut max_value) = (vdupq_n_u8(0), vdupq_n_u16(0));
    for l in 0..lines {
        let (src, dst) = (input.add(stride * l), output.add(13 * n * l));
        for i in 0..n {
            let (checked, vals) = decode_block_neon(src.add(16 * i), dst.add(13 * i));
            max_index = vmaxq_u8(max_index, checked);
            max_value = vmaxq_u16(max_value, vals);
        }
    }
    vmaxvq_u8(max_index) <= 90 && vmaxvq_u16(max_value) <= 8191
}
