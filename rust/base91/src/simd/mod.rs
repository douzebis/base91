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

#[cfg(target_arch = "x86_64")]
pub(crate) mod x86;

#[cfg(target_arch = "aarch64")]
pub(crate) mod aarch64;

mod driver;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::mem::MaybeUninit;

use crate::Error;

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

/// Characters for the tail of `n mod 13` input bytes (spec 0008 §3.3).
const TAIL_CHARS: [usize; 13] = [0, 2, 3, 4, 5, 7, 8, 9, 10, 12, 13, 14, 15];

/// Checks a line width: 0 (unwrapped) or a multiple of 32 (spec 0008 §3.4).
fn check_width(wrap: usize) -> Result<(), Error> {
    if wrap.is_multiple_of(32) {
        Ok(())
    } else {
        Err(Error::InvalidWidth { width: wrap })
    }
}

/// Exact length E(n, W) of the stream encoding `n` bytes with line width
/// `wrap` (spec 0008 §3.4), final LF included.
pub fn encoded_len(n: usize, wrap: usize) -> Result<usize, Error> {
    check_width(wrap)?;
    if n == 0 {
        return Ok(0);
    }
    let payload = 16 * (n / 13) + TAIL_CHARS[n % 13];
    Ok(if wrap == 0 {
        1 + payload
    } else {
        2 + payload + payload.div_ceil(wrap)
    })
}

/// Encode `input` using the SIMD fixed-width variant (spec 0008 §3).
///
/// `wrap` is 0 (no line wrapping) or a multiple of 32: the payload is then
/// written in lines of `wrap` characters, each followed by `\n`.
/// `max_level` caps the SIMD kernel used; [`SimdLevel::default`] uses the
/// best available.
#[cfg(feature = "alloc")]
pub fn encode(input: &[u8], max_level: SimdLevel, wrap: usize) -> Result<Vec<u8>, Error> {
    let len = encoded_len(input.len(), wrap)?;
    let mut out = Vec::with_capacity(len);
    let n = encode_uninit(input, max_level, wrap, &mut out.spare_capacity_mut()[..len]);
    // SAFETY: `encode_uninit` initialized the first `n` bytes of the spare
    // capacity.
    unsafe { out.set_len(n) };
    Ok(out)
}

/// Encode `input` into `output`, and return the number of bytes written.
///
/// `output` must hold at least [`encoded_len`]`(input.len(), wrap)` bytes;
/// otherwise nothing is written and the error gives that size. Needs no
/// allocator.
pub fn encode_slice(
    input: &[u8],
    max_level: SimdLevel,
    wrap: usize,
    output: &mut [u8],
) -> Result<usize, Error> {
    let needed = encoded_len(input.len(), wrap)?;
    if output.len() < needed {
        return Err(Error::BufferTooSmall { needed });
    }
    Ok(encode_uninit(
        input,
        max_level,
        wrap,
        &mut crate::as_uninit(output)[..needed],
    ))
}

/// One-shot encoding into `out`, which holds `encoded_len(input.len(),
/// wrap)` bytes (`wrap` already checked).
fn encode_uninit(
    input: &[u8],
    max_level: SimdLevel,
    wrap: usize,
    out: &mut [MaybeUninit<u8>],
) -> usize {
    let mut core = driver::EncodeCore::new(effective_level(max_level), wrap);
    let n = core.encode(input, out);
    n + core.finish(&mut out[n..])
}

/// Decode `input`, auto-detecting the format (spec 0008 §2).
///
/// After leading whitespace, a `-` indicates a SIMD fixed-width stream,
/// decoded strictly (spec 0008 §3.5); anything else is decoded as a Henke
/// stream, which never fails. `max_level` caps the SIMD kernel used.
#[cfg(feature = "alloc")]
pub fn decode(input: &[u8], max_level: SimdLevel) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(decode_size_hint(input.len()));
    let spare = out.spare_capacity_mut();
    let n = decode_uninit(input, max_level, spare)?;
    // SAFETY: `decode_uninit` initialized the first `n` bytes of the spare
    // capacity.
    unsafe { out.set_len(n) };
    Ok(out)
}

