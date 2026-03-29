// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! ARM NEON kernel for the fixed-width basE91 variant.
//!
//! NEON is mandatory on aarch64 — no runtime feature detection needed.
//! Processes one 13-byte block (→ 16 chars) per call using 128-bit
//! NEON registers.
//!
//! # Bit extraction
//!
//! Groups and their source bytes / in-byte shifts:
//!   g0: bytes  0,1,2   shift 0
//!   g1: bytes  1,2,3   shift 5
//!   g2: bytes  3,4,5   shift 2
//!   g3: bytes  4,5,6   shift 7
//!   g4: bytes  6,7,8   shift 4
//!   g5: bytes  8,9,10  shift 1
//!   g6: bytes  9,10,11 shift 6
//!   g7: bytes 11,12,13 shift 3  (byte 13 is safe padding)
//!
//! `vqtbl1q_u8` (NEON pshufb equivalent) loads each group's 3 bytes into
//! a 32-bit lane (4th byte zeroed via index 0xFF).  Then 4× `vshrq_n_u32`
//! (constant right-shift) + `vbslq_u32` (bit-select / blend) extracts each
//! group value.
//!
//! # Division by 91
//!
//! `vmulq_u16` computes the full 32-bit product in u32 lanes; we then
//! narrow and shift.  Alternatively, we use a scalar-friendly approach:
//! MAGIC91 = 2881, SHIFT91 = 2 (same as x86).
//! hi = (v * 2881) >> (16 + 2);   lo = v - hi * 91.
//! Implemented with `vmull_u16` (widening) + `vshrq_n_u32` + `vmovn_u32`.

#![allow(non_snake_case)]

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

const MAGIC91: u16 = 2881;
const SHIFT91: i32 = 18; // 16 + 2

