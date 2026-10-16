// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! libbase91: the C library of base91-rs (spec 0014 §3).
//!
//! Two APIs, declared in `include/base91.h`:
//!
//! - Henke's basE91 API (`struct basE91`, `basE91_init`, `basE91_encode`,
//!   `basE91_encode_end`, `basE91_decode`, `basE91_decode_end`), binary
//!   compatible with basE91 0.6.0 and running base91-rs's codec.  The state
//!   stays in the caller's struct between calls, as in Henke's library.
//! - The `base91_*` API, for both formats, one shot, without allocation.
//!
//! No panic crosses into C: every exported function runs in [`guard`],
//! which aborts the process on a panic.

#![deny(unsafe_op_in_unsafe_fn)]

use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CStr};
use core::mem::MaybeUninit;
use core::ptr::NonNull;
use std::panic::{catch_unwind, AssertUnwindSafe};

use base91::{__capi, Error};

/// `struct basE91`, as in Henke's `base91.h`: callers allocate it, so its
/// layout is ABI.
#[repr(C)]
pub struct BasE91 {
    pub queue: c_ulong,
    pub nbits: c_uint,
    pub val: c_int,
}

/// Runs `f`, aborting the process if it panics: unwinding into C is
/// undefined behavior.
fn guard<T>(f: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(_) => std::process::abort(),
    }
}

/// Henke's API reads its input through this buffer, one chunk at a time:
/// his command-line tool overlaps the input with the output, which Rust
/// slices may not, and reading a chunk before writing its output keeps the
/// writes behind the unread input, as his byte-by-byte loop does.
const CHUNK: usize = 4096;

/// `len` bytes of C memory at `p`, as uninitialized output.
///
/// # Safety
/// For `len > 0`: `p` is valid for writes of `len` bytes, and nothing else
/// accesses them while the slice lives.
unsafe fn out_slice<'a>(p: *mut c_void, len: usize) -> &'a mut [MaybeUninit<u8>] {
    let p = if len == 0 {
        NonNull::<MaybeUninit<u8>>::dangling().as_ptr()
    } else {
        p.cast()
    };
    // SAFETY: the caller's contract; a dangling, aligned pointer for 0.
    unsafe { core::slice::from_raw_parts_mut(p, len) }
}

/// `len` bytes of C memory at `p`, as input.
///
/// # Safety
/// For `len > 0`: `p` is valid for reads of `len` bytes, not written while
/// the slice lives.
unsafe fn in_slice<'a>(p: *const c_void, len: usize) -> &'a [u8] {
    let p = if len == 0 {
        NonNull::<u8>::dangling().as_ptr()
    } else {
        p.cast()
    };
    // SAFETY: as for `out_slice`.
    unsafe { core::slice::from_raw_parts(p, len) }
}

// ---------------------------------------------------------------------------
// Henke's basE91 API
// ---------------------------------------------------------------------------

/// Henke's `val` for "no pending character" is -1; base91-rs's is
/// `u32::MAX`.
fn val_to_rust(val: c_int) -> u32 {
    u32::try_from(val).unwrap_or(u32::MAX)
}

fn val_to_c(val: u32) -> c_int {
    c_int::try_from(val).unwrap_or(-1)
}

/// # Safety
/// `b` points to a writable `struct basE91`.
#[no_mangle]
pub unsafe extern "C" fn basE91_init(b: *mut BasE91) {
    guard(|| {
        // SAFETY: the caller's contract.
        unsafe {
            b.write(BasE91 {
                queue: 0,
                nbits: 0,
                val: -1,
            })
        }
    })
}

