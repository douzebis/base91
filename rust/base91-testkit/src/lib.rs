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
//! And the shared helpers of the differential tests (spec 0009 §2–§4):
//! [`pending`] (known deviations), [`rng`] (reproducible randomness),
//! [`canary`] (guarded output buffers), [`mutate`] (damaged streams),
//! [`reader`] (reads in chosen pieces, like a pipe).
//!
//! This crate is a dev-dependency only; it is never published.

pub mod canary;
pub mod mutate;
pub mod oracle;
pub mod pending;
pub mod reader;
pub mod rng;

#[cfg(feature = "reference")]
pub mod reference;

/// Whether the upstream reference is compiled in (feature `reference`).
/// Tests that need it check this and skip, with a message, when it is off.
pub const REFERENCE_AVAILABLE: bool = cfg!(feature = "reference");

/// Path of upstream's own command-line tool (`cli.c`), built by `build.rs`
/// with feature `reference` (spec 0009 §9); `None` without it. It picks its
/// mode from its name, like ours: `b91enc`, `b91dec`, or anything else for
/// `base91`.
#[cfg(feature = "reference")]
pub const REFERENCE_CLI: Option<&str> = Some(env!("BASE91_REFERENCE_CLI"));
/// See the `reference` variant.
#[cfg(not(feature = "reference"))]
pub const REFERENCE_CLI: Option<&str> = None;
