// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! x86_64 SIMD kernels for the fixed-width basE91 variant.
//!
//! Two kernels are provided:
//!
//! - **SSE4.1** (`encode_sse41` / `decode_sse41`): processes one 13-byte
//!   block per call using 128-bit XMM registers.
//! - **AVX2** (`encode_avx2` / `decode_avx2`): processes two 13-byte blocks
//!   per call by running the SSE4.1 kernel twice; the benefit is reduced
//!   loop overhead and better instruction scheduling (~1.8× SSE4.1).
//!
//! # Division by 91
//!
//! Integer division by 91 is replaced by a multiply-high trick.
//! For 13-bit values v ∈ [0, 8191]:
//!
//!   hi = (v * MAGIC91) >> (16 + SHIFT91)
//!   lo = v - hi * 91
//!
//! where MAGIC91 = 2881 and SHIFT91 = 2.
//! Verified exhaustively for all v in [0, 8191].
//!
//! # Alphabet gap correction
//!
//! The SIMD alphabet skips `'` (0x27). Encode: add 0x23 to each index,
//! then add 1 for indices ≥ 4 (bytes that would land at 0x27 or above).
//! In SIMD: `_mm_cmpgt_epi8(chars, 0x26)` gives 0xFF where correction
//! needed; subtracting that mask adds 1 (since 0 − 0xFF = 1 mod 256).
//!
//! # Bit extraction
//!
//! Group k occupies bits [k*13, k*13+12]. The 8 groups have byte starts
//! (0,1,3,4,6,8,9,11) and in-byte shifts (0,5,2,7,4,1,6,3).
//! Four groups (g1,g3,g4,g6) span 3 bytes, so a 16-bit extraction misses
//! the high bits.
//!
//! We use `pshufb` (`_mm_shuffle_epi8`) to load each group's 3 bytes
//! (plus a zero-padding byte) into a 32-bit lane, then apply
//! `_mm_srli_epi32` (one per distinct shift) and blend the results.
//! All constants are hardcoded (bit offsets are fully deterministic).
//!
//! - `shuf_lo`: groups 0–3 into 32-bit lanes 0–3 (shifts 0,5,2,7)
//! - `shuf_hi`: groups 4–7 into 32-bit lanes 0–3 (shifts 4,1,6,3)
//!
//! Four `_mm_srli_epi32` + three `_mm_blend_epi16` per half.

#![allow(non_snake_case)]

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

const MAGIC91: u16 = 2881;
const SHIFT91: i32 = 2;

// ---------------------------------------------------------------------------
// Shared encode/decode helpers (compile for SSE4.1; safe to call from AVX2)
// ---------------------------------------------------------------------------