/// Decode `input` into `output`, auto-detecting the format as `decode`
/// does, and return the number of bytes written.
///
/// `output` must hold at least [`decode_size_hint`]`(input.len())` bytes;
/// otherwise nothing is written and the error gives that size. After a
/// decoding error, the contents of `output` are unspecified. Needs no
/// allocator.
pub fn decode_slice(input: &[u8], max_level: SimdLevel, output: &mut [u8]) -> Result<usize, Error> {
    let needed = decode_size_hint(input.len());
    if output.len() < needed {
        return Err(Error::BufferTooSmall { needed });
    }
    decode_uninit(input, max_level, crate::as_uninit(output))
}

/// One-shot decoding into `out`, which holds at least
/// `decode_size_hint(input.len())` bytes: never less than the streaming
/// decoder's bounds for a whole stream.
fn decode_uninit(
    input: &[u8],
    max_level: SimdLevel,
    out: &mut [MaybeUninit<u8>],
) -> Result<usize, Error> {
    let mut decoder = Decoder::new(max_level);
    let n = decoder.decode_uninit(input, out)?;
    Ok(n + decoder.finish_uninit(&mut out[n..])?)
}

/// Encode `input` into a caller-provided buffer without bounds checking.
/// Returns the number of bytes written (including the `-` prefix).
///
/// Writes exactly what [`encode_slice`] writes for the same arguments.
///
/// # Panics
/// If `wrap` is neither 0 nor a multiple of 32.
///
/// # Safety
/// `output` must point to at least `encode_size_hint(input.len(), wrap)`
/// writable bytes.
#[deprecated(since = "0.3.0", note = "use `simd::encode_slice`")]
pub unsafe fn encode_unchecked(
    input: &[u8],
    max_level: SimdLevel,
    wrap: usize,
    output: *mut u8,
) -> usize {
    let len = match encoded_len(input.len(), wrap) {
        Ok(len) => len,
        Err(e) => panic!("{e}"),
    };
    debug_assert!(len <= encode_size_hint(input.len(), wrap));
    // SAFETY: the caller guarantees `encode_size_hint(input.len(), wrap)`
    // writable bytes at `output`, an upper bound on `encoded_len` (tested);
    // they are only written.
    let out = unsafe { core::slice::from_raw_parts_mut(output.cast::<MaybeUninit<u8>>(), len) };
    encode_uninit(input, max_level, wrap, out)
}

/// Decode into a caller-provided buffer without bounds checking.
///
/// Auto-detects the format as [`decode_slice`] does. Returns the number of bytes
/// written, or 0 for an invalid SIMD stream: use [`decode_slice`] to tell
/// errors apart.
///
/// # Safety
/// `output` must point to at least `decode_size_hint(input.len())`
/// writable bytes.
#[deprecated(since = "0.3.0", note = "use `simd::decode_slice`")]
pub unsafe fn decode_unchecked(input: &[u8], max_level: SimdLevel, output: *mut u8) -> usize {
    // SAFETY: the caller guarantees `decode_size_hint(input.len())` writable
    // bytes at `output`; they are only written.
    let out = unsafe {
        core::slice::from_raw_parts_mut(
            output.cast::<MaybeUninit<u8>>(),
            decode_size_hint(input.len()),
        )
    };
    decode_uninit(input, max_level, out).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Streaming encoder
// ---------------------------------------------------------------------------

/// Stateful SIMD fixed-width encoder, with line wrapping (spec 0008 §3.4,
/// §3.6).
///
/// Feed input with `Encoder::encode` (or [`Encoder::encode_slice`]);
/// flush with `Encoder::finish` (or [`Encoder::finish_slice`]). The
/// output is exactly [`encode_slice`]'s, however the input is split. The `-`
/// prefix is written with the first non-empty input, so an encoder that
/// only sees empty input produces the empty stream.
#[derive(Clone, Debug)]
pub struct Encoder {
    core: driver::EncodeCore,
}

impl Encoder {
    /// An encoder with line width `wrap`: 0, or a multiple of 32.
    pub fn new(max_level: SimdLevel, wrap: usize) -> Result<Self, Error> {
        check_width(wrap)?;
        Ok(Self {
            core: driver::EncodeCore::new(effective_level(max_level), wrap),
        })
    }

    /// Encode `input`, appending SIMD-variant characters to `output`.
    #[cfg(feature = "alloc")]
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        let len = self.core.encode_len(input.len());
        output.reserve(len);
        let n = self
            .core
            .encode(input, &mut output.spare_capacity_mut()[..len]);
        // SAFETY: the core initialized the first `n` bytes of the spare
        // capacity.
        unsafe { output.set_len(output.len() + n) };
    }

    /// Encode `input` into `output`, and return the number of bytes written.
    ///
    /// `output` must hold the bytes this call writes (an exact size, known
    /// from the state); otherwise nothing is written, the state is kept, and
    /// the error gives the size.
    pub fn encode_slice(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let needed = self.core.encode_len(input.len());
        if output.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        Ok(self
            .core
            .encode(input, &mut crate::as_uninit(output)[..needed]))
    }

    /// Flush the last characters and the final LF.
    #[cfg(feature = "alloc")]
    pub fn finish(mut self, output: &mut Vec<u8>) {
        let len = self.core.finish_len();
        output.reserve(len);
        let n = self.core.finish(&mut output.spare_capacity_mut()[..len]);
        // SAFETY: as in `encode`.
        unsafe { output.set_len(output.len() + n) };
    }

    /// Flush the last characters and the final LF into `output`, return the
    /// number of bytes written, and reset the encoder.
    ///
    /// `output` must hold the bytes to write; otherwise nothing is written,
    /// the state is kept, and the error gives the size.
    pub fn finish_slice(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let needed = self.core.finish_len();
        if output.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        Ok(self.core.finish(&mut crate::as_uninit(output)[..needed]))
    }
}

