// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Output buffers guarded by canary bytes (spec 0009 §2, §7).
//!
//! For APIs that write through a raw pointer into caller memory: the buffer
//! is exactly the documented size, surrounded on both sides by
//! [`CANARY_LEN`] bytes of [`CANARY`]. Any write outside the documented size
//! changes a canary, which [`Guarded::check`] reports.

/// Value of every canary byte.
pub const CANARY: u8 = 0xA5;
/// Number of canary bytes on each side.
pub const CANARY_LEN: usize = 64;

/// A heap buffer of `len` usable bytes between two canary zones.
pub struct Guarded {
    storage: Vec<u8>,
    len: usize,
}

impl Guarded {
    pub fn new(len: usize) -> Self {
        Self {
            storage: vec![CANARY; len + 2 * CANARY_LEN],
            len,
        }
    }

    /// Pointer to the first usable byte, valid for `len` bytes.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.storage[CANARY_LEN..].as_mut_ptr()
    }

    /// The first `n` usable bytes (after a call reported writing `n`).
    pub fn written(&self, n: usize) -> &[u8] {
        assert!(
            n <= self.len,
            "reported {n} bytes, buffer holds {}",
            self.len
        );
        &self.storage[CANARY_LEN..CANARY_LEN + n]
    }

    /// Panics if a canary byte changed, naming the first changed offset
    /// relative to the start of the usable area.
    pub fn check(&self) {
        let usable_end = CANARY_LEN + self.len;
        let zones = (0..CANARY_LEN).chain(usable_end..self.storage.len());
        for i in zones {
            assert!(
                self.storage[i] == CANARY,
                "canary damaged at offset {} (usable size {})",
                i as isize - CANARY_LEN as isize,
                self.len
            );
        }
    }
}
