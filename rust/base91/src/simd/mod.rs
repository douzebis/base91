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
//! 91 consecutive printable ASCII characters omitting only `'` (0x27):
//! - 0x23–0x26  (4 chars): `#$%&`
//! - 0x28–0x7E (87 chars): `()*+,-./0-9:;<=>?@A-Z[\]^_`a-z{|}~`
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
//!
//! Without the `std` feature (`no_std`), there is no runtime CPU detection:
//! the kernel is chosen at compile time from the target features the crate
//! is built with (`-C target-cpu=…` or `-C target-feature=+avx2`, …). On
//! soft-float targets (`x86_64-unknown-none`,
//! `aarch64-unknown-none-softfloat`, …) keep the target's defaults: enabling
//! SIMD target features there makes the kernels slower, not faster, because
//! the compiler lowers them to scalar code.

#[cfg(feature = "alloc")]
pub mod scalar;

#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub(crate) mod x86;

#[cfg(all(feature = "alloc", target_arch = "aarch64"))]
pub(crate) mod aarch64;

#[cfg(feature = "alloc")]
mod driver;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SimdLevel {
    Scalar,
    Simd128,
    #[default]
    Simd256,
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

#[cfg(feature = "std")]
static ARCH_LEVEL: std::sync::OnceLock<ArchLevel> = std::sync::OnceLock::new();

/// Runtime CPU detection (`std` only).
#[cfg(feature = "std")]
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
        ArchLevel::Scalar
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

#[cfg(feature = "std")]
fn arch_level() -> ArchLevel {
    *ARCH_LEVEL.get_or_init(detect_arch)
}

/// Compile-time selection without `std` (spec 0004 §4): the target features
/// the crate is compiled with decide. The feature sets checked match the
/// kernels' `#[target_feature(enable = …)]` attributes. `if cfg!(…)` (rather
/// than `#[cfg]`) keeps every variant constructed whatever the target.
#[cfg(not(feature = "std"))]
fn detect_arch_static() -> ArchLevel {
    if cfg!(feature = "force-scalar") {
        return ArchLevel::Scalar;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if cfg!(target_feature = "avx2") {
            return ArchLevel::Avx2;
        }
        if cfg!(all(target_feature = "sse4.1", target_feature = "ssse3")) {
            return ArchLevel::Sse41;
        }
    }
    // NEON is checked explicitly: aarch64-unknown-none-softfloat disables it.
    #[cfg(target_arch = "aarch64")]
    {
        if cfg!(target_feature = "neon") {
            return ArchLevel::Neon;
        }
    }
    ArchLevel::Scalar
}

#[cfg(not(feature = "std"))]
#[inline(always)]
fn arch_level() -> ArchLevel {
    detect_arch_static()
}

// Resolve the effective kernel to use given a max_level hint.
//
// The kernels this returns are safe to call: their target features were
// detected at runtime (`std`) or are enabled at compile time (`no_std`).
#[cfg(feature = "alloc")]
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
///
/// With the `std` feature this is runtime detection (cached). Without it
/// (`no_std`), the level comes from the target features the crate is compiled
/// with: build with `-C target-cpu=…` or `-C target-feature=…` to enable the
/// SIMD kernels. On soft-float targets, keep the target's defaults (see the
/// module documentation).
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
    let newlines = if wrap > 0 {
        1 + payload.div_ceil(wrap)
    } else {
        0
    };
    1 + payload + newlines // 1 for '-' prefix; +1 newline for the '-\n' header when wrapping
}

/// Upper bound on decoded output length for `encoded_len` encoded bytes.
#[inline]
pub fn decode_size_hint(encoded_len: usize) -> usize {
    encoded_len * 7 / 8 + 1
}

/// Encode `input` using the SIMD fixed-width variant.
///
/// Output begins with `-` then SIMD-alphabet characters.
/// `wrap=0` means no line wrapping; otherwise the payload is written in lines
/// of `wrap` characters, each followed by `\n` (spec 0008 §3.4).
/// `max_level` caps the SIMD kernel used; [`SimdLevel::default`] uses the best
/// available.
///
/// # Panics
/// If `wrap` is neither 0 nor a multiple of 32.
#[cfg(feature = "alloc")]
pub fn encode(input: &[u8], max_level: SimdLevel, wrap: usize) -> Vec<u8> {
    assert!(
        wrap.is_multiple_of(32),
        "line width must be 0 or a multiple of 32"
    );
    let mut core = driver::EncodeCore::new(effective_level(max_level), wrap);
    let mut out = Vec::with_capacity(driver::EncodeCore::capacity(input.len(), wrap));
    let spare = out.spare_capacity_mut();
    let n = core.encode(input, spare);
    let n = n + core.finish(&mut spare[n..]);
    // SAFETY: the core initialized the first `n` bytes of the spare capacity.
    unsafe { out.set_len(n) };
    out
}