/// Encode 13 input bytes into 16 SIMD-alphabet output characters.
///
/// # Safety
/// Requires SSE4.1 + SSSE3.
/// `input` must be readable for 16 bytes (caller pads if needed).
/// `output` must be writable for 16 bytes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
#[inline]
unsafe fn encode_block_sse41(input: *const u8, output: *mut u8) {
    // Load 16 bytes (13 used, caller provides ≥16 readable bytes).
    let data = _mm_loadu_si128(input as *const __m128i);

    // -----------------------------------------------------------------------
    // Step 1: Extract 8 × 13-bit groups using pshufb + srli_epi32 + blend.
    //
    // Groups and their source bytes / in-byte shifts:
    //   g0: bytes  0, 1, 2  shift 0
    //   g1: bytes  1, 2, 3  shift 5
    //   g2: bytes  3, 4, 5  shift 2
    //   g3: bytes  4, 5, 6  shift 7
    //   g4: bytes  6, 7, 8  shift 4
    //   g5: bytes  8, 9,10  shift 1
    //   g6: bytes  9,10,11  shift 6
    //   g7: bytes 11,12,13* shift 3  (*byte 13 is safe padding ≥ 0)
    //
    // shuf_lo places g0–g3 into 32-bit lanes 0–3 (one group per lane).
    // shuf_hi places g4–g7 into 32-bit lanes 0–3.
    // pshufb index 0x80 produces a zero byte.
    //
    // After shuffling, 4 distinct srli_epi32 produce all needed shifts.
    // _mm_blend_epi16 (blends 16-bit units) selects the right shifted lane.
    //   blend mask bit n: 0 = keep a, 1 = take b.
    //   Each 32-bit lane = two consecutive 16-bit slots.
    // -----------------------------------------------------------------------

    // _mm_set_epi8(b15,b14,...,b1,b0): b0 = result byte 0 (low), b15 = byte 15 (high).
    // pshufb: out[i] = (mask[i] & 0x80) ? 0 : data[mask[i] & 0x0F].
    // x86 is little-endian: within a 32-bit lane, byte 0 is the least-significant byte.
    // Each group needs its lowest-significance byte first so srli_epi32 shifts correctly.
    //   lane 0 (bytes 0-3 of result): data[0]=LSB, data[1], data[2], zero=MSB → g0
    //   lane 1 (bytes 4-7): data[1], data[2], data[3], zero → g1
    //   lane 2 (bytes 8-11): data[3], data[4], data[5], zero → g2
    //   lane 3 (bytes 12-15): data[4], data[5], data[6], zero → g3
    let shuf_lo = _mm_set_epi8(
        -128i8, 6, 5, 4, // lane 3 bytes [15,14,13,12] = g3: zero,data6,data5,data4
        -128i8, 5, 4, 3, // lane 2 bytes [11,10, 9, 8] = g2: zero,data5,data4,data3
        -128i8, 3, 2, 1, // lane 1 bytes [ 7, 6, 5, 4] = g1: zero,data3,data2,data1
        -128i8, 2, 1, 0, // lane 0 bytes [ 3, 2, 1, 0] = g0: zero,data2,data1,data0
    );
    let shuf_hi = _mm_set_epi8(
        -128i8, -128i8, 12, 11, // lane 3 bytes [15,14,13,12] = g7: zero,zero,data12,data11
        -128i8, 11, 10, 9, // lane 2 bytes [11,10, 9, 8] = g6: zero,data11,data10,data9
        -128i8, 10, 9, 8, // lane 1 bytes [ 7, 6, 5, 4] = g5: zero,data10,data9,data8
        -128i8, 8, 7, 6, // lane 0 bytes [ 3, 2, 1, 0] = g4: zero,data8,data7,data6
    );
    let slo = _mm_shuffle_epi8(data, shuf_lo);
    let shi = _mm_shuffle_epi8(data, shuf_hi);

    // Shift each 32-bit lane right by the group's in-byte bit offset.
    // Groups 0–3 need shifts 0, 5, 2, 7 respectively.
    // Groups 4–7 need shifts 4, 1, 6, 3 respectively.
    //
    // We produce 4 fully-shifted versions and blend to pick each lane.
    // blend mask for _mm_blend_epi16: 2 bits per 32-bit lane (lo then hi 16-bit slot).
    //   lane 0 = bits [1:0], lane 1 = bits [3:2], lane 2 = bits [5:4], lane 3 = bits [7:6].
    //
    // For slo (shifts 0,5,2,7): start with shift-0 for lane 0, then layer others.
    // Four shifted copies; blend picks the right one per lane.
    // _mm_blend_epi16 mask: 2 bits per 32-bit lane (bits[1:0]=lane0 … bits[7:6]=lane3).
    let slo0 = slo;
    let slo5 = _mm_srli_epi32(slo, 5);
    let slo2 = _mm_srli_epi32(slo, 2);
    let slo7 = _mm_srli_epi32(slo, 7);
    let slo_a = _mm_blend_epi16(slo0, slo5, 0x0C); // lane 1 from slo5
    let slo_b = _mm_blend_epi16(slo_a, slo2, 0x30); // lane 2 from slo2
    let lo_vals = _mm_blend_epi16(slo_b, slo7, 0xC0); // lane 3 from slo7

    // For shi (shifts 4,1,6,3):
    let shi4 = _mm_srli_epi32(shi, 4);
    let shi1 = _mm_srli_epi32(shi, 1);
    let shi6 = _mm_srli_epi32(shi, 6);
    let shi3 = _mm_srli_epi32(shi, 3);
    let shi_a = _mm_blend_epi16(shi4, shi1, 0x0C);
    let shi_b = _mm_blend_epi16(shi_a, shi6, 0x30);
    let hi_vals = _mm_blend_epi16(shi_b, shi3, 0xC0);

    // Mask each 32-bit lane to 13 bits, then narrow to u16.
    let mask13 = _mm_set1_epi32(0x1FFF);
    let lo_masked = _mm_and_si128(lo_vals, mask13);
    let hi_masked = _mm_and_si128(hi_vals, mask13);

    // Pack 32→16: _mm_packs_epi32 saturates signed; values 0–8191 are fine.
    let vals = _mm_packs_epi32(lo_masked, hi_masked);
    // vals = [g0,g1,g2,g3, g4,g5,g6,g7] as u16x8.

    // -----------------------------------------------------------------------
    // Step 2: Divide each 13-bit value by 91.
    //   hi = mulhi_u16(vals, MAGIC91) >> SHIFT91
    //   lo = vals - hi * 91
    // -----------------------------------------------------------------------
    let magic = _mm_set1_epi16(MAGIC91 as i16);
    let hi = _mm_srli_epi16(_mm_mulhi_epu16(vals, magic), SHIFT91);
    let lo = _mm_sub_epi16(vals, _mm_mullo_epi16(hi, _mm_set1_epi16(91)));

    // -----------------------------------------------------------------------
    // Step 3: Interleave lo/hi into output byte order [lo0,hi0,lo1,hi1,...].
    // -----------------------------------------------------------------------
    // Pack u16→u8 (values 0–90 are in range).
    let lo8 = _mm_packus_epi16(lo, lo); // lo0..lo7 in bytes 0..7 (duplicated)
    let hi8 = _mm_packus_epi16(hi, hi);
    let interleaved = _mm_unpacklo_epi8(lo8, hi8); // [lo0,hi0,lo1,hi1,...]

    // -----------------------------------------------------------------------
    // Step 4: Map indices 0–90 to SIMD-alphabet characters.
    //   indices 0–3   → 0x23 + index
    //   indices 4–90  → 0x24 + index  (skip 0x27 = '\'')
    // Add 0x23 to all, then subtract the cmpgt mask (0xFF where > 0x26)
    // which is equivalent to adding 1 for those positions.
    // -----------------------------------------------------------------------
    let base = _mm_set1_epi8(0x23u8 as i8);
    let chars = _mm_add_epi8(interleaved, base);
    let threshold = _mm_set1_epi8(0x26u8 as i8);
    let needs_bump = _mm_cmpgt_epi8(chars, threshold);
    let corrected = _mm_sub_epi8(chars, needs_bump);

    _mm_storeu_si128(output as *mut __m128i, corrected);
}

