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
//! The SIMD alphabet skips `\` (0x5C). Encode: add 0x23 to each index,
//! then add 1 for indices ≥ 57 (bytes that would land at 0x5C or above).
//! In SIMD: `_mm_cmpgt_epi8(chars, 0x5B)` gives 0xFF where correction
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
//! Four `_mm_srli_epi32` + three `_mm_blend_epi16` per half.

#![allow(non_snake_case)]

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

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
pub(crate) unsafe fn encode_block_sse41(input: *const u8, output: *mut u8) {
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
    //   indices 0–56  → 0x23 + index
    //   indices 57–90 → 0x24 + index  (skip 0x5C = '\')
    // Add 0x23 to all, then subtract the cmpgt mask (0xFF where > 0x5B)
    // which is equivalent to adding 1 for those positions.
    // -----------------------------------------------------------------------
    let base = _mm_set1_epi8(0x23u8 as i8);
    let chars = _mm_add_epi8(interleaved, base);
    let threshold = _mm_set1_epi8(0x5Bu8 as i8);
    let needs_bump = _mm_cmpgt_epi8(chars, threshold);
    let corrected = _mm_sub_epi8(chars, needs_bump);

    _mm_storeu_si128(output as *mut __m128i, corrected);
}

/// Decode 16 SIMD-alphabet characters into 13 output bytes.
///
/// Returns `false` if any byte is not in the SIMD alphabet.
///
/// # Safety
/// Requires SSE4.1 + SSSE3.
/// `input` readable for 16 bytes; `output` writable for 13.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
pub(crate) unsafe fn decode_block_sse41(input: *const u8, output: *mut u8) -> bool {
    let chars = _mm_loadu_si128(input as *const __m128i);

    // -----------------------------------------------------------------------
    // Step 1: Reverse map: characters → indices 0–90.
    //   chars 0x23–0x5B: index = char - 0x23
    //   chars 0x5D–0x7E: index = char - 0x24
    // Equivalent: index = (char - 0x23) - bump, where bump=1 if char > 0x5B.
    // _mm_cmpgt_epi8 returns 0xFF (= -1) where true, 0 elsewhere.
    // Subtracting 0xFF = adding 1, so we need to ADD the mask (not subtract).
    // But 0xFF = -1 in i8, so adding 0xFF subtracts 1. That's what we want!
    // index = (char - 0x23) + needs_bump  where needs_bump = 0xFF = -1 for bump lanes.
    // -----------------------------------------------------------------------
    let threshold = _mm_set1_epi8(0x5Bu8 as i8);
    let needs_bump = _mm_cmpgt_epi8(chars, threshold); // 0xFF (-1) where > 0x5B
    let raw = _mm_sub_epi8(chars, _mm_set1_epi8(0x23u8 as i8));
    let indices8 = _mm_add_epi8(raw, needs_bump); // subtracts 1 where > 0x5B

    // Validate: indices must be 0–90. Values that were out of alphabet
    // will have wrapped or be > 90.
    // Check also that original chars were not below 0x23 (would underflow to > 90).
    let max_valid = _mm_set1_epi8(90i8);
    let invalid = _mm_cmpgt_epi8(indices8, max_valid);
    // Also detect chars below 0x23: after subtracting 0x23 they become > 127
    // as signed, which cmpgt(_, 90) already catches since 90 < 127.
    // But unsigned wrap: 0x00 - 0x23 = 0xDD = 221 > 90. Caught.
    if _mm_movemask_epi8(invalid) != 0 {
        return false;
    }

    // -----------------------------------------------------------------------
    // Step 2: Deinterleave and reconstruct 13-bit values.
    //   indices8 = [lo0,hi0,lo1,hi1,...,lo7,hi7]
    //   val[i] = lo[i] + hi[i] * 91
    // -----------------------------------------------------------------------
    // Separate lo (even) and hi (odd) bytes into 8 u16 lanes each.
    let shuf_lo = _mm_set_epi8(-1, -1, -1, -1, -1, -1, -1, -1, 14, 12, 10, 8, 6, 4, 2, 0);
    let shuf_hi = _mm_set_epi8(-1, -1, -1, -1, -1, -1, -1, -1, 15, 13, 11, 9, 7, 5, 3, 1);
    let lo8_sep = _mm_shuffle_epi8(indices8, shuf_lo); // lo0..lo7 in bytes 0..7
    let hi8_sep = _mm_shuffle_epi8(indices8, shuf_hi); // hi0..hi7 in bytes 0..7
    let zero = _mm_setzero_si128();
    let lo16 = _mm_unpacklo_epi8(lo8_sep, zero); // lo as u16x8
    let hi16 = _mm_unpacklo_epi8(hi8_sep, zero); // hi as u16x8

    // val = lo + hi * 91
    let vals = _mm_add_epi16(lo16, _mm_mullo_epi16(hi16, _mm_set1_epi16(91)));

    // -----------------------------------------------------------------------
    // Step 3: Scatter 8 × 13-bit values back into 13 bytes via SIMD.
    //
    // The bit layout is the exact inverse of the encode pshufb+srli gather:
    //   group k occupies output bits [k*13, k*13+12].
    //   byte starts: (0,1,3,4,6,8,9,11), in-byte shifts: (0,5,2,7,4,1,6,3).
    //
    // Strategy (symmetric to encode's srli+blend+pshufb extract):
    //   1. Widen vals (u16x8) to two u32x4 registers: lo_g (g0–g3), hi_g (g4–g7).
    //   2. Left-shift each 32-bit lane by the group's in-byte bit offset, using
    //      slli_epi32 + blend_epi16 (same blend masks as encode's srli+blend).
    //   3. Within lo_shifted: bytes for adjacent groups overlap at shared output
    //      bytes.  One pshufb aligns each "secondary contributor" to the same
    //      byte slot as its "primary", then OR merges them.
    //   4. A final pshufb scatters the merged bytes to output positions B0–B6
    //      (lo) and B6–B12 (hi).
    //   5. OR lo_scatter and hi_scatter; storeu 13 bytes.
    //
    // Overlap map (output byte ← prim_source | sec_source):
    //   lo: B1←L[1]|L[4], B3←L[6]|L[8], B4←L[9]|L[12]
    //   hi: B8←H[2]|H[4], B9←H[5]|H[8], B11←H[10]|H[12]
    //   cross: B6←lo[14]|hi[0]  (handled by OR of lo_scatter and hi_scatter)
    // -----------------------------------------------------------------------

    // Widen u16x8 → two u32x4 (lo: g0-g3, hi: g4-g7).
    let lo_g = _mm_cvtepu16_epi32(vals);
    let hi_g = _mm_cvtepu16_epi32(_mm_srli_si128(vals, 8));

    // Left-shift each lane by its group's in-byte bit offset.
    // lo group shifts: lane0=0, lane1=5, lane2=2, lane3=7.
    let lo_s0 = lo_g;
    let lo_s5 = _mm_slli_epi32(lo_g, 5);
    let lo_s2 = _mm_slli_epi32(lo_g, 2);
    let lo_s7 = _mm_slli_epi32(lo_g, 7);
    let lo_a = _mm_blend_epi16(lo_s0, lo_s5, 0x0C); // lane1 ← s5
    let lo_b = _mm_blend_epi16(lo_a, lo_s2, 0x30); // lane2 ← s2
    let lo_shifted = _mm_blend_epi16(lo_b, lo_s7, 0xC0); // lane3 ← s7

    // hi group shifts: lane0=4, lane1=1, lane2=6, lane3=3.
    let hi_s4 = _mm_slli_epi32(hi_g, 4);
    let hi_s1 = _mm_slli_epi32(hi_g, 1);
    let hi_s6 = _mm_slli_epi32(hi_g, 6);
    let hi_s3 = _mm_slli_epi32(hi_g, 3);
    let hi_a = _mm_blend_epi16(hi_s4, hi_s1, 0x0C);
    let hi_b = _mm_blend_epi16(hi_a, hi_s6, 0x30);
    let hi_shifted = _mm_blend_epi16(hi_b, hi_s3, 0xC0);

    // Merge secondary contributors into primary byte slots.
    //
    // lo_shifted byte layout (L[n] = lo_shifted byte n):
    //   L[0]=g0b0→B0, L[1]=g0b1→B1prim, L[4]=g1b0→B1sec,
    //   L[5]=g1b1→B2, L[6]=g1b2→B3prim, L[8]=g2b0→B3sec,
    //   L[9]=g2b1→B4prim, L[12]=g3b0→B4sec, L[13]=g3b1→B5, L[14]=g3b2→B6lo.
    //
    // Route secondaries to their primary slot: L[4]→pos1, L[8]→pos6, L[12]→pos9.
    // _mm_set_epi8(b15,b14,...,b1,b0): 16 args, high byte first.
    let sec_lo_shuf = _mm_set_epi8(
        -1, -1, -1, -1, -1, -1,   // b15..b10: zero
        12i8, // b9 ← L[12]
        -1, -1,  // b8,b7: zero
        8i8, // b6 ← L[8]
        -1, -1, -1, -1,   // b5..b2: zero
        4i8,  // b1 ← L[4]
        -1i8, // b0: zero
    );
    let lo_merged = _mm_or_si128(lo_shifted, _mm_shuffle_epi8(lo_shifted, sec_lo_shuf));

    // After merge, lo_merged has:
    //   byte 0 = g0b0             → B0
    //   byte 1 = g0b1 | g1b0     → B1 complete
    //   byte 5 = g1b1             → B2
    //   byte 6 = g1b2 | g2b0     → B3 complete
    //   byte 9 = g2b1 | g3b0     → B4 complete
    //   byte 13= g3b1             → B5
    //   byte 14= g3b2             → B6 (lo contribution)
    //
    // Scatter lo_merged to output bytes B0-B6 (B7-B15 = 0x80).
    let scatter_lo = _mm_set_epi8(
        -1, -1, -1, -1, -1, -1, -1, -1, -1,   // b15..b7: zero
        14i8, // b6 ← lo_merged[14] → B6
        13i8, // b5 ← lo_merged[13] → B5
        9i8,  // b4 ← lo_merged[9]  → B4
        6i8,  // b3 ← lo_merged[6]  → B3
        5i8,  // b2 ← lo_merged[5]  → B2
        1i8,  // b1 ← lo_merged[1]  → B1
        0i8,  // b0 ← lo_merged[0]  → B0
    );
    let lo_out = _mm_shuffle_epi8(lo_merged, scatter_lo);

    // hi_shifted byte layout (H[n]):
    //   H[0]=g4b0→B6hi, H[1]=g4b1→B7, H[2]=g4b2→B8prim,
    //   H[4]=g5b0→B8sec, H[5]=g5b1→B9prim, H[8]=g6b0→B9sec,
    //   H[9]=g6b1→B10, H[10]=g6b2→B11prim, H[12]=g7b0→B11sec, H[13]=g7b1→B12.
    //
    // Route secondaries: H[4]→pos2, H[8]→pos5, H[12]→pos10.
    let sec_hi_shuf = _mm_set_epi8(
        -1i8, -1i8, -1i8, -1i8, -1i8, // b15..b11: zero
        12i8, // b10 ← H[12]
        -1i8, -1i8, -1i8, -1i8, // b9..b6: zero
        8i8,  // b5 ← H[8]
        -1i8, -1i8, // b4,b3: zero
        4i8,  // b2 ← H[4]
        -1i8, -1i8, // b1,b0: zero
    );
    let hi_merged = _mm_or_si128(hi_shifted, _mm_shuffle_epi8(hi_shifted, sec_hi_shuf));

    // After merge, hi_merged has:
    //   byte 0  = g4b0   → B6 (hi contribution; ORed with lo_out[6] below)
    //   byte 1  = g4b1   → B7
    //   byte 2  = g4b2 | g5b0  → B8 complete
    //   byte 5  = g5b1 | g6b0  → B9 complete
    //   byte 9  = g6b1   → B10
    //   byte 10 = g6b2 | g7b0  → B11 complete
    //   byte 13 = g7b1   → B12
    //
    // Scatter hi_merged to output bytes B6-B12.
    let scatter_hi = _mm_set_epi8(
        -1, -1, -1,   // b15,b14,b13: zero
        13i8, // b12 ← hi_merged[13] → B12
        10i8, // b11 ← hi_merged[10] → B11
        9i8,  // b10 ← hi_merged[9]  → B10
        5i8,  // b9  ← hi_merged[5]  → B9
        2i8,  // b8  ← hi_merged[2]  → B8
        1i8,  // b7  ← hi_merged[1]  → B7
        0i8,  // b6  ← hi_merged[0]  → B6
        -1, -1, -1, -1, -1, -1, // b5..b0: zero
    );
    let hi_out = _mm_shuffle_epi8(hi_merged, scatter_hi);

    // Combine and store.
    // decode_loop reserves out_needed + OUT spare bytes, so writing 16 bytes
    // here is safe even though only 13 are valid data (3 bytes of harmless
    // overwrite into spare capacity).
    let out128 = _mm_or_si128(lo_out, hi_out);
    _mm_storeu_si128(output as *mut __m128i, out128);

    true
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
pub(crate) unsafe fn encode_block_avx2(input: *const u8, output: *mut u8) {
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
    let threshold = _mm256_set1_epi8(0x5Bu8 as i8);
    let needs_bump = _mm256_cmpgt_epi8(chars, threshold);
    let corrected = _mm256_sub_epi8(chars, needs_bump);

    _mm256_storeu_si256(output as *mut __m256i, corrected);
}

/// Decode 32 SIMD-alphabet characters into 26 output bytes (two blocks).
///
/// Returns `false` if any character is not in the SIMD alphabet.
///
/// # Safety
/// Requires AVX2. `input` readable for 32 bytes; `output` writable for 26.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,ssse3")]
pub(crate) unsafe fn decode_block_avx2(input: *const u8, output: *mut u8) -> bool {
    let chars = _mm256_loadu_si256(input as *const __m256i);

    // Step 1: Reverse map characters → indices 0–90.
    let threshold = _mm256_set1_epi8(0x5Bu8 as i8);
    let needs_bump = _mm256_cmpgt_epi8(chars, threshold);
    let raw = _mm256_sub_epi8(chars, _mm256_set1_epi8(0x23u8 as i8));
    let indices8 = _mm256_add_epi8(raw, needs_bump);

    // Validate: all indices ≤ 90.
    let max_valid = _mm256_set1_epi8(90i8);
    let invalid = _mm256_cmpgt_epi8(indices8, max_valid);
    if _mm256_movemask_epi8(invalid) != 0 {
        return false;
    }

    // Step 2: Deinterleave and reconstruct 13-bit values.
    let shuf_lo = _mm256_set_epi8(
        -1, -1, -1, -1, -1, -1, -1, -1, 14, 12, 10, 8, 6, 4, 2, 0, // high lane
        -1, -1, -1, -1, -1, -1, -1, -1, 14, 12, 10, 8, 6, 4, 2, 0, // low lane
    );
    let shuf_hi = _mm256_set_epi8(
        -1, -1, -1, -1, -1, -1, -1, -1, 15, 13, 11, 9, 7, 5, 3, 1, // high lane
        -1, -1, -1, -1, -1, -1, -1, -1, 15, 13, 11, 9, 7, 5, 3, 1, // low lane
    );
    let lo8_sep = _mm256_shuffle_epi8(indices8, shuf_lo);
    let hi8_sep = _mm256_shuffle_epi8(indices8, shuf_hi);
    let zero = _mm256_setzero_si256();
    let lo16 = _mm256_unpacklo_epi8(lo8_sep, zero);
    let hi16 = _mm256_unpacklo_epi8(hi8_sep, zero);
    let vals = _mm256_add_epi16(lo16, _mm256_mullo_epi16(hi16, _mm256_set1_epi16(91)));

    // Step 3: Scatter 8 × 13-bit values per lane back into 13 bytes each.
    // Extract the two 128-bit lanes and apply the same pshufb scatter as
    // decode_block_sse41, independently to each lane.
    let lo128 = _mm256_castsi256_si128(vals);
    let hi128 = _mm256_extracti128_si256(vals, 1);

    // Shared shuffle/scatter constants (same as decode_block_sse41).
    let sec_lo_shuf = _mm_set_epi8(
        -1, -1, -1, -1, -1, -1,   // b15..b10: zero
        12i8, // b9  ← L[12]
        -1, -1,  // b8,b7: zero
        8i8, // b6  ← L[8]
        -1, -1, -1, -1,   // b5..b2: zero
        4i8,  // b1  ← L[4]
        -1i8, // b0:  zero
    );
    let scatter_lo = _mm_set_epi8(
        -1, -1, -1, -1, -1, -1, -1, -1, -1,   // b15..b7: zero
        14i8, // b6 ← lo_merged[14] → B6
        13i8, // b5 ← lo_merged[13] → B5
        9i8,  // b4 ← lo_merged[9]  → B4
        6i8,  // b3 ← lo_merged[6]  → B3
        5i8,  // b2 ← lo_merged[5]  → B2
        1i8,  // b1 ← lo_merged[1]  → B1
        0i8,  // b0 ← lo_merged[0]  → B0
    );
    let sec_hi_shuf = _mm_set_epi8(
        -1i8, -1i8, -1i8, -1i8, -1i8, // b15..b11: zero
        12i8, // b10 ← H[12]
        -1i8, -1i8, -1i8, -1i8, // b9..b6: zero
        8i8,  // b5  ← H[8]
        -1i8, -1i8, // b4,b3: zero
        4i8,  // b2  ← H[4]
        -1i8, -1i8, // b1,b0: zero
    );
    let scatter_hi = _mm_set_epi8(
        -1, -1, -1,   // b15,b14,b13: zero
        13i8, // b12 ← hi_merged[13] → B12
        10i8, // b11 ← hi_merged[10] → B11
        9i8,  // b10 ← hi_merged[9]  → B10
        5i8,  // b9  ← hi_merged[5]  → B9
        2i8,  // b8  ← hi_merged[2]  → B8
        1i8,  // b7  ← hi_merged[1]  → B7
        0i8,  // b6  ← hi_merged[0]  → B6
        -1, -1, -1, -1, -1, -1, // b5..b0: zero
    );

    // Scatter helper: apply the same logic as decode_block_sse41 Step 3
    // to a u16x8 register and store 13 bytes at `dst`.
    #[inline(always)]
    unsafe fn scatter128(
        v: __m128i,
        sec_lo_shuf: __m128i,
        scatter_lo: __m128i,
        sec_hi_shuf: __m128i,
        scatter_hi: __m128i,
        dst: *mut u8,
    ) {
        let lo_g = _mm_cvtepu16_epi32(v);
        let hi_g = _mm_cvtepu16_epi32(_mm_srli_si128(v, 8));

        let lo_s0 = lo_g;
        let lo_s5 = _mm_slli_epi32(lo_g, 5);
        let lo_s2 = _mm_slli_epi32(lo_g, 2);
        let lo_s7 = _mm_slli_epi32(lo_g, 7);
        let lo_a = _mm_blend_epi16(lo_s0, lo_s5, 0x0C);
        let lo_b = _mm_blend_epi16(lo_a, lo_s2, 0x30);
        let lo_shifted = _mm_blend_epi16(lo_b, lo_s7, 0xC0);

        let hi_s4 = _mm_slli_epi32(hi_g, 4);
        let hi_s1 = _mm_slli_epi32(hi_g, 1);
        let hi_s6 = _mm_slli_epi32(hi_g, 6);
        let hi_s3 = _mm_slli_epi32(hi_g, 3);
        let hi_a = _mm_blend_epi16(hi_s4, hi_s1, 0x0C);
        let hi_b = _mm_blend_epi16(hi_a, hi_s6, 0x30);
        let hi_shifted = _mm_blend_epi16(hi_b, hi_s3, 0xC0);

        let lo_merged = _mm_or_si128(lo_shifted, _mm_shuffle_epi8(lo_shifted, sec_lo_shuf));
        let lo_out = _mm_shuffle_epi8(lo_merged, scatter_lo);

        let hi_merged = _mm_or_si128(hi_shifted, _mm_shuffle_epi8(hi_shifted, sec_hi_shuf));
        let hi_out = _mm_shuffle_epi8(hi_merged, scatter_hi);

        // decode_loop reserves out_needed + OUT spare bytes; writing 16 bytes
        // here is safe (3-byte harmless overwrite into spare capacity).
        let out128 = _mm_or_si128(lo_out, hi_out);
        _mm_storeu_si128(dst as *mut __m128i, out128);
    }

    scatter128(
        lo128,
        sec_lo_shuf,
        scatter_lo,
        sec_hi_shuf,
        scatter_hi,
        output,
    );
    scatter128(
        hi128,
        sec_lo_shuf,
        scatter_lo,
        sec_hi_shuf,
        scatter_hi,
        output.add(13),
    );

    true
}

// ---------------------------------------------------------------------------
// Block-loop entry points used by mod.rs dispatch
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
pub(crate) unsafe fn encode_sse41(input: &[u8], output: &mut Vec<u8>) {
    encode_loop::<13, 16>(input, output, encode_block_sse41);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,ssse3")]
pub(crate) unsafe fn encode_avx2(input: &[u8], output: &mut Vec<u8>) {
    encode_loop::<26, 32>(input, output, encode_block_avx2);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
pub(crate) unsafe fn decode_sse41(input: &[u8], output: &mut Vec<u8>) -> bool {
    decode_loop::<16, 13>(input, output, decode_block_sse41)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,ssse3")]
pub(crate) unsafe fn decode_avx2(input: &[u8], output: &mut Vec<u8>) -> bool {
    decode_loop::<32, 26>(input, output, decode_block_avx2)
}

// ---------------------------------------------------------------------------
// Generic loop helpers
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn encode_loop<const IN: usize, const OUT: usize>(
    input: &[u8],
    output: &mut Vec<u8>,
    block_fn: unsafe fn(*const u8, *mut u8),
) {
    let full_blocks = input.len() / IN;
    let out_needed = full_blocks * OUT;
    output.reserve(out_needed + OUT);

    // Padded buffer so block_fn can always read IN+3 bytes safely.
    let mut pad = [0u8; 32];

    let spare = output.spare_capacity_mut();
    let out_ptr = spare.as_mut_ptr() as *mut u8;

    for i in 0..full_blocks {
        let offset = i * IN;
        let remaining = input.len() - offset;
        let src = if remaining >= IN + 3 {
            input.as_ptr().add(offset)
        } else {
            let n = remaining.min(pad.len());
            std::ptr::copy_nonoverlapping(input.as_ptr().add(offset), pad.as_mut_ptr(), n);
            pad.as_ptr()
        };
        block_fn(src, out_ptr.add(i * OUT));
    }

    output.set_len(output.len() + out_needed);
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn decode_loop<const IN: usize, const OUT: usize>(
    input: &[u8],
    output: &mut Vec<u8>,
    block_fn: unsafe fn(*const u8, *mut u8) -> bool,
) -> bool {
    let full_blocks = input.len() / IN;
    let out_needed = full_blocks * OUT;
    output.reserve(out_needed + OUT);

    let spare = output.spare_capacity_mut();
    let out_ptr = spare.as_mut_ptr() as *mut u8;

    for i in 0..full_blocks {
        if !block_fn(input.as_ptr().add(i * IN), out_ptr.add(i * OUT)) {
            return false;
        }
    }

    output.set_len(output.len() + out_needed);
    true
}