/// Encode 13 input bytes into 16 SIMD-alphabet output characters.
///
/// # Safety
/// `input` must be readable for at least 16 bytes (padded); `output` for 16.
/// Only safe to call on aarch64.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub(crate) unsafe fn encode_block_neon(input: *const u8, output: *mut u8) {
    let data = vld1q_u8(input);

    // -----------------------------------------------------------------------
    // Step 1: Extract 8 × 13-bit groups using vqtbl1q_u8 + vshrq_n_u32 + blend.
    //
    // vqtbl1q_u8: out[i] = (idx[i] >= 16) ? 0 : data[idx[i] & 0x0F].
    // Index 0xFF produces a zero byte.
    // We load two 128-bit registers (shuf_lo for g0-g3, shuf_hi for g4-g7),
    // each placing one group per 32-bit lane in little-endian order
    // (LSB byte first = lowest source byte at lane byte 0).
    // -----------------------------------------------------------------------
    let shuf_lo = [
        // lane 0 (bytes 0-3): g0 = bytes 0,1,2 + zero
        0u8, 1, 2, 0xFF, // lane 1 (bytes 4-7): g1 = bytes 1,2,3 + zero
        1, 2, 3, 0xFF, // lane 2 (bytes 8-11): g2 = bytes 3,4,5 + zero
        3, 4, 5, 0xFF, // lane 3 (bytes 12-15): g3 = bytes 4,5,6 + zero
        4, 5, 6, 0xFF,
    ];
    let shuf_hi = [
        // lane 0: g4 = bytes 6,7,8 + zero
        6u8, 7, 8, 0xFF, // lane 1: g5 = bytes 8,9,10 + zero
        8, 9, 10, 0xFF, // lane 2: g6 = bytes 9,10,11 + zero
        9, 10, 11, 0xFF, // lane 3: g7 = bytes 11,12 + zero + zero
        11, 12, 0xFF, 0xFF,
    ];
    let tbl_lo = vld1q_u8(shuf_lo.as_ptr());
    let tbl_hi = vld1q_u8(shuf_hi.as_ptr());
    let slo = vqtbl1q_u8(data, tbl_lo);
    let shi = vqtbl1q_u8(data, tbl_hi);

    // Reinterpret as u32 for shift operations.
    let slo32 = vreinterpretq_u32_u8(slo);
    let shi32 = vreinterpretq_u32_u8(shi);

    // Four shifted copies per half; vbslq_u32 selects per-lane.
    // vbslq_u32(mask, a, b): result[i] = (mask[i] & a[i]) | (~mask[i] & b[i]).
    // We build a mask that is all-ones for the lanes we want from 'a'.
    //
    // For slo: shifts 0,5,2,7 for lanes 0,1,2,3.
    let slo0 = slo32;
    let slo5 = vshrq_n_u32(slo32, 5);
    let slo2 = vshrq_n_u32(slo32, 2);
    let slo7 = vshrq_n_u32(slo32, 7);
    // Lane masks: all-ones (0xFFFFFFFF) for the lane to pick from 'a', else 0.
    let mask_lo_0: [u32; 4] = [0xFFFFFFFF, 0, 0, 0]; // pick lane 0 from slo0
    let mask_lo_1: [u32; 4] = [0, 0xFFFFFFFF, 0, 0]; // pick lane 1 from slo5
    let mask_lo_2: [u32; 4] = [0, 0, 0xFFFFFFFF, 0]; // pick lane 2 from slo2
    let mask_lo_3: [u32; 4] = [0, 0, 0, 0xFFFFFFFF]; // pick lane 3 from slo7
    let m0 = vld1q_u32(mask_lo_0.as_ptr());
    let m1 = vld1q_u32(mask_lo_1.as_ptr());
    let m2 = vld1q_u32(mask_lo_2.as_ptr());
    let m3 = vld1q_u32(mask_lo_3.as_ptr());
    // Start with slo7 for all lanes, then overlay correct lanes.
    let lo32_a = vbslq_u32(m0, slo0, slo7);
    let lo32_b = vbslq_u32(m1, slo5, lo32_a);
    let lo32_c = vbslq_u32(m2, slo2, lo32_b);
    let lo32_d = vbslq_u32(m3, slo7, lo32_c);
    // lo32_d: lane0=slo0>>0, lane1=slo5>>5... wait, slo0=slo32, slo5=slo32>>5,
    // so lane0 of lo32_d = lane0 of slo0 = lane0 of slo32 (no shift). ✓
    // lane1 of lo32_d = lane1 of slo5 = lane1 of (slo32>>5). ✓
    let lo_vals = lo32_d;

    // For shi: shifts 4,1,6,3 for lanes 0,1,2,3.
    let shi4 = vshrq_n_u32(shi32, 4);
    let shi1 = vshrq_n_u32(shi32, 1);
    let shi6 = vshrq_n_u32(shi32, 6);
    let shi3 = vshrq_n_u32(shi32, 3);
    let hi32_a = vbslq_u32(m0, shi4, shi3);
    let hi32_b = vbslq_u32(m1, shi1, hi32_a);
    let hi32_c = vbslq_u32(m2, shi6, hi32_b);
    let hi_vals = vbslq_u32(m3, shi3, hi32_c);

    // Mask to 13 bits.
    let mask13 = vdupq_n_u32(0x1FFF);
    let lo_masked = vandq_u32(lo_vals, mask13);
    let hi_masked = vandq_u32(hi_vals, mask13);

    // Narrow u32→u16: vmovn_u32 takes low 16 bits of each lane.
    let lo16 = vmovn_u32(lo_masked); // u16x4
    let hi16 = vmovn_u32(hi_masked); // u16x4
    let vals = vcombine_u16(lo16, hi16); // [g0,g1,g2,g3, g4,g5,g6,g7] u16x8

    // -----------------------------------------------------------------------
    // Step 2: Divide each 13-bit value by 91 using widening multiply.
    //   hi = (v * MAGIC91) >> SHIFT91
    //   lo = v - hi * 91
    // vmull_u16: u16x4 × u16x4 → u32x4 (widening, no overflow).
    // -----------------------------------------------------------------------
    let magic = vdupq_n_u16(MAGIC91);
    let vals_lo = vget_low_u16(vals);
    let vals_hi = vget_high_u16(vals);

    let prod_lo = vmull_u16(vals_lo, vget_low_u16(magic));
    let prod_hi = vmull_u16(vals_hi, vget_high_u16(magic));
    let hi_lo = vmovn_u32(vshrq_n_u32(prod_lo, SHIFT91)); // u16x4
    let hi_hi = vmovn_u32(vshrq_n_u32(prod_hi, SHIFT91)); // u16x4
    let hi = vcombine_u16(hi_lo, hi_hi); // u16x8

    let lo = vsubq_u16(vals, vmulq_u16(hi, vdupq_n_u16(91)));

    // -----------------------------------------------------------------------
    // Step 3: Interleave lo/hi into output byte order [lo0,hi0,lo1,hi1,...].
    // -----------------------------------------------------------------------
    let lo8 = vmovn_u16(lo);
    let hi8 = vmovn_u16(hi);
    // vzipq_u8 interleaves: [lo0,hi0,lo1,hi1,...,lo7,hi7]
    let interleaved = vzipq_u8(vcombine_u8(lo8, lo8), vcombine_u8(hi8, hi8)).0;

    // -----------------------------------------------------------------------
    // Step 4: Map indices 0–90 to SIMD-alphabet characters.
    //   indices 0–56  → 0x23 + index
    //   indices 57–90 → 0x24 + index  (skip 0x5C = '\')
    // Add 0x23, then subtract cmpgt mask (0xFF where > 0x5B) to add 1.
    // -----------------------------------------------------------------------
    let base = vdupq_n_u8(0x23);
    let chars = vaddq_u8(interleaved, base);
    let threshold = vdupq_n_u8(0x5B);
    let needs_bump = vcgtq_u8(chars, threshold); // 0xFF where > 0x5B
    let corrected = vsubq_u8(chars, needs_bump); // subtract 0xFF = add 1

    vst1q_u8(output, corrected);
}

