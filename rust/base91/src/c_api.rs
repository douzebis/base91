// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! C-compatible API — drop-in replacement for `base91.h` / `base91.c`.
//!
//! The struct layout and all five function signatures are identical to the
//! Joachim Henke reference implementation, so existing C callers only need
//! to swap the library at link time.

use crate::codec::{DECTAB, ENCTAB};
use core::ffi::c_void;

/// C-compatible state struct.  Layout matches `struct basE91` in `base91.h`:
///
/// ```c
/// struct basE91 { unsigned long queue; unsigned int nbits; int val; };
/// ```
///
/// `val == -1` means no pending first character (mirrors the C sentinel).
#[repr(C)]
pub struct basE91 {
    /// Bit accumulator.  C uses `unsigned long` (64-bit on x86-64 Linux);
    /// we use `u64` to match exactly.
    pub queue: u64,
    /// Number of valid bits in `queue`.
    pub nbits: u32,
    /// Pending first character of a decode pair, or −1 when empty.
    pub val: i32,
}

/// Initialize (or reset) a `basE91` state struct.
#[no_mangle]
pub unsafe extern "C" fn basE91_init(b: *mut basE91) {
    (*b).queue = 0;
    (*b).nbits = 0;
    (*b).val = -1;
}

/// Encode `len` bytes from `i`, appending base91 characters to `o`.
///
/// Returns the number of bytes written.  `o` must have room for at least
/// `encode_size_hint(len)` bytes.
#[no_mangle]
pub unsafe extern "C" fn basE91_encode(
    b: *mut basE91,
    i: *const c_void,
    len: usize,
    o: *mut c_void,
) -> usize {
    let input = core::slice::from_raw_parts(i as *const u8, len);
    let ob = o as *mut u8;
    let mut n: usize = 0;

    // Hoist state to locals so LLVM can keep them in registers and emit
    // immediate-count shifts — same technique as encode_unchecked.
    let mut queue = (*b).queue;
    let mut nbits = (*b).nbits;

    for &byte in input {
        queue |= (byte as u64) << nbits;
        nbits += 8;
        if nbits > 13 {
            // Safety: val ≤ 91²−1 = 8280, so q,r ∈ 0..=90.
            let val = (queue as u32) & 0x1fff;
            if val > 88 {
                // 13-bit path: val in 89..=8191
                queue >>= 13;
                nbits -= 13;
                let q = val / 91;
                let r = val - q * 91;
                ob.add(n).write(*ENCTAB.get_unchecked(r as usize));
                ob.add(n + 1).write(*ENCTAB.get_unchecked(q as usize));
            } else {
                // 14-bit path: val in 0..=88 or 8192..=8280
                let val = (queue as u32) & 0x3fff;
                queue >>= 14;
                nbits -= 14;
                let q = val / 91;
                let r = val - q * 91;
                ob.add(n).write(*ENCTAB.get_unchecked(r as usize));
                ob.add(n + 1).write(*ENCTAB.get_unchecked(q as usize));
            }
            n += 2;
        }
    }

    (*b).queue = queue;
    (*b).nbits = nbits;
    n
}

/// Flush remaining bits after the last `basE91_encode` call.
///
/// Writes 0, 1, or 2 bytes to `o`.  Returns the number of bytes written.
/// Resets the struct state so it can be reused.
#[no_mangle]
pub unsafe extern "C" fn basE91_encode_end(b: *mut basE91, o: *mut c_void) -> usize {
    let ob = o as *mut u8;
    let mut n: usize = 0;

    if (*b).nbits > 0 {
        let queue = (*b).queue as u32;
        ob.add(n)
            .write(*ENCTAB.get_unchecked((queue % 91) as usize));
        n += 1;
        if (*b).nbits > 7 || queue > 90 {
            ob.add(n)
                .write(*ENCTAB.get_unchecked((queue / 91) as usize));
            n += 1;
        }
    }
    (*b).queue = 0;
    (*b).nbits = 0;
    (*b).val = -1;
    n
}

/// Decode `len` base91 characters from `i`, writing raw bytes to `o`.
///
/// Non-alphabet bytes are silently skipped.
/// Returns the number of bytes written.
#[no_mangle]
pub unsafe extern "C" fn basE91_decode(
    b: *mut basE91,
    i: *const c_void,
    len: usize,
    o: *mut c_void,
) -> usize {
    let input = core::slice::from_raw_parts(i as *const u8, len);
    let ob = o as *mut u8;
    let mut n: usize = 0;

    // Hoist state to locals so LLVM can keep them in registers.
    // Without this, every access goes through the pointer `b`, which LLVM
    // cannot prove non-aliasing with `ob`, causing memory round-trips on
    // every iteration — the same slowdown seen in the C reference.
    let mut queue = (*b).queue;
    let mut nbits = (*b).nbits;
    let mut val = (*b).val;

    for &byte in input {
        let d = *DECTAB.get_unchecked(byte as usize) as u32;
        if d == 91 {
            continue;
        }
        if val == -1 {
            val = d as i32;
        } else {
            let v = val as u32 + d * 91;
            val = -1;
            queue |= (v as u64) << nbits;
            nbits += if v & 0x1fff > 88 { 13 } else { 14 };
            ob.add(n).write(queue as u8);
            n += 1;
            queue >>= 8;
            nbits -= 8;
            if nbits >= 8 {
                ob.add(n).write(queue as u8);
                n += 1;
                queue >>= 8;
                nbits -= 8;
            }
        }
    }

    (*b).queue = queue;
    (*b).nbits = nbits;
    (*b).val = val;
    n
}

/// Flush any remaining partial decode value after the last `basE91_decode`.
///
/// Writes 0 or 1 byte to `o`.  Resets the struct state.
#[no_mangle]
pub unsafe extern "C" fn basE91_decode_end(b: *mut basE91, o: *mut c_void) -> usize {
    let ob = o as *mut u8;
    let mut n: usize = 0;

    if (*b).val != -1 {
        ob.add(n)
            .write(((*b).queue as u32 | ((*b).val as u32) << (*b).nbits) as u8);
        n += 1;
    }
    (*b).queue = 0;
    (*b).nbits = 0;
    (*b).val = -1;
    n
}
