// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! SIMD-accelerated fixed-width basE91 variant.
//!
//! # Wire format
//!
//! A SIMD-variant stream begins with a single `-` byte (0x2D), followed by
//! fixed-width 13-bit groups encoded two characters each in the SIMD alphabet.
//! The leading `-` is not in the Henke alphabet, so the two formats are
//! unambiguously distinguished by the first byte.
//!
//! # Alphabet
//!
//! 91 consecutive printable ASCII characters omitting only `\` (0x5C):
//! - 0x23–0x5B (57 chars): `#$%&'()*+,-./0-9:;<=>?@A-Z[`
//! - 0x5D–0x7E (34 chars): `]^_`a-z{|}~`
//!
//! # Runtime dispatch
//!
//! A single binary selects the best available kernel at startup, capped by the
//! caller's [`SimdLevel`] hint:
//!
//! | Platform             | Kernel                                          |
//! |----------------------|-------------------------------------------------|
//! | x86_64 with AVX2    | AVX2 (26 bytes → 32 chars per iter)             |
//! | x86_64 with SSE4.1  | SSE4.1 (13 bytes → 16 chars per iter)           |
//! | aarch64             | NEON (13 bytes → 16 chars per iter)             |
//! | other               | scalar fixed-width fallback                     |
//!
//! The Cargo feature `force-scalar` bypasses SIMD detection (for benchmarking).

pub mod scalar;

#[cfg(target_arch = "x86_64")]
pub(crate) mod x86;

#[cfg(target_arch = "aarch64")]
pub(crate) mod aarch64;

use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Public SimdLevel
// ---------------------------------------------------------------------------

/// Arch-agnostic SIMD width selector.
///
/// Acts as a maximum-level hint: the dispatcher uses the best available
/// kernel up to this width.
/// - `Simd256` on a machine without AVX2/SVE2 falls back to `Simd128`.
/// - `Simd128` on a machine without SSE4.1/NEON falls back to `Scalar`.
///
/// Mapping to arch-specific kernels:
/// - `Scalar`  → scalar fixed-width path (all architectures)
/// - `Simd128` → SSE4.1 (x86_64) / NEON (aarch64)
/// - `Simd256` → AVX2  (x86_64) / SVE2-256 (aarch64, not yet implemented)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SimdLevel {
    Scalar,
    Simd128,
    Simd256,
}

impl Default for SimdLevel {
    fn default() -> Self {
        SimdLevel::Simd256
    }
}

// ---------------------------------------------------------------------------
// Internal arch detection
// ---------------------------------------------------------------------------

// Arch-specific level detected at runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchLevel {
    #[cfg(target_arch = "x86_64")]
    Avx2,
    #[cfg(target_arch = "x86_64")]
    Sse41,
    #[cfg(target_arch = "aarch64")]
    Neon,
    Scalar,
}

static ARCH_LEVEL: OnceLock<ArchLevel> = OnceLock::new();

fn detect_arch() -> ArchLevel {
    #[cfg(feature = "force-scalar")]
    {
        return ArchLevel::Scalar;
    }

    #[cfg(all(not(feature = "force-scalar"), target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            return ArchLevel::Avx2;
        }
        if is_x86_feature_detected!("sse4.1") {
            return ArchLevel::Sse41;
        }
        return ArchLevel::Scalar;
    }

    #[cfg(all(not(feature = "force-scalar"), target_arch = "aarch64"))]
    {
        // NEON is mandatory on aarch64.
        return ArchLevel::Neon;
    }

    #[cfg(not(any(
        feature = "force-scalar",
        target_arch = "x86_64",
        target_arch = "aarch64"
    )))]
    ArchLevel::Scalar
}

fn arch_level() -> ArchLevel {
    *ARCH_LEVEL.get_or_init(detect_arch)
}