/// # Safety
/// As Henke's: `b` initialized by `basE91_init`; `i` readable for `len`
/// bytes; `o` writable for what the call may write (16/13 of `len`, plus
/// 2). Input and output may overlap as in Henke's command-line tool.
#[no_mangle]
pub unsafe extern "C" fn basE91_encode(
    b: *mut BasE91,
    i: *const c_void,
    len: usize,
    o: *mut c_void,
) -> usize {
    guard(|| {
        if len == 0 {
            return 0;
        }
        // SAFETY: the caller's contract.
        let state = unsafe { &mut *b };
        // `queue` holds at most 21 bits.
        let mut encoder = __capi::encoder(state.queue as u32, state.nbits);
        let mut chunk = [0u8; CHUNK];
        let (mut read, mut written) = (0, 0);
        while read < len {
            let n = (len - read).min(CHUNK);
            // SAFETY: `i` is readable for `len` bytes; `chunk` is ours.
            unsafe {
                core::ptr::copy_nonoverlapping(i.cast::<u8>().add(read), chunk.as_mut_ptr(), n)
            };
            let bound = __capi::encode_len(&encoder, n);
            // SAFETY: `o` is writable for the whole call's output, of which
            // `bound` bounds this chunk's share; the input is only read
            // through `chunk` while the slice lives.
            let out = unsafe { out_slice(o.cast::<u8>().add(written).cast(), bound) };
            written += __capi::encode(&mut encoder, &chunk[..n], out);
            read += n;
        }
        let (queue, nbits) = __capi::encoder_state(&encoder);
        state.queue = c_ulong::from(queue);
        state.nbits = nbits;
        written
    })
}

/// # Safety
/// As Henke's: `b` as in `basE91_encode`; `o` writable for 2 bytes.
#[no_mangle]
pub unsafe extern "C" fn basE91_encode_end(b: *mut BasE91, o: *mut c_void) -> usize {
    guard(|| {
        // SAFETY: the caller's contract.
        let state = unsafe { &mut *b };
        let mut encoder = __capi::encoder(state.queue as u32, state.nbits);
        // SAFETY: `o` is writable for 2 bytes, and `encode_finish_len` is at
        // most 2.
        let out = unsafe { out_slice(o, __capi::encode_finish_len(&encoder)) };
        let n = __capi::encode_finish(&mut encoder, out);
        *state = BasE91 {
            queue: 0,
            nbits: 0,
            val: -1,
        };
        n
    })
}

/// # Safety
/// As Henke's: `b` initialized by `basE91_init`; `i` readable for `len`
/// bytes; `o` writable for what the call may write (14/16 of `len`, plus
/// 1). Input and output may overlap as in Henke's command-line tool.
#[no_mangle]
pub unsafe extern "C" fn basE91_decode(
    b: *mut BasE91,
    i: *const c_void,
    len: usize,
    o: *mut c_void,
) -> usize {
    guard(|| {
        if len == 0 {
            return 0;
        }
        // SAFETY: the caller's contract.
        let state = unsafe { &mut *b };
        let mut decoder = __capi::decoder(state.queue as u32, state.nbits, val_to_rust(state.val));
        let mut chunk = [0u8; CHUNK];
        let (mut read, mut written) = (0, 0);
        while read < len {
            let n = (len - read).min(CHUNK);
            // SAFETY: as in `basE91_encode`.
            unsafe {
                core::ptr::copy_nonoverlapping(i.cast::<u8>().add(read), chunk.as_mut_ptr(), n)
            };
            let bound = __capi::decode_len(&decoder, n);
            // SAFETY: as in `basE91_encode`.
            let out = unsafe { out_slice(o.cast::<u8>().add(written).cast(), bound) };
            written += __capi::decode(&mut decoder, &chunk[..n], out);
            read += n;
        }
        let (queue, nbits, val) = __capi::decoder_state(&decoder);
        state.queue = c_ulong::from(queue);
        state.nbits = nbits;
        state.val = val_to_c(val);
        written
    })
}