/// Decode 16 SIMD-alphabet characters into 13 output bytes.
///
/// Returns what the batch validates (spec 0011 §4): the indices, `'` forced
/// to 0xFF, and the pair values. The output is written either way.
///
/// # Safety
/// Requires SSE4.1 + SSSE3.
/// `input` readable for 16 bytes; `output` writable for 13.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
#[inline]
unsafe fn decode_block_sse41(input: *const u8, output: *mut u8) -> (__m128i, __m128i) {
    let chars = _mm_loadu_si128(input as *const __m128i);

    // -----------------------------------------------------------------------
    // Step 1: Reverse map: characters → indices 0–90.
    //   chars 0x23–0x26: index = char - 0x23
    //   chars 0x28–0x7E: index = char - 0x24
    // Equivalent: index = (char - 0x23) - bump, where bump=1 if char > 0x26.
    // _mm_cmpgt_epi8 returns 0xFF (= -1) where true, 0 elsewhere.
    // Subtracting 0xFF = adding 1, so we need to ADD the mask (not subtract).
    // But 0xFF = -1 in i8, so adding 0xFF subtracts 1. That's what we want!
    // index = (char - 0x23) + needs_bump  where needs_bump = 0xFF = -1 for bump lanes.
    // -----------------------------------------------------------------------
    let threshold = _mm_set1_epi8(0x26u8 as i8);
    let needs_bump = _mm_cmpgt_epi8(chars, threshold); // 0xFF (-1) where > 0x26
    let raw = _mm_sub_epi8(chars, _mm_set1_epi8(0x23u8 as i8));
    let indices8 = _mm_add_epi8(raw, needs_bump); // subtracts 1 where > 0x26

    // Validity (spec 0010 §5, spec 0011 §4): every byte outside 0x23–0x7E
    // gets an index above 90 as an *unsigned* byte (the subtraction wraps);
    // `'` (0x27) maps to the index of `&`, so it is forced to 0xFF. The
    // batch keeps a running maximum of these and of the pair values, and
    // compares them with 90 and 8191 once.
    let checked = _mm_or_si128(indices8, _mm_cmpeq_epi8(chars, _mm_set1_epi8(0x27)));

    // -----------------------------------------------------------------------
    // Step 2: Deinterleave and reconstruct 13-bit values.
    //   indices8 = [lo0,hi0,lo1,hi1,...,lo7,hi7]
    //   val[i] = lo[i] + hi[i] * 91
    // -----------------------------------------------------------------------
    // Separate lo (even) and hi (odd) bytes into 8 u16 lanes each.
    // val = lo + 91 * hi for each pair, in one multiply-add: unsigned bytes
    // times signed weights (1, 91), adjacent products summed into 16 bits.
    // At most 255 + 91 * 255 = 23460, so nothing saturates, even for bytes
    // outside the alphabet.
    let vals = _mm_maddubs_epi16(indices8, _mm_set1_epi16(91 << 8 | 1));

    // -----------------------------------------------------------------------
    // Step 3: pack the eight 13-bit values into 13 bytes (spec 0011).
    //
    // `pmaddwd` joins value pairs into 26-bit fields: field k = v[2k] +
    // v[2k+1] << 13, in 32-bit lane k. Field k starts at bit 26k of the
    // output; shifted left by 0, 2, 4, 6 bits (26k mod 8, as a multiply:
    // SSE4.1 has no per-lane shift), its byte b is output byte 3k + b.
    // Bytes 3, 6 and 9 get a contribution from two fields: a second
    // shuffle brings the upper field's byte 3 and an OR merges it.
    // -----------------------------------------------------------------------
    let fields = _mm_madd_epi16(vals, _mm_set1_epi32(8192 << 16 | 1));
    let placed = _mm_mullo_epi32(fields, _mm_set_epi32(64, 16, 4, 1));
    let primary = _mm_shuffle_epi8(
        placed,
        _mm_set_epi8(-1, -1, -1, 15, 14, 13, 12, 10, 9, 8, 6, 5, 4, 2, 1, 0),
    );
    let secondary = _mm_shuffle_epi8(
        placed,
        _mm_set_epi8(-1, -1, -1, -1, -1, -1, 11, -1, -1, 7, -1, -1, 3, -1, -1, -1),
    );
    // Stores 16 bytes, of which 13 are data: the caller provides 3 bytes of
    // slack (see `decode_lines_sse41`).
    _mm_storeu_si128(output as *mut __m128i, _mm_or_si128(primary, secondary));

    (checked, vals)
}