// Resolve the effective kernel to use given a max_level hint.
fn effective_level(max_level: SimdLevel) -> ArchLevel {
    match (arch_level(), max_level) {
        // If caller caps at Scalar, always use scalar.
        (_, SimdLevel::Scalar) => ArchLevel::Scalar,

        // Simd128: use the best 128-bit kernel, no 256-bit.
        #[cfg(target_arch = "x86_64")]
        (ArchLevel::Avx2 | ArchLevel::Sse41, SimdLevel::Simd128) => ArchLevel::Sse41,
        #[cfg(target_arch = "aarch64")]
        (ArchLevel::Neon, SimdLevel::Simd128) => ArchLevel::Neon,

        // Simd256 or Simd128 with no SIMD available: fall through to scalar.
        (ArchLevel::Scalar, _) => ArchLevel::Scalar,

        // Simd256: use the best available kernel (already detected).
        #[cfg(target_arch = "x86_64")]
        (ArchLevel::Avx2, SimdLevel::Simd256) => ArchLevel::Avx2,
        #[cfg(target_arch = "x86_64")]
        (ArchLevel::Sse41, SimdLevel::Simd256) => ArchLevel::Sse41,
        #[cfg(target_arch = "aarch64")]
        (ArchLevel::Neon, SimdLevel::Simd256) => ArchLevel::Neon,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detect the best [`SimdLevel`] available on the current CPU.
pub fn detect() -> SimdLevel {
    match arch_level() {
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Avx2 => SimdLevel::Simd256,
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Sse41 => SimdLevel::Simd128,
        #[cfg(target_arch = "aarch64")]
        ArchLevel::Neon => SimdLevel::Simd128,
        ArchLevel::Scalar => SimdLevel::Scalar,
    }
}

/// Upper bound on SIMD-encoded output length for `input_len` bytes.
///
/// `wrap=0` means no line wrapping. Accounts for `\n` bytes when `wrap > 0`.
/// Use this to size buffers before any unchecked encode call.
#[inline]
pub fn encode_size_hint(input_len: usize, wrap: usize) -> usize {
    let payload = (input_len * 16).div_ceil(13) + 2;
    let newlines = if wrap > 0 { payload.div_ceil(wrap) } else { 0 };
    1 + payload + newlines // 1 for '-' prefix
}

/// Upper bound on decoded output length for `encoded_len` encoded bytes.
#[inline]
pub fn decode_size_hint(encoded_len: usize) -> usize {
    encoded_len * 7 / 8 + 1
}

/// Encode `input` using the SIMD fixed-width variant.
///
/// Output begins with `-` then SIMD-alphabet characters.
/// `wrap=0` means no line wrapping; otherwise `\n` is inserted after every
/// `wrap` output characters (`wrap` must be a multiple of 16).
/// `max_level` caps the SIMD kernel used; [`SimdLevel::default`] uses the best
/// available.
pub fn encode(input: &[u8], max_level: SimdLevel, wrap: usize) -> Vec<u8> {
    if input.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(encode_size_hint(input.len(), wrap));
    out.push(b'-');
    encode_into(input, max_level, wrap, &mut out);
    out
}

/// Decode a SIMD-variant stream (starting with `-`) to binary bytes.
///
/// Returns `None` if `input` does not start with `-`.
/// `max_level` caps the SIMD kernel used.
pub fn decode(input: &[u8], max_level: SimdLevel) -> Option<Vec<u8>> {
    if input.is_empty() {
        return Some(Vec::new());
    }
    let payload = input.strip_prefix(b"-")?;
    let mut out = Vec::with_capacity(decode_size_hint(payload.len()));
    if !decode_into(payload, max_level, &mut out) {
        return None;
    }
    Some(out)
}

/// Encode `input` into a caller-provided buffer without bounds checking.
/// Returns the number of bytes written (including the `-` prefix).
///
/// # Safety
/// `output` must point to at least `encode_size_hint(input.len(), wrap)`
/// writable bytes.
pub unsafe fn encode_unchecked(
    input: &[u8],
    max_level: SimdLevel,
    wrap: usize,
    output: *mut u8,
) -> usize {
    let mut out = Vec::from_raw_parts(output, 0, encode_size_hint(input.len(), wrap));
    out.push(b'-');
    encode_into(input, max_level, wrap, &mut out);
    let n = out.len();
    std::mem::forget(out);
    n
}

/// Decode into a caller-provided buffer without bounds checking.
/// Returns bytes written, or `usize::MAX` if input does not start with `-`.
///
/// # Safety
/// `output` must point to at least `decode_size_hint(input.len())`
/// writable bytes.
pub unsafe fn decode_unchecked(input: &[u8], max_level: SimdLevel, output: *mut u8) -> usize {
    let Some(payload) = input.strip_prefix(b"-") else {
        return usize::MAX;
    };
    let mut out = Vec::from_raw_parts(output, 0, decode_size_hint(input.len()));
    decode_into(payload, max_level, &mut out);
    let n = out.len();
    std::mem::forget(out);
    n
}

// ---------------------------------------------------------------------------
// Internal dispatch helpers
// ---------------------------------------------------------------------------

fn encode_into(input: &[u8], max_level: SimdLevel, wrap: usize, output: &mut Vec<u8>) {
    match effective_level(max_level) {
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Avx2 => {
            let full = (input.len() / 26) * 26;
            unsafe { x86::encode_avx2(&input[..full], output) };
            // Handle 13–25 byte tail with SSE4.1 before falling to scalar.
            let tail = &input[full..];
            let sse_full = (tail.len() / 13) * 13;
            if sse_full > 0 {
                unsafe { x86::encode_sse41(&tail[..sse_full], output) };
            }
            let mut sc = scalar::ScalarEncoder::new();
            sc.encode(&tail[sse_full..], output);
            sc.finish(output);
        }
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Sse41 => {
            let full = (input.len() / 13) * 13;
            unsafe { x86::encode_sse41(&input[..full], output) };
            let mut sc = scalar::ScalarEncoder::new();
            sc.encode(&input[full..], output);
            sc.finish(output);
        }
        #[cfg(target_arch = "aarch64")]
        ArchLevel::Neon => {
            let full = (input.len() / 13) * 13;
            unsafe { aarch64::encode_neon(&input[..full], output) };
            let mut sc = scalar::ScalarEncoder::new();
            sc.encode(&input[full..], output);
            sc.finish(output);
        }
        ArchLevel::Scalar => {
            let mut sc = scalar::ScalarEncoder::new();
            sc.encode(input, output);
            sc.finish(output);
        }
    }
    // Insert wrap newlines if requested (post-payload, after the '-' prefix byte).
    if wrap > 0 && output.len() > 1 {
        insert_wrap_newlines(output, wrap);
    }
}

fn decode_into(input: &[u8], max_level: SimdLevel, output: &mut Vec<u8>) -> bool {
    match effective_level(max_level) {
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Avx2 => {
            let full = (input.len() / 32) * 32;
            if !unsafe { x86::decode_avx2(&input[..full], output) } {
                return false;
            }
            // Handle 16–31 char tail with SSE4.1 before falling to scalar.
            let tail = &input[full..];
            let sse_full = (tail.len() / 16) * 16;
            if sse_full > 0 {
                if !unsafe { x86::decode_sse41(&tail[..sse_full], output) } {
                    return false;
                }
            }
            let mut sc = scalar::ScalarDecoder::new();
            sc.decode(&tail[sse_full..], output);
            sc.finish(output);
            true
        }
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Sse41 => {
            let full = (input.len() / 16) * 16;
            if !unsafe { x86::decode_sse41(&input[..full], output) } {
                return false;
            }
            let mut sc = scalar::ScalarDecoder::new();
            sc.decode(&input[full..], output);
            sc.finish(output);
            true
        }
        #[cfg(target_arch = "aarch64")]
        ArchLevel::Neon => {
            let full = (input.len() / 16) * 16;
            if !unsafe { aarch64::decode_neon(&input[..full], output) } {
                return false;
            }
            let mut sc = scalar::ScalarDecoder::new();
            sc.decode(&input[full..], output);
            sc.finish(output);
            true
        }
        ArchLevel::Scalar => {
            let mut sc = scalar::ScalarDecoder::new();
            sc.decode(input, output);
            sc.finish(output);
            true
        }
    }
}

/// Insert `\n` after every `wrap` payload characters (after the leading `-`).
fn insert_wrap_newlines(output: &mut Vec<u8>, wrap: usize) {
    // payload starts at index 1 (after '-')
    let payload_len = output.len() - 1;
    let newline_count = payload_len / wrap;
    if newline_count == 0 {
        return;
    }
    // Grow the buffer to fit the newlines, then shift right-to-left.
    output.resize(output.len() + newline_count, 0);
    let buf = unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr(), output.len()) };
    let total = buf.len();
    // dst: write cursor (from the end); src: read cursor into original payload.
    let mut dst = total - 1;
    // last byte of original payload (in-place)
    let mut src = total - 1 - newline_count;
    // Number of chars in the trailing partial block.
    let mut col = payload_len % wrap;
    if col == 0 {
        col = wrap;
    }
    loop {
        // Copy `col` payload chars rightward.
        for _ in 0..col {
            buf[dst] = buf[src];
            dst -= 1;
            src -= 1;
        }
        if src == 0 {
            // src==0 means we just passed the '-' prefix byte; done.
            break;
        }
        buf[dst] = b'\n';
        dst -= 1;
        col = wrap;
    }
}

