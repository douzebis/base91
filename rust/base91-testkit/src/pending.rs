// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Known deviations of `base91-rs` from spec 0008, each with the spec that
//! removes it (spec 0009, decision 1).
//!
//! Tests call [`is_pending`] and skip exactly the comparisons a pending
//! deviation would fail. Implementing a fix means deleting its entry from
//! [`PENDING`], which switches those comparisons on. Nothing else is
//! skipped.

/// A known deviation. Variants stay after their fix (tests keep naming
/// them); only [`PENDING`] changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deviation {
    /// `simd::encode_unchecked` / `decode_unchecked` corrupt the caller's
    /// memory. Tests must not call them at all. Fixed by spec 0005.
    SimdUnchecked,
    /// `simd::Decoder` (and `io::DecoderReader` on SIMD streams) lose state
    /// at chunk boundaries. Fixed by spec 0006, bug 1.
    SimdStreamingDecoder,
    /// Wrapped SIMD encoding: no final LF (spec 0008 §3.4), and corrupt
    /// output when the payload fills its last line. Fixed by spec 0006,
    /// bug 3.
    SimdWrappedEncoding,
    /// `simd::Encoder` turns an empty stream into `-`. Fixed by spec 0006,
    /// bug 4.
    SimdStreamingEncoderEmpty,
    /// The legacy `Decoder` can panic when fed very small chunks (its
    /// reservation ignores a pending character). Fixed by spec 0006, bug 5.
    LegacyStreamingDecoder,
    /// `io::DecoderReader` returns end of file early when a read from its
    /// inner reader decodes to nothing. Fixed by spec 0006, bug 6.
    IoReaderEarlyEof,
    /// SIMD decoders silently return wrong data for invalid input (or panic
    /// on an arithmetic overflow when overflow checks are on), and
    /// mishandle unusual but valid forms (CR LF, whitespace before `-`).
    /// Fixed by the strict-decoder spec (spec 0008, annex A9–A13b; not
    /// written yet).
    SimdStrictDecoding,
}

/// Deviations not fixed yet.
pub const PENDING: &[Deviation] = &[
    Deviation::SimdUnchecked,
    Deviation::SimdStreamingDecoder,
    Deviation::SimdWrappedEncoding,
    Deviation::SimdStreamingEncoderEmpty,
    Deviation::LegacyStreamingDecoder,
    Deviation::IoReaderEarlyEof,
    Deviation::SimdStrictDecoding,
];

/// Whether `deviation` is still pending.
pub fn is_pending(deviation: Deviation) -> bool {
    PENDING.contains(&deviation)
}