// ---------------------------------------------------------------------------
// AVX2 kernel — two blocks (26 bytes → 32 chars) per call
// ---------------------------------------------------------------------------
//
// The 256-bit registers hold two independent 13-byte blocks in their low and
// high 128-bit lanes respectively.  _mm256_shuffle_epi8, _mm256_srli_epi32,
// _mm256_blend_epi16, _mm256_and_si256, _mm256_packs_epi32,
// _mm256_mulhi_epu16, _mm256_mullo_epi16, _mm256_packus_epi16,
// _mm256_unpacklo_epi8, _mm256_add_epi8, _mm256_cmpgt_epi8, _mm256_sub_epi8
// all operate independently on each 128-bit lane — no cross-lane interaction.
//
// The shuffle constants are the SSE4.1 shuf_lo/shuf_hi patterns repeated in
// both 128-bit lanes of the 256-bit register.

/// Encode 26 input bytes into 32 SIMD-alphabet output characters.
///
/// # Safety
/// Requires AVX2 + SSSE3. `input` readable for 32 bytes (padded); `output` for 32.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,ssse3")]
#[inline]
unsafe fn encode_block_avx2(input: *const u8, output: *mut u8) {
    // Load the two 13-byte blocks into the low and high 128-bit lanes
    // independently.  A single 256-bit load would place block1 at an offset
    // of 13 bytes into the high lane, misaligning it relative to the shuffle
    // constants.  Instead we load block0 at input[0..15] into the low lane
    // and block1 at input[13..28] into the high lane, so both lanes have
    // their respective block starting at byte 0 of the lane.
    let lo = _mm_loadu_si128(input as *const __m128i);
    let hi = _mm_loadu_si128(input.add(13) as *const __m128i);
    let data = _mm256_set_m128i(hi, lo);

    // Step 1: Extract 16 × 13-bit groups (8 per 128-bit lane) using
    // pshufb + srli_epi32 + blend — same logic as SSE4.1, applied to both
    // lanes simultaneously.
    //
    // shuf_lo256: same as SSE4.1 shuf_lo, repeated in both lanes.
    //   lane byte layout (each 128-bit lane): g0–g3, one group per 32-bit slot.
    // _mm256_set_epi8 args: byte 31 (high) down to byte 0 (low).
    let shuf_lo256 = _mm256_set_epi8(
        // high lane (block 1, bytes 13–25 of input → positions 16–28 in the
        // loaded 256-bit register; same relative offsets as low lane)
        -128i8, 6, 5, 4, // lane3: g3 bytes 4,5,6 + zero
        -128i8, 5, 4, 3, // lane2: g2 bytes 3,4,5 + zero
        -128i8, 3, 2, 1, // lane1: g1 bytes 1,2,3 + zero
        -128i8, 2, 1, 0, // lane0: g0 bytes 0,1,2 + zero
        // low lane (block 0, bytes 0–12)
        -128i8, 6, 5, 4, -128i8, 5, 4, 3, -128i8, 3, 2, 1, -128i8, 2, 1, 0,
    );
    let shuf_hi256 = _mm256_set_epi8(
        // high lane
        -128i8, -128i8, 12, 11, // lane3: g7 bytes 11,12 + zero,zero
        -128i8, 11, 10, 9, // lane2: g6 bytes 9,10,11 + zero
        -128i8, 10, 9, 8, // lane1: g5 bytes 8,9,10 + zero
        -128i8, 8, 7, 6, // lane0: g4 bytes 6,7,8 + zero
        // low lane
        -128i8, -128i8, 12, 11, -128i8, 11, 10, 9, -128i8, 10, 9, 8, -128i8, 8, 7, 6,
    );
    let slo = _mm256_shuffle_epi8(data, shuf_lo256);
    let shi = _mm256_shuffle_epi8(data, shuf_hi256);

    // Four shifted copies + blend, per the SSE4.1 scheme.
    let slo0 = slo;
    let slo5 = _mm256_srli_epi32(slo, 5);
    let slo2 = _mm256_srli_epi32(slo, 2);
    let slo7 = _mm256_srli_epi32(slo, 7);
    let slo_a = _mm256_blend_epi16(slo0, slo5, 0x0C);
    let slo_b = _mm256_blend_epi16(slo_a, slo2, 0x30);
    let lo_vals = _mm256_blend_epi16(slo_b, slo7, 0xC0);

    let shi4 = _mm256_srli_epi32(shi, 4);
    let shi1 = _mm256_srli_epi32(shi, 1);
    let shi6 = _mm256_srli_epi32(shi, 6);
    let shi3 = _mm256_srli_epi32(shi, 3);
    let shi_a = _mm256_blend_epi16(shi4, shi1, 0x0C);
    let shi_b = _mm256_blend_epi16(shi_a, shi6, 0x30);
    let hi_vals = _mm256_blend_epi16(shi_b, shi3, 0xC0);

    let mask13 = _mm256_set1_epi32(0x1FFF);
    let lo_masked = _mm256_and_si256(lo_vals, mask13);
    let hi_masked = _mm256_and_si256(hi_vals, mask13);

    // Pack 32→16 within each 128-bit lane.
    let vals = _mm256_packs_epi32(lo_masked, hi_masked);

    // Step 2: Divide by 91.
    let magic = _mm256_set1_epi16(MAGIC91 as i16);
    let hi = _mm256_srli_epi16(_mm256_mulhi_epu16(vals, magic), SHIFT91);
    let lo = _mm256_sub_epi16(vals, _mm256_mullo_epi16(hi, _mm256_set1_epi16(91)));

    // Step 3: Interleave lo/hi → [lo0,hi0,lo1,hi1,...].
    let lo8 = _mm256_packus_epi16(lo, lo);
    let hi8 = _mm256_packus_epi16(hi, hi);
    let interleaved = _mm256_unpacklo_epi8(lo8, hi8);

    // Step 4: Map to SIMD-alphabet.
    let base = _mm256_set1_epi8(0x23u8 as i8);
    let chars = _mm256_add_epi8(interleaved, base);
    let threshold = _mm256_set1_epi8(0x26u8 as i8);
    let needs_bump = _mm256_cmpgt_epi8(chars, threshold);
    let corrected = _mm256_sub_epi8(chars, needs_bump);

    _mm256_storeu_si256(output as *mut __m256i, corrected);
}