// ---------------------------------------------------------------------------
// Streaming encoder
// ---------------------------------------------------------------------------

/// Stateful SIMD fixed-width encoder. Wrap is built in — no second pass.
///
/// Feed input with [`Encoder::encode`]; flush with [`Encoder::finish`].
/// The `-` prefix is written on the first `encode` call.
pub struct Encoder {
    scalar: scalar::ScalarEncoder,
    max_level: SimdLevel,
    wrap: usize,
    prefix_written: bool,
}

impl Encoder {
    pub fn new(max_level: SimdLevel, wrap: usize) -> Self {
        Self {
            scalar: scalar::ScalarEncoder::new(),
            max_level,
            wrap,
            prefix_written: false,
        }
    }

    /// Encode `input`, appending SIMD-variant characters to `output`.
    /// Writes the `-` prefix on the first call.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        if !self.prefix_written {
            output.push(b'-');
            self.prefix_written = true;
        }
        // If there are pending carry bits from the previous chunk, drain them
        // through the scalar encoder until alignment (nbits==0) is restored,
        // then let SIMD process the remaining aligned bytes.
        //
        // nbits=0 is only reached at 13-byte block boundaries (104 bits = 8×13).
        // In the worst case (nbits=1 on entry) up to 12 bytes must be consumed
        // before the next block boundary restores alignment.
        let input = if !self.scalar.is_aligned() {
            let mut i = 0;
            while i < input.len() && !self.scalar.is_aligned() {
                self.scalar.encode(&input[i..i + 1], output);
                i += 1;
            }
            &input[i..]
        } else {
            input
        };

        match effective_level(self.max_level) {
            #[cfg(target_arch = "x86_64")]
            ArchLevel::Avx2 => {
                let full = (input.len() / 26) * 26;
                unsafe { x86::encode_avx2(&input[..full], output) };
                let tail = &input[full..];
                let sse_full = (tail.len() / 13) * 13;
                if sse_full > 0 {
                    unsafe { x86::encode_sse41(&tail[..sse_full], output) };
                }
                self.scalar.encode(&tail[sse_full..], output);
            }
            #[cfg(target_arch = "x86_64")]
            ArchLevel::Sse41 => {
                let full = (input.len() / 13) * 13;
                unsafe { x86::encode_sse41(&input[..full], output) };
                self.scalar.encode(&input[full..], output);
            }
            #[cfg(target_arch = "aarch64")]
            ArchLevel::Neon => {
                let full = (input.len() / 13) * 13;
                unsafe { aarch64::encode_neon(&input[..full], output) };
                self.scalar.encode(&input[full..], output);
            }
            ArchLevel::Scalar => {
                self.scalar.encode(input, output);
            }
        }
    }

    /// Flush remaining bits (0–2 chars).
    pub fn finish(self, output: &mut Vec<u8>) {
        self.scalar.finish(output);
    }
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new(SimdLevel::default(), 0)
    }
}