/// Decode `input`, auto-detecting the format (spec 0008 §2).
///
/// After leading whitespace, a `-` indicates a SIMD fixed-width stream,
/// decoded strictly (spec 0008 §3.5); anything else is decoded as a Henke
/// stream. Returns `None` for an invalid SIMD stream. `max_level` caps the
/// SIMD kernel used.
#[cfg(feature = "alloc")]
pub fn decode(input: &[u8], max_level: SimdLevel) -> Option<Vec<u8>> {
    let mut decoder = Decoder::new(max_level);
    let mut out = Vec::new();
    (decoder.decode_inner(input, &mut out) && decoder.finish_inner(&mut out)).then_some(out)
}

/// Encode `input` into a caller-provided buffer without bounds checking.
/// Returns the number of bytes written (including the `-` prefix).
///
/// Writes exactly what [`encode`] returns for the same arguments; empty
/// input writes nothing and returns 0. It currently encodes into a
/// temporary `Vec` and copies the result (one allocation per call); spec
/// 0010 replaces this function with a slice-based API.
///
/// # Safety
/// `output` must point to at least `encode_size_hint(input.len(), wrap)`
/// writable bytes.
#[cfg(feature = "alloc")]
pub unsafe fn encode_unchecked(
    input: &[u8],
    max_level: SimdLevel,
    wrap: usize,
    output: *mut u8,
) -> usize {
    let encoded = encode(input, max_level, wrap);
    debug_assert!(encoded.len() <= encode_size_hint(input.len(), wrap));
    // SAFETY: the caller guarantees `output` has at least
    // `encode_size_hint(input.len(), wrap)` writable bytes, and the size
    // hint is an upper bound on `encode`'s output (tests/unchecked.rs).
    core::ptr::copy_nonoverlapping(encoded.as_ptr(), output, encoded.len());
    encoded.len()
}

/// Decode into a caller-provided buffer without bounds checking.
///
/// Auto-detects format: a leading `-` selects the SIMD path; otherwise falls
/// back to Henke.  Returns bytes written.
///
/// Writes exactly what [`decode`] returns for the same arguments; empty
/// input writes nothing and returns 0. It currently decodes into a
/// temporary `Vec` and copies the result (one allocation per call); spec
/// 0010 replaces this function with a slice-based API.
///
/// # Safety
/// `output` must point to at least `decode_size_hint(input.len())`
/// writable bytes.
#[cfg(feature = "alloc")]
pub unsafe fn decode_unchecked(input: &[u8], max_level: SimdLevel, output: *mut u8) -> usize {
    let decoded = decode(input, max_level).unwrap_or_default();
    debug_assert!(decoded.len() <= decode_size_hint(input.len()));
    // SAFETY: as in `encode_unchecked`, with `decode_size_hint`.
    core::ptr::copy_nonoverlapping(decoded.as_ptr(), output, decoded.len());
    decoded.len()
}

// ---------------------------------------------------------------------------
// Streaming encoder
// ---------------------------------------------------------------------------

/// Stateful SIMD fixed-width encoder (no line wrapping).
///
/// Feed input with [`Encoder::encode`]; flush with [`Encoder::finish`].
/// The `-` prefix is written with the first non-empty input, so an encoder
/// that only sees empty input produces the empty stream, like [`encode`].
#[cfg(feature = "alloc")]
pub struct Encoder {
    core: driver::EncodeCore,
}

#[cfg(feature = "alloc")]
impl Encoder {
    pub fn new(max_level: SimdLevel) -> Self {
        Self {
            core: driver::EncodeCore::new(effective_level(max_level), 0),
        }
    }