/// Decode 32 SIMD-alphabet characters into 26 output bytes (two blocks).
///
/// Returns what the batch validates, as `decode_block_sse41`. The output is
/// written either way.
///
/// # Safety
/// Requires AVX2. `input` readable for 32 bytes; `output` writable for 26.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,ssse3")]
#[inline]
unsafe fn decode_block_avx2(input: *const u8, output: *mut u8) -> (__m256i, __m256i) {
    let chars = _mm256_loadu_si256(input as *const __m256i);

    // Step 1: Reverse map characters → indices 0–90.
    let threshold = _mm256_set1_epi8(0x26u8 as i8);
    let needs_bump = _mm256_cmpgt_epi8(chars, threshold);
    let raw = _mm256_sub_epi8(chars, _mm256_set1_epi8(0x23u8 as i8));
    let indices8 = _mm256_add_epi8(raw, needs_bump);

    // Validity: as in `decode_block_sse41`.
    let checked = _mm256_or_si256(indices8, _mm256_cmpeq_epi8(chars, _mm256_set1_epi8(0x27)));

    // Step 2: Deinterleave and reconstruct 13-bit values.
    // val = lo + 91 * hi, as in `decode_block_sse41` (each 128-bit lane).
    let vals = _mm256_maddubs_epi16(indices8, _mm256_set1_epi16(91 << 8 | 1));

    // Step 3: pack each lane's eight 13-bit values into 13 bytes, as in
    // `decode_block_sse41`, with a real per-lane shift (`vpsllvd`) instead
    // of the multiply. The shuffles work lane by lane.
    let fields = _mm256_madd_epi16(vals, _mm256_set1_epi32(8192 << 16 | 1));
    let placed = _mm256_sllv_epi32(fields, _mm256_set_epi32(6, 4, 2, 0, 6, 4, 2, 0));
    let primary = _mm256_shuffle_epi8(
        placed,
        _mm256_set_epi8(
            -1, -1, -1, 15, 14, 13, 12, 10, 9, 8, 6, 5, 4, 2, 1, 0, // high lane
            -1, -1, -1, 15, 14, 13, 12, 10, 9, 8, 6, 5, 4, 2, 1, 0, // low lane
        ),
    );
    let secondary = _mm256_shuffle_epi8(
        placed,
        _mm256_set_epi8(
            -1, -1, -1, -1, -1, -1, 11, -1, -1, 7, -1, -1, 3, -1, -1, -1, // high lane
            -1, -1, -1, -1, -1, -1, 11, -1, -1, 7, -1, -1, 3, -1, -1, -1, // low lane
        ),
    );
    let packed = _mm256_or_si256(primary, secondary);
    // Two 16-byte stores of 13 data bytes each: the second one ends 3 bytes
    // past the unit's 26 (see `decode_lines_avx2`).
    _mm_storeu_si128(output as *mut __m128i, _mm256_castsi256_si128(packed));
    _mm_storeu_si128(
        output.add(13) as *mut __m128i,
        _mm256_extracti128_si256(packed, 1),
    );

    (checked, vals)
}