/// Decode 16 SIMD-alphabet characters into 13 output bytes.
///
/// Returns `false` if any input character is not in the SIMD alphabet.
///
/// # Safety
/// `input` must be readable for 16 bytes; `output` writable for 13.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub(crate) unsafe fn decode_block_neon(input: *const u8, output: *mut u8) -> bool {
    let chars = vld1q_u8(input);

    // -----------------------------------------------------------------------
    // Step 1: Reverse map characters to indices 0–90.
    //   chars 0x23–0x5B: index = char - 0x23
    //   chars 0x5D–0x7E: index = char - 0x24  (= (char - 0x23) - 1)
    // needs_bump = 0xFF where char > 0x5B; subtracting 0xFF adds 1.
    // index = (char - 0x23) + needs_bump   (needs_bump is 0xFF = -1 in u8)
    // -----------------------------------------------------------------------
    let base = vdupq_n_u8(0x23);
    let threshold = vdupq_n_u8(0x5B);
    let needs_bump = vcgtq_u8(chars, threshold); // 0xFF where > 0x5B
    let raw = vsubq_u8(chars, base);
    let indices8 = vaddq_u8(raw, needs_bump); // subtracts 1 where > 0x5B

    // Validate: all indices must be ≤ 90.
    let max_valid = vdupq_n_u8(90);
    let invalid = vcgtq_u8(indices8, max_valid);
    if vmaxvq_u8(invalid) != 0 {
        return false;
    }

    // -----------------------------------------------------------------------
    // Step 2: Deinterleave and reconstruct 13-bit values.
    //   indices8 = [lo0,hi0,lo1,hi1,...,lo7,hi7]
    //   val[i] = lo[i] + hi[i] * 91
    // -----------------------------------------------------------------------
    let zero = vdupq_n_u8(0);
    // vuzpq_u8: unzip even/odd bytes.
    let uzp = vuzpq_u8(indices8, zero);
    let lo8_sep = uzp.0;
    let hi8_sep = uzp.1;
    // Widen to u16, then val = lo + hi * 91.
    let lo16 = vmovl_u8(vget_low_u8(lo8_sep));
    let hi16 = vmovl_u8(vget_low_u8(hi8_sep));
    let vals = vmlaq_u16(lo16, hi16, vdupq_n_u16(91));

    // -----------------------------------------------------------------------
    // Step 3: Scatter 8 × 13-bit values back into 13 output bytes using
    // vqtbl1q_u8 (NEON equivalent of pshufb).
    //
    // Mirror of the x86 pshufb scatter design:
    //   - Widen vals (u16x8) to two u32x4 registers: lo_g (g0–g3), hi_g (g4–g7).
    //   - Left-shift each 32-bit lane by the group's in-byte offset.
    //   - vqtbl1q_u8 aligns secondary contributors to primary byte slots; OR merges.
    //   - A second vqtbl1q_u8 scatters merged bytes to final output positions.
    //   - OR lo_out and hi_out; store 16 bytes (safe: caller reserves extra space).
    //
    // vqtbl1q_u8(tbl, idx): out[i] = (idx[i] >= 16) ? 0 : tbl[idx[i]].
    // Index 0xFF (≥ 16) produces a zero byte — same as pshufb index 0x80.
    // -----------------------------------------------------------------------

    // Widen u16x8 → two u32x4.
    let lo16 = vget_low_u16(vals);
    let hi16 = vget_high_u16(vals);
    let lo_g = vmovl_u16(lo16); // u32x4: g0,g1,g2,g3
    let hi_g = vmovl_u16(hi16); // u32x4: g4,g5,g6,g7

    // Left-shift each lane by its group's in-byte bit offset.
    // lo group shifts: lane0=0, lane1=5, lane2=2, lane3=7.
    let lo_s0 = lo_g;
    let lo_s2 = vshlq_n_u32(lo_g, 2);
    let lo_s5 = vshlq_n_u32(lo_g, 5);
    let lo_s7 = vshlq_n_u32(lo_g, 7);
    // vbslq_u32(mask, a, b): result[i] = (mask[i] & a[i]) | (~mask[i] & b[i])
    let mask0: [u32; 4] = [0xFFFFFFFF, 0, 0, 0];
    let mask1: [u32; 4] = [0, 0xFFFFFFFF, 0, 0];
    let mask2: [u32; 4] = [0, 0, 0xFFFFFFFF, 0];
    let mask3: [u32; 4] = [0, 0, 0, 0xFFFFFFFF];
    let m0 = vld1q_u32(mask0.as_ptr());
    let m1 = vld1q_u32(mask1.as_ptr());
    let m2 = vld1q_u32(mask2.as_ptr());
    let m3 = vld1q_u32(mask3.as_ptr());
    let lo_a = vbslq_u32(m0, lo_s0, lo_s7);
    let lo_b = vbslq_u32(m1, lo_s5, lo_a);
    let lo_c = vbslq_u32(m2, lo_s2, lo_b);
    let lo_shifted = vreinterpretq_u8_u32(vbslq_u32(m3, lo_s7, lo_c));

    // hi group shifts: lane0=4, lane1=1, lane2=6, lane3=3.
    let hi_s1 = vshlq_n_u32(hi_g, 1);
    let hi_s3 = vshlq_n_u32(hi_g, 3);
    let hi_s4 = vshlq_n_u32(hi_g, 4);
    let hi_s6 = vshlq_n_u32(hi_g, 6);
    let hi_a = vbslq_u32(m0, hi_s4, hi_s3);
    let hi_b = vbslq_u32(m1, hi_s1, hi_a);
    let hi_c = vbslq_u32(m2, hi_s6, hi_b);
    let hi_shifted = vreinterpretq_u8_u32(vbslq_u32(m3, hi_s3, hi_c));

    // Merge secondary contributors into primary byte slots.
    // lo: B1←L[1]|L[4], B3←L[6]|L[8], B4←L[9]|L[12]
    // Route secondaries: L[4]→pos1, L[8]→pos6, L[12]→pos9.
    let sec_lo_idx: [u8; 16] = [
        0xFF, 4, 0xFF, 0xFF, 0xFF, 0xFF, 8, 0xFF, 0xFF, 12, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let sec_lo_tbl = vld1q_u8(sec_lo_idx.as_ptr());
    let lo_merged = vorrq_u8(lo_shifted, vqtbl1q_u8(lo_shifted, sec_lo_tbl));

    // Scatter lo_merged to output bytes B0–B6.
    // lo_merged bytes of interest: 0→B0, 1→B1, 5→B2, 6→B3, 9→B4, 13→B5, 14→B6.
    let scatter_lo_idx: [u8; 16] = [
        0, 1, 5, 6, 9, 13, 14, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let scatter_lo_tbl = vld1q_u8(scatter_lo_idx.as_ptr());
    let lo_out = vqtbl1q_u8(lo_merged, scatter_lo_tbl);

    // hi: B8←H[2]|H[4], B9←H[5]|H[8], B11←H[10]|H[12]
    // Route secondaries: H[4]→pos2, H[8]→pos5, H[12]→pos10.
    let sec_hi_idx: [u8; 16] = [
        0xFF, 0xFF, 4, 0xFF, 0xFF, 8, 0xFF, 0xFF, 0xFF, 0xFF, 12, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let sec_hi_tbl = vld1q_u8(sec_hi_idx.as_ptr());
    let hi_merged = vorrq_u8(hi_shifted, vqtbl1q_u8(hi_shifted, sec_hi_tbl));

    // Scatter hi_merged to output bytes B6–B12.
    // hi_merged bytes of interest: 0→B6, 1→B7, 2→B8, 5→B9, 9→B10, 10→B11, 13→B12.
    let scatter_hi_idx: [u8; 16] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 1, 2, 5, 9, 10, 13, 0xFF, 0xFF, 0xFF,
    ];
    let scatter_hi_tbl = vld1q_u8(scatter_hi_idx.as_ptr());
    let hi_out = vqtbl1q_u8(hi_merged, scatter_hi_tbl);

    // Combine and store 16 bytes (decode_neon reserves extra output space,
    // so 3 bytes of overwrite into spare capacity are safe).
    let out128 = vorrq_u8(lo_out, hi_out);
    vst1q_u8(output, out128);

    true
}

// ---------------------------------------------------------------------------
// Block-loop entry points
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub(crate) unsafe fn encode_neon(input: &[u8], output: &mut Vec<u8>) {
    let full_blocks = input.len() / 13;
    output.reserve(full_blocks * 16 + 16);

    let mut pad_buf = [0u8; 16];
    let spare = output.spare_capacity_mut();
    let out_ptr = spare.as_mut_ptr() as *mut u8;

    for i in 0..full_blocks {
        let src = input.as_ptr().add(i * 13);
        let remaining = input.len() - i * 13;
        let src_ptr = if remaining >= 16 {
            src
        } else {
            let copy_len = remaining.min(16);
            std::ptr::copy_nonoverlapping(src, pad_buf.as_mut_ptr(), copy_len);
            pad_buf.as_ptr()
        };
        encode_block_neon(src_ptr, out_ptr.add(i * 16));
    }
    output.set_len(output.len() + full_blocks * 16);
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
pub(crate) unsafe fn decode_neon(input: &[u8], output: &mut Vec<u8>) -> bool {
    let full_blocks = input.len() / 16;
    output.reserve(full_blocks * 13 + 13);

    let spare = output.spare_capacity_mut();
    let out_ptr = spare.as_mut_ptr() as *mut u8;

    for i in 0..full_blocks {
        let src = input.as_ptr().add(i * 16);
        if !decode_block_neon(src, out_ptr.add(i * 13)) {
            return false;
        }
    }
    output.set_len(output.len() + full_blocks * 13);
    true
}
