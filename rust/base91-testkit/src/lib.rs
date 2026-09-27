// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Test oracles for `base91-rs` (spec 0009 §1).
//!
//! - [`oracle`]: a bit-by-bit model of the SIMD wire format, written from
//!   spec 0008 alone and sharing no code with the library.
//! - `reference` (feature `reference`): the vendored, unmodified upstream
//!   basE91 0.6.0 C implementation, the oracle for the legacy format.
//!
//! This crate is a dev-dependency only; it is never published.

pub mod oracle;

#[cfg(feature = "reference")]
pub mod reference;

/// Whether the upstream reference is compiled in (feature `reference`).
/// Tests that need it check this and skip, with a message, when it is off.
pub const REFERENCE_AVAILABLE: bool = cfg!(feature = "reference");