// ---------------------------------------------------------------------------
// Batch entry points, used by the driver (spec 0010 §3, §5)
// ---------------------------------------------------------------------------
//
// A batch is `lines` lines of `n` blocks (or AVX2 units) each. The encoded
// side of a line starts every `stride` bytes (the line and its terminator);
// the raw side is contiguous. One call per batch keeps the per-line cost to
// the loop itself, and the decoders test their validity once per batch.

/// Encodes `lines` × `n` blocks of 13 bytes; line `l`'s characters go to
/// `output + l · stride`.
///
/// # Safety
/// Requires SSE4.1 + SSSE3. `input` readable for `13 n · lines + 3` bytes
/// (the kernel loads 16); every line's `16 n` output bytes writable.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
pub(crate) unsafe fn encode_lines_sse41(
    input: *const u8,
    output: *mut u8,
    stride: usize,
    n: usize,
    lines: usize,
) {
    for l in 0..lines {
        let (src, dst) = (input.add(13 * n * l), output.add(stride * l));
        for i in 0..n {
            encode_block_sse41(src.add(13 * i), dst.add(16 * i));
        }
    }
}

/// As [`encode_lines_sse41`], with `n` AVX2 units of 26 bytes per line.
///
/// # Safety
/// Requires AVX2 + SSSE3. `input` readable for `26 n · lines + 3` bytes;
/// every line's `32 n` output bytes writable.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,ssse3")]
pub(crate) unsafe fn encode_lines_avx2(
    input: *const u8,
    output: *mut u8,
    stride: usize,
    n: usize,
    lines: usize,
) {
    for l in 0..lines {
        let (src, dst) = (input.add(26 * n * l), output.add(stride * l));
        for i in 0..n {
            encode_block_avx2(src.add(26 * i), dst.add(32 * i));
        }
    }
}

