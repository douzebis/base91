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
//! # Bit packing
//!
//! Group k occupies bits [k*13, k*13+12]. Groups 2k and 2k+1 form a 26-bit
//! field at bit 26k, i.e. at bit 2k of byte 3k: four fields, one per 32-bit
//! lane, each four bytes wide once aligned by a per-lane shift of 2k bits.
//! The encoder gathers bytes 3k..3k+3 into lane k with one `pshufb` and
//! splits each field into its two groups; the decoder joins the groups
//! with `pmaddwd` and scatters the fields back with two `pshufb`
//! (spec 0011).

#![allow(non_snake_case)]

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

const MAGIC91: u16 = 2881;
const SHIFT91: i32 = 2;

// ---------------------------------------------------------------------------
// Shared encode/decode helpers (compile for SSE4.1; safe to call from AVX2)
// ---------------------------------------------------------------------------

/// The constants of `encode_block_sse41`, built once per batch (spec 0011
/// §3).
#[cfg(target_arch = "x86_64")]
struct EncodeSse41 {
    gather: __m128i,
    shifts: __m128i,
    mask13: __m128i,
    magic: __m128i,
    low_byte: __m128i,
    n91: __m128i,
    base: __m128i,
    gap: __m128i,
}

#[cfg(target_arch = "x86_64")]
impl EncodeSse41 {
    #[target_feature(enable = "sse4.1,ssse3")]
    #[inline]
    unsafe fn new() -> Self {
        Self {
            gather: _mm_set_epi8(12, 11, 10, 9, 9, 8, 7, 6, 6, 5, 4, 3, 3, 2, 1, 0),
            shifts: _mm_set_epi32(1, 4, 16, 64),
            mask13: _mm_set1_epi32(0x1FFF_1FFF),
            magic: _mm_set1_epi16(MAGIC91 as i16),
            low_byte: _mm_set1_epi16(0x00FF),
            n91: _mm_set1_epi16(91),
            base: _mm_set1_epi8(0x23u8 as i8),
            gap: _mm_set1_epi8(0x26u8 as i8),
        }
    }
}

/// Encode 13 input bytes into 16 SIMD-alphabet output characters.
///
/// # Safety
/// Requires SSE4.1 + SSSE3.
/// `input` must be readable for 16 bytes (caller pads if needed).
/// `output` must be writable for 16 bytes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1,ssse3")]
#[inline]
unsafe fn encode_block_sse41(k: &EncodeSse41, input: *const u8, output: *mut u8) {
    // Load 16 bytes (13 used, caller provides ≥16 readable bytes).
    let data = _mm_loadu_si128(input as *const __m128i);

    // -----------------------------------------------------------------------
    // Step 1: unpack the 13 bytes into eight 13-bit values (spec 0011), the
    // mirror of the decoder's packing.
    //
    // Groups 2k and 2k+1 form a 26-bit field starting at bit 26k, i.e. at
    // bit 2k of byte 3k. A shuffle puts bytes 3k..3k+3 in 32-bit lane k;
    // a left shift by 6 - 2k (a multiply: SSE4.1 has no per-lane shift)
    // aligns every field on bits 6..31. Then `>> 6` holds group 2k in the
    // low 16 bits and `>> 3` group 2k+1 in the high 16 bits: a blend picks
    // each half and an AND keeps 13 bits of each.
    // -----------------------------------------------------------------------
    let gathered = _mm_shuffle_epi8(data, k.gather);
    let aligned = _mm_mullo_epi32(gathered, k.shifts);
    let halves = _mm_blend_epi16(_mm_srli_epi32(aligned, 6), _mm_srli_epi32(aligned, 3), 0xAA);
    let vals = _mm_and_si128(halves, k.mask13);
    // vals = [g0,g1,g2,g3, g4,g5,g6,g7] as u16x8.

    // -----------------------------------------------------------------------
    // Step 2: Divide each 13-bit value by 91.
    //   hi = mulhi_u16(vals, MAGIC91) >> SHIFT91
    //   lo = vals - hi * 91
    // The shift is a 32-bit one, then a mask clears the bits it brings in
    // from the neighboring 16-bit lane: LLVM fuses a 16-bit shift with the
    // multiply-high into a 32-bit multiply of widened values (7 instructions
    // instead of 3, spec 0011).
    // -----------------------------------------------------------------------
    let magic = k.magic;
    let hi = _mm_and_si128(
        _mm_srli_epi32(_mm_mulhi_epu16(vals, magic), SHIFT91),
        k.low_byte,
    );
    let lo = _mm_sub_epi16(vals, _mm_mullo_epi16(hi, k.n91));

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
    let base = k.base;
    let chars = _mm_add_epi8(interleaved, base);
    let threshold = k.gap;
    let needs_bump = _mm_cmpgt_epi8(chars, threshold);
    let corrected = _mm_sub_epi8(chars, needs_bump);

    _mm_storeu_si128(output as *mut __m128i, corrected);
}