    /// Encode `input`, appending SIMD-variant characters to `output`.
    /// Writes the `-` prefix with the first non-empty input.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        output.reserve(driver::EncodeCore::capacity(input.len(), 0));
        let n = self.core.encode(input, output.spare_capacity_mut());
        // SAFETY: the core initialized the first `n` bytes of the spare
        // capacity.
        unsafe { output.set_len(output.len() + n) };
    }

    /// Flush remaining bits (0–2 chars).
    pub fn finish(mut self, output: &mut Vec<u8>) {
        output.reserve(driver::EncodeCore::capacity(0, 0));
        let n = self.core.finish(output.spare_capacity_mut());
        // SAFETY: as in `encode`.
        unsafe { output.set_len(output.len() + n) };
    }
}

#[cfg(feature = "alloc")]
impl Default for Encoder {
    fn default() -> Self {
        Self::new(SimdLevel::default())
    }
}

// ---------------------------------------------------------------------------
// Streaming decoder
// ---------------------------------------------------------------------------

/// Stateful decoder. Auto-detects the format (spec 0008 §2): after leading
/// whitespace, `-` selects the SIMD variant, decoded strictly; anything else
/// the Henke format.
#[cfg(feature = "alloc")]
pub struct Decoder {
    max_level: SimdLevel,
    mode: DecoderMode,
}

#[cfg(feature = "alloc")]
enum DecoderMode {
    /// Only whitespace so far; `offset` bytes of it.
    Undecided {
        offset: usize,
    },
    Simd(driver::DecodeCore),
    Henke(crate::Decoder),
}

#[cfg(feature = "alloc")]
impl Decoder {
    pub fn new(max_level: SimdLevel) -> Self {
        Self {
            max_level,
            mode: DecoderMode::Undecided { offset: 0 },
        }
    }