/// # Safety
/// As Henke's: `b` as in `basE91_decode`; `o` writable for 1 byte.
#[no_mangle]
pub unsafe extern "C" fn basE91_decode_end(b: *mut BasE91, o: *mut c_void) -> usize {
    guard(|| {
        // SAFETY: the caller's contract.
        let state = unsafe { &mut *b };
        let mut decoder = __capi::decoder(state.queue as u32, state.nbits, val_to_rust(state.val));
        // SAFETY: `o` is writable for 1 byte, and `decode_finish_len` is at
        // most 1.
        let out = unsafe { out_slice(o, __capi::decode_finish_len(&decoder)) };
        let n = __capi::decode_finish(&mut decoder, out);
        *state = BasE91 {
            queue: 0,
            nbits: 0,
            val: -1,
        };
        n
    })
}

// ---------------------------------------------------------------------------
// The base91_* API
// ---------------------------------------------------------------------------

pub const BASE91_HENKE: c_int = 0;
pub const BASE91_SIMD: c_int = 1;

pub const BASE91_OK: c_int = 0;
pub const BASE91_ERR_BUFFER_TOO_SMALL: c_int = 1;
pub const BASE91_ERR_INVALID_WIDTH: c_int = 2;
pub const BASE91_ERR_INVALID_CHARACTER: c_int = 3;
pub const BASE91_ERR_INVALID_LAYOUT: c_int = 4;
pub const BASE91_ERR_INVALID_LENGTH: c_int = 5;
pub const BASE91_ERR_INVALID_VALUE: c_int = 6;
pub const BASE91_ERR_NULL_POINTER: c_int = 7;
pub const BASE91_ERR_INVALID_FORMAT: c_int = 8;

/// The return code of an error, and its offset.
fn error_code(e: Error) -> (c_int, Option<usize>) {
    match e {
        Error::BufferTooSmall { .. } => (BASE91_ERR_BUFFER_TOO_SMALL, None),
        Error::InvalidWidth { .. } => (BASE91_ERR_INVALID_WIDTH, None),
        Error::InvalidCharacter { offset } => (BASE91_ERR_INVALID_CHARACTER, Some(offset)),
        Error::InvalidLayout { offset } => (BASE91_ERR_INVALID_LAYOUT, Some(offset)),
        Error::InvalidLength => (BASE91_ERR_INVALID_LENGTH, None),
        Error::InvalidValue { offset } => (BASE91_ERR_INVALID_VALUE, Some(offset)),
        // `Error` is non-exhaustive: a future error, never a success.
        _ => (BASE91_ERR_INVALID_VALUE, None),
    }
}

/// The output size of `base91_encode`, or the error code.
fn encode_len(format: c_int, len: usize, wrap: usize) -> Result<usize, c_int> {
    match format {
        BASE91_HENKE if wrap == 0 => Ok(base91::encode_size_hint(len)),
        BASE91_HENKE => Err(BASE91_ERR_INVALID_WIDTH),
        BASE91_SIMD => base91::simd::encoded_len(len, wrap).map_err(|_| BASE91_ERR_INVALID_WIDTH),
        _ => Err(BASE91_ERR_INVALID_FORMAT),
    }
}

#[no_mangle]
pub extern "C" fn base91_encoded_len_max(format: c_int, len: usize, wrap: usize) -> usize {
    guard(|| encode_len(format, len, wrap).unwrap_or(0))
}

/// # Safety
/// `in_` readable for `len` bytes; `out` writable for `out_cap` bytes, not
/// overlapping the input; `written` writable.
#[no_mangle]
pub unsafe extern "C" fn base91_encode(
    format: c_int,
    in_: *const c_void,
    len: usize,
    wrap: usize,
    out: *mut c_void,
    out_cap: usize,
    written: *mut usize,
) -> c_int {
    guard(|| {
        if written.is_null() || (in_.is_null() && len > 0) {
            return BASE91_ERR_NULL_POINTER;
        }
        let needed = match encode_len(format, len, wrap) {
            Ok(needed) => needed,
            Err(code) => return code,
        };
        if out_cap < needed {
            // SAFETY: `written` is not null, and writable.
            unsafe { written.write(needed) };
            return BASE91_ERR_BUFFER_TOO_SMALL;
        }
        if out.is_null() && needed > 0 {
            return BASE91_ERR_NULL_POINTER;
        }
        // SAFETY: the caller's contract; `needed <= out_cap`.
        let (input, output) = unsafe { (in_slice(in_, len), out_slice(out, needed)) };
        let n = if format == BASE91_HENKE {
            __capi::henke_encode(input, output)
        } else {
            match __capi::simd_encode(input, wrap, output) {
                Ok(n) => n,
                Err(e) => return error_code(e).0,
            }
        };
        // SAFETY: as above.
        unsafe { written.write(n) };
        BASE91_OK
    })
}