/// The constants of `decode_block_sse41`, built once per batch (spec 0011
/// §3).
#[cfg(target_arch = "x86_64")]
struct DecodeSse41 {
    gap: __m128i,
    base: __m128i,
    apos: __m128i,
    pair_weights: __m128i,
    field_weights: __m128i,
    shifts: __m128i,
    primary: __m128i,
    secondary: __m128i,
}

#[cfg(target_arch = "x86_64")]
impl DecodeSse41 {
    #[target_feature(enable = "sse4.1,ssse3")]
    #[inline]
    unsafe fn new() -> Self {
        Self {
            gap: _mm_set1_epi8(0x26u8 as i8),
            base: _mm_set1_epi8(0x23u8 as i8),
            apos: _mm_set1_epi8(0x27),
            pair_weights: _mm_set1_epi16(91 << 8 | 1),
            field_weights: _mm_set1_epi32(8192 << 16 | 1),
            shifts: _mm_set_epi32(64, 16, 4, 1),
            primary: _mm_set_epi8(-1, -1, -1, 15, 14, 13, 12, 10, 9, 8, 6, 5, 4, 2, 1, 0),
            secondary: _mm_set_epi8(-1, -1, -1, -1, -1, -1, 11, -1, -1, 7, -1, -1, 3, -1, -1, -1),
        }
    }
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
unsafe fn decode_block_sse41(
    k: &DecodeSse41,
    input: *const u8,
    output: *mut u8,
) -> (__m128i, __m128i) {
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
    let threshold = k.gap;
    let needs_bump = _mm_cmpgt_epi8(chars, threshold); // 0xFF (-1) where > 0x26
    let raw = _mm_sub_epi8(chars, k.base);
    let indices8 = _mm_add_epi8(raw, needs_bump); // subtracts 1 where > 0x26

    // Validity (spec 0010 §5, spec 0011 §4): every byte outside 0x23–0x7E
    // gets an index above 90 as an *unsigned* byte (the subtraction wraps);
    // `'` (0x27) maps to the index of `&`, so it is forced to 0xFF. The
    // batch keeps a running maximum of these and of the pair values, and
    // compares them with 90 and 8191 once.
    let checked = _mm_or_si128(indices8, _mm_cmpeq_epi8(chars, k.apos));

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
    let vals = _mm_maddubs_epi16(indices8, k.pair_weights);

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
    let fields = _mm_madd_epi16(vals, k.field_weights);
    let placed = _mm_mullo_epi32(fields, k.shifts);
    let primary = _mm_shuffle_epi8(placed, k.primary);
    let secondary = _mm_shuffle_epi8(placed, k.secondary);
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
// high 128-bit lanes respectively. Every operation of the kernels works
// independently on each 128-bit lane — no cross-lane interaction — and the
// shuffle constants are the SSE4.1 ones repeated in both lanes.

/// The constants of `encode_block_avx2`, built once per batch (spec 0011
/// §3).
#[cfg(target_arch = "x86_64")]
struct EncodeAvx2 {
    gather: __m256i,
    shifts: __m256i,
    mask13: __m256i,
    magic: __m256i,
    low_byte: __m256i,
    n91: __m256i,
    base: __m256i,
    gap: __m256i,
}

#[cfg(target_arch = "x86_64")]
impl EncodeAvx2 {
    #[target_feature(enable = "avx2,ssse3")]
    #[inline]
    unsafe fn new() -> Self {
        Self {
            gather: _mm256_set_epi8(
                12, 11, 10, 9, 9, 8, 7, 6, 6, 5, 4, 3, 3, 2, 1, 0, // high lane
                12, 11, 10, 9, 9, 8, 7, 6, 6, 5, 4, 3, 3, 2, 1, 0, // low lane
            ),
            shifts: _mm256_set_epi32(0, 2, 4, 6, 0, 2, 4, 6),
            mask13: _mm256_set1_epi32(0x1FFF_1FFF),
            magic: _mm256_set1_epi16(MAGIC91 as i16),
            low_byte: _mm256_set1_epi16(0x00FF),
            n91: _mm256_set1_epi16(91),
            base: _mm256_set1_epi8(0x23u8 as i8),
            gap: _mm256_set1_epi8(0x26u8 as i8),
        }
    }
}

/// Encode 26 input bytes into 32 SIMD-alphabet output characters.
///
/// # Safety
/// Requires AVX2 + SSSE3. `input` readable for 32 bytes (padded); `output` for 32.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,ssse3")]
#[inline]
unsafe fn encode_block_avx2(k: &EncodeAvx2, input: *const u8, output: *mut u8) {
    // Load the two 13-byte blocks into the low and high 128-bit lanes
    // independently.  A single 256-bit load would place block1 at an offset
    // of 13 bytes into the high lane, misaligning it relative to the shuffle
    // constants.  Instead we load block0 at input[0..15] into the low lane
    // and block1 at input[13..28] into the high lane, so both lanes have
    // their respective block starting at byte 0 of the lane.
    let lo = _mm_loadu_si128(input as *const __m128i);
    let hi = _mm_loadu_si128(input.add(13) as *const __m128i);
    let data = _mm256_set_m128i(hi, lo);

    // Step 1: unpack each lane's 13 bytes into eight 13-bit values, as in
    // `encode_block_sse41`, with a real per-lane shift (`vpsllvd`) instead
    // of the multiply. The shuffle works lane by lane.
    let gathered = _mm256_shuffle_epi8(data, k.gather);
    let aligned = _mm256_sllv_epi32(gathered, k.shifts);
    let halves = _mm256_blend_epi16(
        _mm256_srli_epi32(aligned, 6),
        _mm256_srli_epi32(aligned, 3),
        0xAA,
    );
    let vals = _mm256_and_si256(halves, k.mask13);

    // Step 2: Divide by 91, the shift unfused as in `encode_block_sse41`.
    let magic = k.magic;
    let hi = _mm256_and_si256(
        _mm256_srli_epi32(_mm256_mulhi_epu16(vals, magic), SHIFT91),
        k.low_byte,
    );
    let lo = _mm256_sub_epi16(vals, _mm256_mullo_epi16(hi, k.n91));

    // Step 3: Interleave lo/hi → [lo0,hi0,lo1,hi1,...].
    let lo8 = _mm256_packus_epi16(lo, lo);
    let hi8 = _mm256_packus_epi16(hi, hi);
    let interleaved = _mm256_unpacklo_epi8(lo8, hi8);

    // Step 4: Map to SIMD-alphabet.
    let base = k.base;
    let chars = _mm256_add_epi8(interleaved, base);
    let threshold = k.gap;
    let needs_bump = _mm256_cmpgt_epi8(chars, threshold);
    let corrected = _mm256_sub_epi8(chars, needs_bump);

    _mm256_storeu_si256(output as *mut __m256i, corrected);
}

/// The constants of `decode_block_avx2`, built once per batch (spec 0011
/// §3).
#[cfg(target_arch = "x86_64")]
struct DecodeAvx2 {
    gap: __m256i,
    base: __m256i,
    apos: __m256i,
    pair_weights: __m256i,
    field_weights: __m256i,
    shifts: __m256i,
    primary: __m256i,
    secondary: __m256i,
}

#[cfg(target_arch = "x86_64")]
impl DecodeAvx2 {
    #[target_feature(enable = "avx2,ssse3")]
    #[inline]
    unsafe fn new() -> Self {
        Self {
            gap: _mm256_set1_epi8(0x26u8 as i8),
            base: _mm256_set1_epi8(0x23u8 as i8),
            apos: _mm256_set1_epi8(0x27),
            pair_weights: _mm256_set1_epi16(91 << 8 | 1),
            field_weights: _mm256_set1_epi32(8192 << 16 | 1),
            shifts: _mm256_set_epi32(6, 4, 2, 0, 6, 4, 2, 0),
            primary: _mm256_set_epi8(
                -1, -1, -1, 15, 14, 13, 12, 10, 9, 8, 6, 5, 4, 2, 1, 0, // high lane
                -1, -1, -1, 15, 14, 13, 12, 10, 9, 8, 6, 5, 4, 2, 1, 0, // low lane
            ),
            secondary: _mm256_set_epi8(
                -1, -1, -1, -1, -1, -1, 11, -1, -1, 7, -1, -1, 3, -1, -1, -1, // high lane
                -1, -1, -1, -1, -1, -1, 11, -1, -1, 7, -1, -1, 3, -1, -1, -1, // low lane
            ),
        }
    }
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
unsafe fn decode_block_avx2(
    k: &DecodeAvx2,
    input: *const u8,
    output: *mut u8,
) -> (__m256i, __m256i) {
    let chars = _mm256_loadu_si256(input as *const __m256i);

    // Step 1: Reverse map characters → indices 0–90.
    let threshold = k.gap;
    let needs_bump = _mm256_cmpgt_epi8(chars, threshold);
    let raw = _mm256_sub_epi8(chars, k.base);
    let indices8 = _mm256_add_epi8(raw, needs_bump);

    // Validity: as in `decode_block_sse41`.
    let checked = _mm256_or_si256(indices8, _mm256_cmpeq_epi8(chars, k.apos));

    // Step 2: Deinterleave and reconstruct 13-bit values.
    // val = lo + 91 * hi, as in `decode_block_sse41` (each 128-bit lane).
    let vals = _mm256_maddubs_epi16(indices8, k.pair_weights);

    // Step 3: pack each lane's eight 13-bit values into 13 bytes, as in
    // `decode_block_sse41`, with a real per-lane shift (`vpsllvd`) instead
    // of the multiply. The shuffles work lane by lane.
    let fields = _mm256_madd_epi16(vals, k.field_weights);
    let placed = _mm256_sllv_epi32(fields, k.shifts);
    let primary = _mm256_shuffle_epi8(placed, k.primary);
    let secondary = _mm256_shuffle_epi8(placed, k.secondary);
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
    let k = EncodeSse41::new();
    for l in 0..lines {
        let (src, dst) = (input.add(13 * n * l), output.add(stride * l));
        for i in 0..n {
            encode_block_sse41(&k, src.add(13 * i), dst.add(16 * i));
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
    let k = EncodeAvx2::new();
    for l in 0..lines {
        let (src, dst) = (input.add(26 * n * l), output.add(stride * l));
        for i in 0..n {
            encode_block_avx2(&k, src.add(26 * i), dst.add(32 * i));
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
    let k = DecodeSse41::new();
    // Running maximums over the whole batch, one validity test at the end.
    // Nested loops, the structure of every batch function: a flat loop pays
    // a line counter per block, 15–19 % here; one call per line pays up to
    // 41 % on short lines (spec 0011, G3).
    let (mut max_index, mut max_value) = (_mm_setzero_si128(), _mm_setzero_si128());
    for l in 0..lines {
        let (src, dst) = (input.add(stride * l), output.add(13 * n * l));
        for i in 0..n {
            let (checked, vals) = decode_block_sse41(&k, src.add(16 * i), dst.add(13 * i));
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
    // As `decode_lines_sse41`.
    let k = DecodeAvx2::new();
    let (mut max_index, mut max_value) = (_mm256_setzero_si256(), _mm256_setzero_si256());
    for l in 0..lines {
        let (src, dst) = (input.add(stride * l), output.add(26 * n * l));
        for i in 0..n {
            let (checked, vals) = decode_block_avx2(&k, src.add(32 * i), dst.add(26 * i));
            max_index = _mm256_max_epu8(max_index, checked);
            max_value = _mm256_max_epu16(max_value, vals);
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