// ---------------------------------------------------------------------------
// Streaming decoder
// ---------------------------------------------------------------------------

/// Stateful decoder. Auto-detects the format (spec 0008 §2): after leading
/// whitespace, `-` selects the SIMD variant, decoded strictly (spec 0008
/// §3.5); anything else the Henke format, which never fails.
///
/// The result is exactly [`decode_slice`]'s, however the input is split: the same
/// bytes, or the same error. Once it has reported an error, the decoder
/// reports it again on every call.
#[derive(Clone, Debug)]
pub struct Decoder {
    max_level: SimdLevel,
    mode: DecoderMode,
}

#[derive(Clone, Debug)]
enum DecoderMode {
    /// Only whitespace so far; `offset` bytes of it.
    Undecided {
        offset: usize,
    },
    Simd(driver::DecodeCore),
    Henke(crate::Decoder),
}

impl Decoder {
    pub fn new(max_level: SimdLevel) -> Self {
        Self {
            max_level,
            mode: DecoderMode::Undecided { offset: 0 },
        }
    }

    /// Decode `input`, appending bytes to `output`.
    #[cfg(feature = "alloc")]
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) -> Result<(), Error> {
        output.reserve(self.decode_len(input.len()));
        let n = self.decode_uninit(input, output.spare_capacity_mut())?;
        // SAFETY: `decode_uninit` initialized the first `n` bytes of the
        // spare capacity.
        unsafe { output.set_len(output.len() + n) };
        Ok(())
    }

    /// Decode `input` into `output`, and return the number of bytes written.
    ///
    /// `output` must hold at least `decode_size_hint(input.len() + 15)`
    /// bytes (a smaller bound, known from the state, is enough; the error
    /// gives it); otherwise nothing is written and the state is kept. After a
    /// decoding error, the contents of `output` are unspecified.
    pub fn decode_slice(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let needed = self.decode_len(input.len());
        if output.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        self.decode_uninit(input, crate::as_uninit(output))
    }

    /// Check the end of the stream (the last line, the payload length) and
    /// flush the last bytes.
    #[cfg(feature = "alloc")]
    pub fn finish(mut self, output: &mut Vec<u8>) -> Result<(), Error> {
        output.reserve(self.finish_len()?);
        let n = self.finish_uninit(output.spare_capacity_mut())?;
        // SAFETY: as in `decode`.
        unsafe { output.set_len(output.len() + n) };
        Ok(())
    }

    /// Check the end of the stream and flush the last bytes into `output`;
    /// on success, return the number of bytes written and reset the decoder.
    ///
    /// `output` must hold the bytes to write; otherwise nothing is written,
    /// the state is kept, and the error gives the size.
    pub fn finish_slice(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let needed = self.finish_len()?;
        if output.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        self.finish_uninit(crate::as_uninit(output))
    }

    /// Upper bound on what one `decode` call writes for `n` input bytes.
    fn decode_len(&self, n: usize) -> usize {
        match &self.mode {
            // Whatever the format turns out to be, a fresh decoder writes
            // at most this.
            DecoderMode::Undecided { .. } => decode_size_hint(n),
            DecoderMode::Simd(core) => core.decode_len(n),
            DecoderMode::Henke(decoder) => decoder.decode_len(n),
        }
    }

    /// Exactly what `finish` writes, or the error it reports.
    fn finish_len(&self) -> Result<usize, Error> {
        match &self.mode {
            DecoderMode::Undecided { .. } => Ok(0),
            DecoderMode::Simd(core) => core.finish_len(),
            DecoderMode::Henke(decoder) => Ok(decoder.finish_len()),
        }
    }

    /// Decoding core: `out` holds at least `decode_len(input.len())` bytes.
    fn decode_uninit(&mut self, input: &[u8], out: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
        let mut input = input;
        if let DecoderMode::Undecided { offset } = self.mode {
            let Some(p) = input.iter().position(|&b| !driver::is_whitespace(b)) else {
                self.mode = DecoderMode::Undecided {
                    offset: offset + input.len(),
                };
                return Ok(0);
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
            DecoderMode::Undecided { .. } => Ok(0),
            DecoderMode::Simd(core) => core.decode(input, out),
            DecoderMode::Henke(decoder) => Ok(decoder.decode_uninit(input, out)),
        }
    }

    /// Finishing core: `out` holds at least `finish_len()` bytes. Resets the
    /// decoder on success.
    fn finish_uninit(&mut self, out: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
        let n = match &mut self.mode {
            DecoderMode::Undecided { .. } => 0,
            DecoderMode::Simd(core) => core.finish(out)?,
            DecoderMode::Henke(decoder) => decoder.finish_uninit(out),
        };
        *self = Self::new(self.max_level);
        Ok(n)
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

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    fn round_trip(input: &[u8]) -> Vec<u8> {
        let encoded = encode(input, SimdLevel::default(), 0).unwrap();
        if !encoded.is_empty() {
            assert_eq!(encoded[0], b'-');
        }
        decode(&encoded, SimdLevel::default()).expect("decode failed")
    }

    #[test]
    fn empty() {
        assert_eq!(encode(b"", SimdLevel::default(), 0), Ok(vec![]));
        assert_eq!(decode(b"", SimdLevel::default()), Ok(vec![]));
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
    fn all_levels_round_trip() {
        let input: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        for level in [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256] {
            let encoded = encode(&input, level, 0).unwrap();
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
        let simd = encode(input, SimdLevel::default(), 0).unwrap();
        let henke = crate::encode(input);
        assert_ne!(&simd[1..], henke.as_slice());
        assert_eq!(simd[0], b'-');
        assert_ne!(henke[0], b'-');
    }

    #[test]
    fn encoder_chunk_boundaries() {
        let input: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        for wrap in [0, 64] {
            let reference = encode(&input, SimdLevel::default(), wrap).unwrap();
            for chunk_size in [1, 2, 7, 13, 14, 25, 26, 27, 64, 100, 256] {
                let mut enc = Encoder::new(SimdLevel::default(), wrap).unwrap();
                let mut out = Vec::new();
                for chunk in input.chunks(chunk_size) {
                    enc.encode(chunk, &mut out);
                }
                enc.finish(&mut out);
                assert_eq!(out, reference, "wrap={wrap}, chunk_size={chunk_size}");
            }
        }
    }

    #[test]
    fn invalid_widths_are_rejected() {
        for wrap in [1, 16, 48, 76] {
            let err = Error::InvalidWidth { width: wrap };
            assert_eq!(encode(b"x", SimdLevel::default(), wrap), Err(err));
            assert_eq!(encoded_len(1, wrap), Err(err));
            assert_eq!(
                encode_slice(b"x", SimdLevel::default(), wrap, &mut [0; 64]),
                Err(err)
            );
            assert!(Encoder::new(SimdLevel::default(), wrap).is_err());
        }
    }

    #[test]
    fn slice_api_matches_vec_api() {
        let input: Vec<u8> = (0u8..=255).cycle().take(3000).collect();
        for wrap in [0, 32, 64] {
            let encoded = encode(&input, SimdLevel::default(), wrap).unwrap();
            let mut buf = vec![0u8; encoded_len(input.len(), wrap).unwrap()];
            let n = encode_slice(&input, SimdLevel::default(), wrap, &mut buf).unwrap();
            assert_eq!(&buf[..n], &encoded[..]);

            let mut back = vec![0u8; decode_size_hint(encoded.len())];
            let n = decode_slice(&encoded, SimdLevel::default(), &mut back).unwrap();
            assert_eq!(&back[..n], &input[..]);
        }
    }

    #[test]
    fn buffer_too_small_writes_nothing() {
        let input = b"Hello, world!";
        let needed = encoded_len(input.len(), 0).unwrap();
        let mut buf = vec![0xA5u8; needed - 1];
        assert_eq!(
            encode_slice(input, SimdLevel::default(), 0, &mut buf),
            Err(Error::BufferTooSmall { needed })
        );
        assert!(buf.iter().all(|&b| b == 0xA5));

        let encoded = encode(input, SimdLevel::default(), 0).unwrap();
        let needed = decode_size_hint(encoded.len());
        let mut buf = vec![0xA5u8; needed - 1];
        assert_eq!(
            decode_slice(&encoded, SimdLevel::default(), &mut buf),
            Err(Error::BufferTooSmall { needed })
        );
        assert!(buf.iter().all(|&b| b == 0xA5));
    }

    #[test]
    fn streaming_slice_api_keeps_state_on_buffer_too_small() {
        let input: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        let reference = encode(&input, SimdLevel::default(), 64).unwrap();

        let mut enc = Encoder::new(SimdLevel::default(), 64).unwrap();
        let mut out = Vec::new();
        for chunk in input.chunks(100) {
            // Too small first: nothing changes, then the right size.
            assert!(matches!(
                enc.encode_slice(chunk, &mut []),
                Err(Error::BufferTooSmall { .. }) | Ok(0)
            ));
            let mut buf = [0u8; 256];
            let n = enc.encode_slice(chunk, &mut buf).unwrap();
            out.extend_from_slice(&buf[..n]);
        }
        let mut buf = [0u8; 8];
        let n = enc.finish_slice(&mut buf).unwrap();
        out.extend_from_slice(&buf[..n]);
        assert_eq!(out, reference);

        let mut dec = Decoder::new(SimdLevel::default());
        let mut back = Vec::new();
        for chunk in reference.chunks(37) {
            if let Err(Error::BufferTooSmall { needed }) = dec.decode_slice(chunk, &mut []) {
                assert!(needed > 0);
            }
            let mut buf = [0u8; 64];
            let n = dec.decode_slice(chunk, &mut buf).unwrap();
            back.extend_from_slice(&buf[..n]);
        }
        let mut buf = [0u8; 16];
        let n = dec.finish_slice(&mut buf).unwrap();
        back.extend_from_slice(&buf[..n]);
        assert_eq!(back, input);
    }

    #[test]
    fn decoder_reports_the_error_again() {
        let mut dec = Decoder::new(SimdLevel::default());
        let mut out = Vec::new();
        let err = Err(Error::InvalidCharacter { offset: 3 });
        assert_eq!(dec.decode(b"-##\"##", &mut out), err);
        assert_eq!(dec.decode(b"####", &mut out), err);
        assert_eq!(dec.finish(&mut out), err);
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
            driver::ALPHABET[(v % 91) as usize],
            driver::ALPHABET[(v / 91) as usize],
        ]
    }

    #[test]
    fn kernels_reject_every_byte_outside_the_alphabet() {
        let input: Vec<u8> = (0u8..=255).cycle().take(26).collect();
        let valid = encode(&input, SimdLevel::Scalar, 0).unwrap();
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
        let valid = encode(&input, SimdLevel::Scalar, 0).unwrap();
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