// ---------------------------------------------------------------------------
// Streaming decoder
// ---------------------------------------------------------------------------

/// Stateful SIMD fixed-width decoder. Auto-detects Henke vs SIMD from first byte.
pub struct Decoder {
    scalar: scalar::ScalarDecoder,
    max_level: SimdLevel,
    format_detected: bool,
    is_simd: bool,
    henke: Option<crate::Decoder>,
}

impl Decoder {
    pub fn new(max_level: SimdLevel) -> Self {
        Self {
            scalar: scalar::ScalarDecoder::new(),
            max_level,
            format_detected: false,
            is_simd: false,
            henke: None,
        }
    }

    /// Decode `input`, appending bytes to `output`.
    /// Detects format from the first byte. Returns `false` on invalid characters.
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) -> bool {
        if input.is_empty() {
            return true;
        }
        if !self.format_detected {
            self.format_detected = true;
            if input[0] == b'-' {
                self.is_simd = true;
                return decode_into(&input[1..], self.max_level, output);
            } else {
                self.is_simd = false;
                self.henke = Some(crate::Decoder::new());
                self.henke.as_mut().unwrap().decode(input, output);
                return true;
            }
        }
        if self.is_simd {
            decode_into(input, self.max_level, output)
        } else {
            self.henke.as_mut().unwrap().decode(input, output);
            true
        }
    }

    pub fn finish(self, output: &mut Vec<u8>) {
        if self.is_simd {
            self.scalar.finish(output);
        } else if let Some(h) = self.henke {
            h.finish(output);
        }
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new(SimdLevel::default())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(input: &[u8]) -> Vec<u8> {
        let encoded = encode(input, SimdLevel::default(), 0);
        if !encoded.is_empty() {
            assert_eq!(encoded[0], b'-');
        }
        decode(&encoded, SimdLevel::default()).expect("decode failed")
    }

    #[test]
    fn empty() {
        assert_eq!(encode(b"", SimdLevel::default(), 0), b"");
        assert_eq!(decode(b"", SimdLevel::default()), Some(vec![]));
    }

    #[test]
    fn all_bytes() {
        let input: Vec<u8> = (0u8..=255).collect();
        assert_eq!(round_trip(&input), input);
    }

    #[test]
    fn hello() {
        assert_eq!(round_trip(b"Hello, world!"), b"Hello, world!");
    }

    #[test]
    fn large() {
        let input: Vec<u8> = (0u8..=255).cycle().take(100_000).collect();
        assert_eq!(round_trip(&input), input);
    }

    #[test]
    fn simd_matches_scalar() {
        let input: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        let simd_encoded = encode(&input, SimdLevel::default(), 0);
        let scalar_decoded = scalar::decode(&simd_encoded).expect("scalar decode failed");
        assert_eq!(scalar_decoded, input);
    }

    #[test]
    fn level_scalar_matches_scalar_module() {
        let input: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        let a = encode(&input, SimdLevel::Scalar, 0);
        let b = scalar::encode(&input);
        assert_eq!(a, b);
    }

    #[test]
    fn all_levels_round_trip() {
        let input: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        for level in [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256] {
            let encoded = encode(&input, level, 0);
            let decoded = decode(&encoded, level).expect("decode failed");
            assert_eq!(decoded, input, "round-trip failed for {level:?}");
        }
    }

    #[test]
    fn no_prefix_returns_none() {
        let encoded = encode(b"test", SimdLevel::default(), 0);
        assert!(decode(&encoded[1..], SimdLevel::default()).is_none());
    }

    #[test]
    fn simd_and_henke_are_different() {
        let input = b"Hello, world!";
        let simd = encode(input, SimdLevel::default(), 0);
        let henke = crate::encode(input);
        assert_ne!(&simd[1..], henke.as_slice());
        assert_eq!(simd[0], b'-');
        assert_ne!(henke[0], b'-');
    }

    #[test]
    fn encoder_chunk_boundaries() {
        // Verify Encoder produces identical output regardless of chunk split,
        // covering the carry-state bug where a fresh ScalarEncoder was created
        // per encode() call, discarding bits from the previous tail.
        let input: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        let reference = encode(&input, SimdLevel::default(), 0);
        for chunk_size in [1, 2, 7, 13, 14, 25, 26, 27, 64, 100, 256] {
            let mut enc = Encoder::new(SimdLevel::default(), 0);
            let mut out = Vec::new();
            for chunk in input.chunks(chunk_size) {
                enc.encode(chunk, &mut out);
            }
            enc.finish(&mut out);
            assert_eq!(out, reference, "chunk_size={chunk_size}");
        }
    }
}