#[no_mangle]
pub extern "C" fn base91_decoded_len_max(len: usize) -> usize {
    guard(|| base91::decode_size_hint(len))
}

/// # Safety
/// `in_` readable for `len` bytes; `out` writable for `out_cap` bytes, not
/// overlapping the input; `written` writable; `error_offset` writable or
/// null.
#[no_mangle]
pub unsafe extern "C" fn base91_decode(
    in_: *const c_void,
    len: usize,
    out: *mut c_void,
    out_cap: usize,
    written: *mut usize,
    error_offset: *mut usize,
) -> c_int {
    guard(|| {
        if written.is_null() || (in_.is_null() && len > 0) {
            return BASE91_ERR_NULL_POINTER;
        }
        let needed = base91::decode_size_hint(len);
        if out_cap < needed {
            // SAFETY: `written` is not null, and writable.
            unsafe { written.write(needed) };
            return BASE91_ERR_BUFFER_TOO_SMALL;
        }
        if out.is_null() {
            return BASE91_ERR_NULL_POINTER;
        }
        // SAFETY: the caller's contract; `needed <= out_cap`.
        let (input, output) = unsafe { (in_slice(in_, len), out_slice(out, needed)) };
        match __capi::decode_any(input, output) {
            Ok(n) => {
                // SAFETY: as above.
                unsafe { written.write(n) };
                BASE91_OK
            }
            Err(e) => {
                let (code, offset) = error_code(e);
                if let (Some(offset), false) = (offset, error_offset.is_null()) {
                    // SAFETY: `error_offset` is not null, and writable.
                    unsafe { error_offset.write(offset) };
                }
                code
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn base91_simd_level() -> *const c_char {
    guard(|| {
        let name: &'static CStr = match base91::simd::detect() {
            base91::simd::SimdLevel::Simd256 => c"avx2",
            base91::simd::SimdLevel::Simd128 if cfg!(target_arch = "x86_64") => c"sse4.1",
            base91::simd::SimdLevel::Simd128 => c"neon",
            base91::simd::SimdLevel::Scalar => c"scalar",
        };
        name.as_ptr()
    })
}

#[no_mangle]
pub extern "C" fn base91_strerror(code: c_int) -> *const c_char {
    let text: &'static CStr = match code {
        BASE91_OK => c"success",
        BASE91_ERR_BUFFER_TOO_SMALL => c"output buffer too small",
        BASE91_ERR_INVALID_WIDTH => c"invalid line width (0, or a multiple of 32)",
        BASE91_ERR_INVALID_CHARACTER => c"invalid character",
        BASE91_ERR_INVALID_LAYOUT => c"invalid line layout",
        BASE91_ERR_INVALID_LENGTH => c"invalid payload length",
        BASE91_ERR_INVALID_VALUE => c"invalid value",
        BASE91_ERR_NULL_POINTER => c"null pointer",
        BASE91_ERR_INVALID_FORMAT => c"invalid format",
        _ => c"unknown error",
    };
    text.as_ptr()
}

#[no_mangle]
pub extern "C" fn base91_version() -> *const c_char {
    const VERSION: &CStr =
        match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
            Ok(version) => version,
            Err(_) => panic!("the version contains a NUL byte"),
        };
    VERSION.as_ptr()
}

#[cfg(test)]
mod tests;