/// Whether a batch was valid, from the running maxima of its indices (at
/// most 90) and pair values (at most 8191).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
#[inline]
unsafe fn valid_sse41(max_index: __m128i, max_value: __m128i) -> bool {
    let over = _mm_or_si128(
        _mm_subs_epu8(max_index, _mm_set1_epi8(90)),
        _mm_srli_epi16(max_value, 13),
    );
    _mm_testz_si128(over, over) != 0
}

/// Decodes `lines` × `n` blocks of 16 characters, line `l` read from
/// `input + l · stride`, into contiguous output, and returns whether every
/// block was valid. The output is written even for invalid blocks.
///
/// # Safety
/// Requires SSE4.1 + SSSE3. Every line's `16 n` input bytes readable;
/// `output` writable for `13 n · lines + 3` (the kernel stores 16).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
pub(crate) unsafe fn decode_lines_sse41(
    input: *const u8,
    stride: usize,
    output: *mut u8,
    n: usize,
    lines: usize,
) -> bool {
    // Running maximums over the whole batch, one validity test at the end.
    // Nested loops: chosen by measurement, the flat loop of
    // `decode_lines_avx2` being 7–19 % slower here (spec 0011, stage 3).
    let (mut max_index, mut max_value) = (_mm_setzero_si128(), _mm_setzero_si128());
    for l in 0..lines {
        let (src, dst) = (input.add(stride * l), output.add(13 * n * l));
        for i in 0..n {
            let (checked, vals) = decode_block_sse41(src.add(16 * i), dst.add(13 * i));
            max_index = _mm_max_epu8(max_index, checked);
            max_value = _mm_max_epu16(max_value, vals);
        }
    }
    valid_sse41(max_index, max_value)
}

/// As [`decode_lines_sse41`], with `n` AVX2 units of 32 characters per line.
///
/// # Safety
/// Requires AVX2 + SSSE3. Every line's `32 n` input bytes readable;
/// `output` writable for `26 n · lines + 3`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,ssse3")]
pub(crate) unsafe fn decode_lines_avx2(
    input: *const u8,
    stride: usize,
    output: *mut u8,
    n: usize,
    lines: usize,
) -> bool {
    // One flat loop, skipping each terminator at the end of a line, and
    // running maximums over the whole batch. Chosen by measurement with the
    // former kernel (nested loops up to 24 % slower on an efficiency core);
    // SSE4.1 measures the other way (`decode_lines_sse41`). See spec 0011,
    // "Code-generation sensitivity"; to be re-checked for G3.
    let skip = stride.wrapping_sub(32 * n);
    let (mut src, mut dst) = (input, output);
    let mut left = n;
    let (mut max_index, mut max_value) = (_mm256_setzero_si256(), _mm256_setzero_si256());
    for _ in 0..n * lines {
        let (checked, vals) = decode_block_avx2(src, dst);
        max_index = _mm256_max_epu8(max_index, checked);
        max_value = _mm256_max_epu16(max_value, vals);
        src = src.add(32);
        dst = dst.add(26);
        left -= 1;
        if left == 0 {
            src = src.wrapping_add(skip);
            left = n;
        }
    }
    valid_sse41(
        _mm_max_epu8(
            _mm256_castsi256_si128(max_index),
            _mm256_extracti128_si256(max_index, 1),
        ),
        _mm_max_epu16(
            _mm256_castsi256_si128(max_value),
            _mm256_extracti128_si256(max_value, 1),
        ),
    )
}