    /// Decode `input`, appending bytes to `output`.
    /// Returns `false` once the SIMD stream is invalid (spec 0008 §3.5).
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) -> bool {
        self.decode_inner(input, output)
    }

    /// Flush the last bytes. An invalid SIMD stream (including an invalid
    /// payload length, only known here) adds nothing.
    pub fn finish(mut self, output: &mut Vec<u8>) {
        self.finish_inner(output);
    }

    fn decode_inner(&mut self, input: &[u8], output: &mut Vec<u8>) -> bool {
        let mut input = input;
        if let DecoderMode::Undecided { offset } = self.mode {
            let Some(p) = input.iter().position(|&b| !driver::is_whitespace(b)) else {
                self.mode = DecoderMode::Undecided {
                    offset: offset + input.len(),
                };
                return true;
            };
            if input[p] == b'-' {
                let level = effective_level(self.max_level);
                self.mode = DecoderMode::Simd(driver::DecodeCore::new(level, offset + p + 1));
                input = &input[p + 1..];
            } else {
                self.mode = DecoderMode::Henke(crate::Decoder::new());
                input = &input[p..];
            }
        }
        match &mut self.mode {
            DecoderMode::Undecided { .. } => true,
            DecoderMode::Simd(core) => {
                output.reserve(driver::DecodeCore::capacity(input.len()) + 16);
                match core.decode(input, output.spare_capacity_mut()) {
                    Ok(n) => {
                        // SAFETY: the core initialized the first `n` bytes of
                        // the spare capacity.
                        unsafe { output.set_len(output.len() + n) };
                        true
                    }
                    Err(_) => false,
                }
            }
            DecoderMode::Henke(decoder) => {
                decoder.decode(input, output);
                true
            }
        }
    }

    fn finish_inner(&mut self, output: &mut Vec<u8>) -> bool {
        match &mut self.mode {
            DecoderMode::Undecided { .. } => true,
            DecoderMode::Simd(core) => {
                output.reserve(driver::DecodeCore::FINISH_CAPACITY);
                match core.finish(output.spare_capacity_mut()) {
                    Ok(n) => {
                        // SAFETY: as in `decode_inner`.
                        unsafe { output.set_len(output.len() + n) };
                        true
                    }
                    Err(_) => false,
                }
            }
            DecoderMode::Henke(decoder) => {
                core::mem::take(decoder).finish(output);
                true
            }
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for Decoder {
    fn default() -> Self {
        Self::new(SimdLevel::default())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "alloc"))]
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
    fn non_simd_input_falls_back_to_henke() {
        // Input without '-' prefix is treated as a Henke stream.
        let henke_encoded = crate::encode(b"test");
        let decoded =
            decode(&henke_encoded, SimdLevel::default()).expect("Henke fallback must succeed");
        assert_eq!(decoded, b"test");
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
            let mut enc = Encoder::new(SimdLevel::default());
            let mut out = Vec::new();
            for chunk in input.chunks(chunk_size) {
                enc.encode(chunk, &mut out);
            }
            enc.finish(&mut out);
            assert_eq!(out, reference, "chunk_size={chunk_size}");
        }
    }

    // -----------------------------------------------------------------------
    // Kernel validation (spec 0010 §5, §7)
    // -----------------------------------------------------------------------

    /// A batch decode kernel: input, stride, output, blocks (or units) per
    /// line, lines.
    type BlockFn = unsafe fn(*const u8, usize, *mut u8, usize, usize) -> bool;

    /// The decode block kernels this CPU can run, with their input width.
    fn decode_kernels() -> Vec<(&'static str, usize, BlockFn)> {
        let candidates: [(&'static str, usize, BlockFn, bool); ARCH_KERNELS] = [
            #[cfg(target_arch = "x86_64")]
            (
                "sse41",
                16,
                x86::decode_lines_sse41,
                std::is_x86_feature_detected!("sse4.1") && std::is_x86_feature_detected!("ssse3"),
            ),
            #[cfg(target_arch = "x86_64")]
            (
                "avx2",
                32,
                x86::decode_lines_avx2,
                std::is_x86_feature_detected!("avx2"),
            ),
            #[cfg(target_arch = "aarch64")]
            (
                "neon",
                16,
                aarch64::decode_lines_neon,
                std::arch::is_aarch64_feature_detected!("neon"),
            ),
        ];
        candidates
            .into_iter()
            .filter(|&(_, _, _, available)| available)
            .map(|(name, width, kernel, _)| (name, width, kernel))
            .collect()
    }

    /// Number of decode kernels compiled for this architecture.
    const ARCH_KERNELS: usize = if cfg!(target_arch = "x86_64") {
        2
    } else if cfg!(target_arch = "aarch64") {
        1
    } else {
        0
    };

    fn in_alphabet(b: u8) -> bool {
        matches!(b, 0x23..=0x26 | 0x28..=0x7E)
    }

    /// Runs `kernel` on one block (or AVX2 unit): the first `width` bytes of
    /// `block`.
    fn run(kernel: BlockFn, block: &[u8]) -> bool {
        // The kernels store up to 32 bytes; the output is not inspected.
        let mut out = [0u8; 64];
        // SAFETY: the caller detected the kernel's CPU features;
        // `block` holds the kernel's input width, `out` more than it writes.
        unsafe { kernel(block.as_ptr(), 0, out.as_mut_ptr(), 1, 1) }
    }

    /// Two characters encoding the pair value `v` (low index first).
    fn pair(v: u32) -> [u8; 2] {
        [
            scalar::enc_char((v % 91) as u8),
            scalar::enc_char((v / 91) as u8),
        ]
    }

    #[test]
    fn kernels_reject_every_byte_outside_the_alphabet() {
        let input: Vec<u8> = (0u8..=255).cycle().take(26).collect();
        let valid = encode(&input, SimdLevel::Scalar, 0);
        for (name, width, kernel) in decode_kernels() {
            let block = &valid[1..1 + width];
            assert!(run(kernel, block), "{name}: valid block rejected");
            for pos in 0..width {
                for b in (0u8..=255).filter(|&b| !in_alphabet(b)) {
                    let mut bad = block.to_vec();
                    bad[pos] = b;
                    assert!(
                        !run(kernel, &bad),
                        "{name}: byte {b:#04x} at {pos} accepted"
                    );
                }
            }
        }
    }

    #[test]
    fn kernels_reject_pairs_above_8191() {
        let input: Vec<u8> = (0u8..=255).cycle().take(26).collect();
        let valid = encode(&input, SimdLevel::Scalar, 0);
        for (name, width, kernel) in decode_kernels() {
            let block = &valid[1..1 + width];
            for k in 0..width / 2 {
                let mut bad = block.to_vec();
                for v in 0..=90 + 90 * 91 {
                    bad[2 * k..2 * k + 2].copy_from_slice(&pair(v));
                    let accepted = run(kernel, &bad);
                    assert!(accepted == (v <= 8191), "{name}: pair {k} worth {v}");
                }
            }
        }
    }
}
