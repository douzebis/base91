// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! The error type of the fallible functions (spec 0010 §1).

use core::fmt;

/// Why an encode or decode call failed.
///
/// The decoding classes are those of spec 0008 §3.5. Their `offset` is the
/// position, in the whole input (or stream, when streaming), that spec 0008
/// §3.5 "Which error is reported" assigns to the first error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The output buffer is shorter than the size this call requires.
    BufferTooSmall {
        /// The size required.
        needed: usize,
    },
    /// The line width is neither 0 nor a multiple of 32 (spec 0008 §3.4).
    InvalidWidth {
        /// The width given.
        width: usize,
    },
    /// A byte that is neither an alphabet character nor part of a valid
    /// line terminator.
    InvalidCharacter {
        /// Position of the byte.
        offset: usize,
    },
    /// A terminator where none belongs, mixed terminators, or lines of the
    /// wrong length.
    InvalidLayout {
        /// Position of the first byte that breaks the layout.
        offset: usize,
    },
    /// A payload length L with L mod 16 in {1, 6, 11}.
    InvalidLength,
    /// A character pair worth more than 8191.
    InvalidValue {
        /// Position of the pair's first character.
        offset: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::BufferTooSmall { needed } => {
                write!(f, "output buffer too small ({needed} bytes needed)")
            }
            Error::InvalidWidth { width } => {
                write!(
                    f,
                    "invalid line width {width} (must be 0 or a multiple of 32)"
                )
            }
            Error::InvalidCharacter { offset } => write!(f, "invalid character at offset {offset}"),
            Error::InvalidLayout { offset } => write!(f, "invalid line layout at offset {offset}"),
            Error::InvalidLength => write!(f, "invalid payload length"),
            Error::InvalidValue { offset } => {
                write!(f, "invalid character pair at offset {offset}")
            }
        }
    }
}

impl core::error::Error for Error {}
