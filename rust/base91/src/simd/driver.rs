// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Encode and decode drivers (spec 0010 §3).
//!
//! One core per direction, used by every API layer. The cores write into
//! `&mut [MaybeUninit<u8>]` and never allocate; they handle the stream
//! layout themselves (the `-` prefix, line breaks, terminators), so that the
//! kernels only ever see whole blocks, in place:
//!
//! - **Encode:** every block of 16 characters is written by a kernel
//!   directly at its final position; a line of W characters (W a multiple
//!   of 32) is a whole number of blocks, followed by one LF.
//! - **Decode:** the layout is decided by the header (spec 0008 §3.4). An
//!   unwrapped payload is never searched for terminators: whole blocks go
//!   to the kernels, whose validity check rejects any byte outside the
//!   alphabet. A wrapped payload is decoded a line at a time, with one
//!   terminator comparison per line. Anything else (a block split across
//!   calls, the end of the stream, an invalid batch) goes through a
//!   byte-at-a-time path that applies spec 0008 §3.5 exactly, including
//!   which error is reported.

use core::mem::MaybeUninit;

#[cfg(target_arch = "aarch64")]
use super::aarch64;
#[cfg(target_arch = "x86_64")]
use super::x86;
use super::ArchLevel;
use crate::Error;

/// The SIMD alphabet in index order (spec 0008 §3.1).
pub(crate) const ALPHABET: &[u8; 91] =
    b"#$%&()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

/// Character to index; `NOT_ALPHABET` outside the alphabet.
const INDEX: [u8; 256] = {
    let mut table = [NOT_ALPHABET; 256];
    let mut i = 0;
    while i < 91 {
        table[ALPHABET[i] as usize] = i as u8;
        i += 1;
    }
    table
};
const NOT_ALPHABET: u8 = 0xFF;

/// Decoded bytes for a payload tail of `L mod 16` characters (spec 0008
/// §3.3); `None` for the invalid lengths 1, 6 and 11.
const TAIL_BYTES: [Option<usize>; 16] = [
    Some(0),
    None,
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    None,
    Some(5),
    Some(6),
    Some(7),
    Some(8),
    None,
    Some(9),
    Some(10),
    Some(11),
    Some(12),
];

/// Blocks decoded per validity check on the fast path. An invalid batch is
/// re-read byte by byte, so this also bounds that work.
const BATCH: usize = 64;

/// Blocks per batch of whole wrapped lines (decode). Larger than `BATCH`:
/// such a batch holds only complete lines followed by their terminators,
/// so it fails only on invalid input, and the per-batch driver work is
/// spread over more lines (spec 0011, stage 2).
const LINE_BATCH: usize = 256;

/// First batch of a wrapped stream's first line, in blocks: W is unknown
/// until the first terminator, and the batch holding it fails and is
/// re-read byte by byte. Starting small and doubling bounds that re-read
/// to about twice the line (spec 0011, stage 2).
const FIRST_LINE_PROBE: usize = 2;

pub(crate) fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// Writes `b` at `out[*w]` and advances `*w`.
#[inline]
fn put(out: &mut [MaybeUninit<u8>], w: &mut usize, b: u8) {
    out[*w].write(b);
    *w += 1;
}

// ---------------------------------------------------------------------------
// Block runners: dispatch to the kernels of a level
// ---------------------------------------------------------------------------
//
// A batch is `lines` lines of `n` blocks. The encoded side of line `l`
// starts at `l · stride` (a line and its terminator); the raw side is
// contiguous. See the batch entry points in `x86.rs` and `aarch64.rs`.

/// Encodes a batch of blocks (13 bytes → 16 characters).
///
/// # Safety
/// The kernels of `level` are available (see `effective_level`). `src` is
/// readable for `13 n · lines + 3` bytes; every line's `16 n` bytes at
/// `dst + l · stride` are writable.
unsafe fn encode_run(
    level: ArchLevel,
    src: *const u8,
    dst: *mut u8,
    stride: usize,
    n: usize,
    lines: usize,
) {
    match level {
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Avx2 => {
            if n.is_multiple_of(2) {
                x86::encode_lines_avx2(src, dst, stride, n / 2, lines);
            } else {
                // Odd blocks per line: only a single, partial line.
                for l in 0..lines {
                    let (src, dst) = (src.add(13 * n * l), dst.add(stride * l));
                    x86::encode_lines_avx2(src, dst, 0, n / 2, 1);
                    x86::encode_lines_sse41(src.add(13 * (n - 1)), dst.add(16 * (n - 1)), 0, 1, 1);
                }
            }
        }
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Sse41 => x86::encode_lines_sse41(src, dst, stride, n, lines),
        #[cfg(target_arch = "aarch64")]
        ArchLevel::Neon => aarch64::encode_lines_neon(src, dst, stride, n, lines),
        ArchLevel::Scalar => {
            for l in 0..lines {
                for i in 0..n {
                    let block = &*src.add(13 * (n * l + i)).cast::<[u8; 13]>();
                    let chars = encode_block_scalar(block);
                    core::ptr::copy_nonoverlapping(
                        chars.as_ptr(),
                        dst.add(stride * l + 16 * i),
                        16,
                    );
                }
            }
        }
    }
}

/// Decodes a batch of blocks (16 characters → 13 bytes), and returns
/// whether all of them were valid. The output is written either way.
///
/// # Safety
/// The kernels of `level` are available. Every line's `16 n` bytes at
/// `src + l · stride` are readable; `dst` is writable for
/// `13 n · lines + DECODE_SLACK`.
unsafe fn decode_run(
    level: ArchLevel,
    src: *const u8,
    stride: usize,
    dst: *mut u8,
    n: usize,
    lines: usize,
) -> bool {
    match level {
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Avx2 => {
            if n.is_multiple_of(2) {
                x86::decode_lines_avx2(src, stride, dst, n / 2, lines)
            } else {
                // Odd blocks per line: only a single, partial line.
                let mut ok = true;
                for l in 0..lines {
                    let (src, dst) = (src.add(stride * l), dst.add(13 * n * l));
                    ok &= x86::decode_lines_avx2(src, 0, dst, n / 2, 1);
                    ok &= x86::decode_lines_sse41(
                        src.add(16 * (n - 1)),
                        0,
                        dst.add(13 * (n - 1)),
                        1,
                        1,
                    );
                }
                ok
            }
        }
        #[cfg(target_arch = "x86_64")]
        ArchLevel::Sse41 => x86::decode_lines_sse41(src, stride, dst, n, lines),
        #[cfg(target_arch = "aarch64")]
        ArchLevel::Neon => aarch64::decode_lines_neon(src, stride, dst, n, lines),
        ArchLevel::Scalar => {
            let mut bad = 0;
            for l in 0..lines {
                for i in 0..n {
                    bad |= decode_block_scalar(
                        src.add(stride * l + 16 * i),
                        dst.add(13 * (n * l + i)),
                    );
                }
            }
            bad == 0
        }
    }
}

/// Bytes the decode kernels may store past their 13 per block.
const DECODE_SLACK: usize = 3;

/// Encodes `lines` × `n` blocks from `src` (contiguous) to `out`, line `l`
/// at `w + l · stride`.
fn encode_blocks(
    level: ArchLevel,
    src: &[u8],
    out: &mut [MaybeUninit<u8>],
    w: usize,
    stride: usize,
    n: usize,
    lines: usize,
) {
    let blocks = n * lines;
    assert!(n > 0 && lines > 0 && src.len() >= 13 * blocks);
    assert!(out.len() >= w + stride * (lines - 1) + 16 * n);
    let dst = out[w..].as_mut_ptr().cast::<u8>();
    // SAFETY (all calls): `level` comes from `effective_level`; the writes
    // stay within `out` (checked above). The kernels load 3 bytes past each
    // block, which stays within `src` except for the very last block: when
    // `src` ends there, that block goes through a padded copy.
    unsafe {
        if src.len() >= 13 * blocks + 3 {
            encode_run(level, src.as_ptr(), dst, stride, n, lines);
            return;
        }
        let last_line = dst.add(stride * (lines - 1));
        encode_run(level, src.as_ptr(), dst, stride, n, lines - 1);
        let tail = &src[13 * n * (lines - 1)..];
        encode_run(level, tail.as_ptr(), last_line, 0, n - 1, 1);
        let mut pad = [0u8; 16];
        pad[..13].copy_from_slice(&tail[13 * (n - 1)..13 * n]);
        encode_run(level, pad.as_ptr(), last_line.add(16 * (n - 1)), 0, 1, 1);
    }
}

/// Decodes `lines` × `n` blocks, line `l` read from `src[l · stride..]`, to
/// `out[w..w + 13 n · lines]`, and returns whether they were all valid.
fn decode_blocks(
    level: ArchLevel,
    src: &[u8],
    stride: usize,
    out: &mut [MaybeUninit<u8>],
    w: usize,
    n: usize,
    lines: usize,
) -> bool {
    let bytes = 13 * n * lines;
    assert!(n > 0 && lines > 0 && src.len() >= stride * (lines - 1) + 16 * n);
    assert!(out.len() >= w + bytes);
    let dst = out[w..].as_mut_ptr().cast::<u8>();
    // SAFETY (all calls): `level` comes from `effective_level`; the reads
    // stay within `src` and the writes within `out` (checked above), the
    // stores past each block's 13 bytes included: when `out` has no room
    // for the very last block's slack, that block goes through a stack
    // buffer.
    unsafe {
        if out.len() - w >= bytes + DECODE_SLACK {
            return decode_run(level, src.as_ptr(), stride, dst, n, lines);
        }
        let last_line = src[stride * (lines - 1)..].as_ptr();
        let last_out = dst.add(13 * n * (lines - 1));
        let mut last = [0u8; 16];
        let ok = decode_run(level, src.as_ptr(), stride, dst, n, lines - 1)
            & decode_run(level, last_line, 0, last_out, n - 1, 1)
            & decode_run(
                level,
                last_line.add(16 * (n - 1)),
                0,
                last.as_mut_ptr(),
                1,
                1,
            );
        core::ptr::copy_nonoverlapping(last.as_ptr(), last_out.add(13 * (n - 1)), 13);
        ok
    }
}

/// Scalar block encoder: 13 bytes → 16 characters.
fn encode_block_scalar(block: &[u8]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..13].copy_from_slice(&block[..13]);
    let bits = u128::from_le_bytes(bytes);
    let mut chars = [0u8; 16];
    for k in 0..8 {
        let v = ((bits >> (13 * k)) & 0x1FFF) as usize;
        chars[2 * k] = ALPHABET[v % 91];
        chars[2 * k + 1] = ALPHABET[v / 91];
    }
    chars
}

/// Scalar block decoder: 16 characters → 13 bytes. Returns a mask that is
/// nonzero if any character is not in the alphabet or any pair is worth
/// more than 8191; the output is written either way.
///
/// The fixed output pattern of the former `ScalarDecoder` fast path, with
/// validity checks added.
///
/// # Safety
/// `src` readable for 16 bytes, `dst` writable for 13.
#[inline(always)]
unsafe fn decode_block_scalar(src: *const u8, dst: *mut u8) -> u32 {
    let block = &*src.cast::<[u8; 16]>();
    let out = &mut *dst.cast::<[u8; 13]>();
    // Bit 7 of an index is set only for `NOT_ALPHABET`; a pair value above
    // 8191 has bits above bit 12. Pairs are handled one at a time, straight
    // into the output: an array of values invites the compiler to vectorize
    // this loop, badly.
    let mut bad = 0u32;
    let mut pair = |k: usize| {
        let lo = INDEX[block[2 * k] as usize];
        let hi = INDEX[block[2 * k + 1] as usize];
        let v = u32::from(lo) + 91 * u32::from(hi);
        bad |= u32::from(lo | hi) & 0x80 | v >> 13;
        v & 0x1FFF
    };
    // Starting a block with no pending bits, pairs 0, 2 and 5 complete one
    // byte and the others two: the output positions are fixed.
    let mut q = pair(0);
    out[0] = q as u8;
    q = (q >> 8) | pair(1) << 5;
    out[1] = q as u8;
    out[2] = (q >> 8) as u8;
    q = (q >> 16) | pair(2) << 2;
    out[3] = q as u8;
    q = (q >> 8) | pair(3) << 7;
    out[4] = q as u8;
    out[5] = (q >> 8) as u8;
    q = (q >> 16) | pair(4) << 4;
    out[6] = q as u8;
    out[7] = (q >> 8) as u8;
    q = (q >> 16) | pair(5) << 1;
    out[8] = q as u8;
    q = (q >> 8) | pair(6) << 6;
    out[9] = q as u8;
    out[10] = (q >> 8) as u8;
    q = (q >> 16) | pair(7) << 3;
    out[11] = q as u8;
    out[12] = (q >> 8) as u8;
    bad
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// Streaming encoder core (spec 0008 §3.4, §3.6): up to 12 pending bits,
/// the column, and whether the prefix has been written.
#[derive(Clone, Debug)]
pub(crate) struct EncodeCore {
    level: ArchLevel,
    /// Line width: 0, or a multiple of 32 (checked by the callers).
    wrap: usize,
    started: bool,
    col: usize,
    queue: u32,
    nbits: u32,
}

impl EncodeCore {
    pub(crate) fn new(level: ArchLevel, wrap: usize) -> Self {
        debug_assert!(wrap.is_multiple_of(32));
        Self {
            level,
            wrap,
            started: false,
            col: 0,
            queue: 0,
            nbits: 0,
        }
    }

    /// Exactly what `encode` writes for `n` input bytes.
    pub(crate) fn encode_len(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let header = if self.started {
            0
        } else {
            1 + usize::from(self.wrap > 0)
        };
        // Whole 13-bit groups from the pending bits and 8n new ones.
        let chars = 2 * ((self.nbits as usize + 8 * n) / 13);
        let newlines = if self.wrap == 0 {
            0
        } else {
            (self.col + chars) / self.wrap
        };
        header + chars + newlines
    }

    /// Exactly what `finish` writes.
    pub(crate) fn finish_len(&self) -> usize {
        if !self.started {
            return 0;
        }
        let chars = match self.nbits {
            0 => 0,
            1..=6 => 1,
            _ => 2,
        };
        // An LF when the characters complete a line, and a final LF unless
        // the stream ends on a line boundary: ⌈(col + chars) / W⌉ in all.
        let newlines = if self.wrap == 0 {
            0
        } else {
            (self.col + chars).div_ceil(self.wrap)
        };
        chars + newlines
    }

    /// Encodes `input` into `out`, which holds at least
    /// `encode_len(input.len())` bytes, and returns the bytes written.
    pub(crate) fn encode(&mut self, input: &[u8], out: &mut [MaybeUninit<u8>]) -> usize {
        if input.is_empty() {
            return 0;
        }
        let mut w = 0;
        if !self.started {
            // The prefix is written lazily, so that an empty stream stays
            // empty (spec 0008 §3.4).
            self.started = true;
            put(out, &mut w, b'-');
            if self.wrap > 0 {
                put(out, &mut w, b'\n');
            }
        }
        let mut i = 0;
        // Pending bits: a byte at a time up to the next block boundary.
        while self.nbits != 0 && i < input.len() {
            self.push_byte(input[i], out, &mut w);
            i += 1;
        }
        // Whole blocks, straight to their final position: the rest of the
        // current line, then whole lines in one batch (each followed by its
        // LF), then what is left.
        let mut blocks = (input.len() - i) / 13;
        while blocks > 0 {
            let per_line = self.wrap / 16;
            let (n, lines) = if self.wrap > 0 && self.col == 0 && blocks >= per_line {
                // Batches of about `BATCH` blocks, so that the LFs below go
                // into lines still in the cache.
                (per_line, (blocks / per_line).min((BATCH / per_line).max(1)))
            } else if self.wrap > 0 {
                (blocks.min((self.wrap - self.col) / 16), 1)
            } else {
                (blocks, 1)
            };
            let stride = self.wrap + 1;
            encode_blocks(self.level, &input[i..], out, w, stride, n, lines);
            i += 13 * n * lines;
            blocks -= n * lines;
            if self.wrap == 0 {
                w += 16 * n;
            } else if self.col + 16 * n == self.wrap {
                // The batch ends lines (whole ones after the first, since the
                // first starts at `col`): an LF after each.
                let first = self.wrap - self.col;
                let lfs = &mut out[w + first..=w + first + stride * (lines - 1)];
                for lf in lfs.iter_mut().step_by(stride) {
                    lf.write(b'\n');
                }
                w += first + 1 + stride * (lines - 1);
                self.col = 0;
            } else {
                w += 16 * n;
                self.col += 16 * n;
            }
        }
        for &b in &input[i..] {
            self.push_byte(b, out, &mut w);
        }
        w
    }

    /// Writes the last characters and the final LF (spec 0008 §3.4) into
    /// `out`, which holds at least `finish_len()` bytes, and resets the
    /// encoder.
    pub(crate) fn finish(&mut self, out: &mut [MaybeUninit<u8>]) -> usize {
        let mut w = 0;
        if self.started {
            if self.nbits > 0 {
                // A partial group of r bits: one character if r ≤ 6, two
                // otherwise (spec 0008 §3.2, rule 4).
                let v = (self.queue & 0x1FFF) as usize;
                self.put_char(ALPHABET[v % 91], out, &mut w);
                if self.nbits > 6 {
                    self.put_char(ALPHABET[v / 91], out, &mut w);
                }
            }
            if self.wrap > 0 && self.col != 0 {
                put(out, &mut w, b'\n');
            }
        }
        *self = Self::new(self.level, self.wrap);
        w
    }

    fn push_byte(&mut self, b: u8, out: &mut [MaybeUninit<u8>], w: &mut usize) {
        self.queue |= u32::from(b) << self.nbits;
        self.nbits += 8;
        if self.nbits >= 13 {
            let v = (self.queue & 0x1FFF) as usize;
            self.queue >>= 13;
            self.nbits -= 13;
            self.put_char(ALPHABET[v % 91], out, w);
            self.put_char(ALPHABET[v / 91], out, w);
        }
    }

    fn put_char(&mut self, c: u8, out: &mut [MaybeUninit<u8>], w: &mut usize) {
        put(out, w, c);
        if self.wrap > 0 {
            self.col += 1;
            if self.col == self.wrap {
                put(out, w, b'\n');
                self.col = 0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Right after `-`: the next byte decides the layout.
    Header,
    /// After `-` CR: CR LF makes the stream wrapped.
    HeaderCr,
    Body,
    /// An error was reported; every later call reports it again.
    Failed(Error),
}

/// Streaming decoder core for a SIMD stream, from the byte after its `-`
/// (spec 0008 §3.5, §3.6). Bounded state: the layout, the line, a pending
/// whitespace run (its start, length and first bytes) and up to 15 payload
/// characters of a split block.
#[derive(Clone, Debug)]
pub(crate) struct DecodeCore {
    level: ArchLevel,
    phase: Phase,
    /// Stream offset of the next byte.
    offset: usize,
    /// The terminator of a wrapped stream; empty when unwrapped.
    term: &'static [u8],
    /// Line width, once the first line is complete.
    width: usize,
    lines_done: usize,
    /// Characters in the current line.
    line: usize,
    /// Pending whitespace run: start, length (0 = none) and first bytes.
    run_start: usize,
    run_len: usize,
    run_head: [u8; 4],
    /// Payload characters of the current, incomplete block.
    carry: [u8; 16],
    carry_len: usize,
    /// Offset of the pending pair's first character.
    pair_offset: usize,
    payload_len: usize,
    /// Blocks in the next first-line batch of a wrapped stream.
    probe: usize,
}

impl DecodeCore {
    /// A decoder for the stream bytes that follow the `-` at `offset - 1`.
    pub(crate) fn new(level: ArchLevel, offset: usize) -> Self {
        Self {
            level,
            phase: Phase::Header,
            offset,
            term: b"",
            width: 0,
            lines_done: 0,
            line: 0,
            run_start: 0,
            run_len: 0,
            run_head: [0; 4],
            carry: [0; 16],
            carry_len: 0,
            pair_offset: 0,
            payload_len: 0,
            probe: FIRST_LINE_PROBE,
        }
    }

    /// Upper bound on what `decode` writes for `n` input bytes: the blocks
    /// that the carried characters and `n` more can complete. Never more
    /// than `decode_size_hint(n + 15)`.
    pub(crate) fn decode_len(&self, n: usize) -> usize {
        13 * ((self.carry_len + n) / 16)
    }

    /// Exactly what `finish` writes, or the error it reports.
    pub(crate) fn finish_len(&self) -> Result<usize, Error> {
        match self.phase {
            Phase::Failed(e) => Err(e),
            Phase::Header | Phase::HeaderCr => Ok(0),
            Phase::Body => TAIL_BYTES[self.carry_len].ok_or(Error::InvalidLength),
        }
    }

    /// Decodes `input` into `out`, which holds at least
    /// `decode_len(input.len())` bytes, and returns the bytes written.
    pub(crate) fn decode(
        &mut self,
        input: &[u8],
        out: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        let mut i = 0;
        let mut w = 0;
        while i < input.len() {
            match self.phase {
                Phase::Failed(e) => return Err(e),
                Phase::Header => match input[i] {
                    b'\n' => {
                        self.term = b"\n";
                        self.phase = Phase::Body;
                        self.offset += 1;
                        i += 1;
                    }
                    b'\r' => {
                        self.phase = Phase::HeaderCr;
                        self.offset += 1;
                        i += 1;
                    }
                    _ => self.phase = Phase::Body,
                },
                Phase::HeaderCr => {
                    self.phase = Phase::Body;
                    if input[i] == b'\n' {
                        self.term = b"\r\n";
                        self.offset += 1;
                        i += 1;
                    } else {
                        // Unwrapped, starting with a whitespace run.
                        self.run_start = self.offset - 1;
                        self.run_len = 1;
                        self.run_head[0] = b'\r';
                    }
                }
                Phase::Body => {
                    let step = if self.run_len == 0 && self.carry_len == 0 {
                        self.fast(&input[i..], out, &mut w)
                    } else {
                        Ok(0)
                    };
                    let step = match step {
                        Ok(0) => self.slow(input[i], out, &mut w).map(|()| 1),
                        other => other,
                    };
                    match step {
                        Ok(used) => i += used,
                        Err(e) => {
                            self.phase = Phase::Failed(e);
                            return Err(e);
                        }
                    }
                }
            }
        }
        Ok(w)
    }

    /// Checks the end of the stream and writes the last bytes into `out`,
    /// which holds at least `finish_len()` bytes.
    pub(crate) fn finish(&mut self, out: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
        match self.phase {
            Phase::Failed(e) => Err(e),
            // `-` alone, or followed only by whitespace.
            Phase::Header | Phase::HeaderCr => Ok(0),
            Phase::Body => {
                // A pending run is trailing whitespace, and the last line may
                // be shorter than W: only the payload length is left to check
                // (spec 0008 §3.5, rule 6).
                debug_assert_eq!(self.carry_len, self.payload_len % 16);
                let Some(m) = TAIL_BYTES[self.carry_len] else {
                    self.phase = Phase::Failed(Error::InvalidLength);
                    return Err(Error::InvalidLength);
                };
                let mut bits = 0u128;
                for (k, &c) in self.carry[..self.carry_len].iter().enumerate() {
                    // Pairs were checked as they completed; a last odd
                    // character carries padding bits, which are ignored.
                    let weight = if k % 2 == 1 { 91 } else { 1 };
                    bits += (u128::from(INDEX[c as usize]) * weight) << (13 * (k / 2));
                }
                for (k, &b) in bits.to_le_bytes()[..m].iter().enumerate() {
                    out[k].write(b);
                }
                Ok(m)
            }
        }
    }

    /// The fast path: whole blocks in place, and wrapped line breaks, while
    /// no whitespace run is pending and no block is split. Returns the input
    /// bytes consumed (0 when it cannot proceed).
    fn fast(
        &mut self,
        input: &[u8],
        out: &mut [MaybeUninit<u8>],
        w: &mut usize,
    ) -> Result<usize, Error> {
        let t = self.term;
        let term_at = |at: usize| input[at] == t[0] && (t.len() == 1 || input[at + 1] == t[1]);
        let mut i = 0;
        loop {
            let known = !t.is_empty() && self.lines_done > 0;
            // A full line: one terminator comparison, and the line break is
            // taken when the next line visibly starts.
            if known && self.line == self.width {
                let next = i + t.len();
                if next < input.len() && term_at(i) && !is_whitespace(input[next]) {
                    self.offset += t.len();
                    self.lines_done += 1;
                    self.line = 0;
                    i = next;
                } else {
                    return Ok(i);
                }
            }
            // At a line start with W known: whole lines, as many as are
            // present with their terminators (the last one's is checked on
            // the next turn), up to a batch.
            let (n, lines, stride) = if known && self.line == 0 {
                let (per_line, stride) = (self.width / 16, self.width + t.len());
                // The lines wholly present, assuming the layout; their
                // terminators are checked after the kernel call (below),
                // when the batch is in the cache.
                let present = match (input.len() - i).checked_sub(self.width) {
                    Some(rest) => rest / stride + 1,
                    None => 0,
                };
                (
                    per_line,
                    present.min((LINE_BATCH / per_line).max(1)),
                    stride,
                )
            } else {
                let room = if known {
                    (self.width - self.line) / 16
                } else if !t.is_empty() {
                    // The first line of a wrapped stream (W unknown).
                    self.probe
                } else {
                    usize::MAX
                };
                (((input.len() - i) / 16).min(room).min(BATCH), 1, 0)
            };
            if n == 0 || lines == 0 {
                return Ok(i);
            }
            let span = stride * (lines - 1) + 16 * n;
            let batch = &input[i..i + span];
            let decoded = decode_blocks(self.level, batch, stride, out, *w, n, lines);
            // The terminators between the batch's lines, one fixed-count,
            // branch-free pass (the one after the last line is checked on the
            // next turn).
            let mut off = 0;
            if lines > 1 {
                let terms = &batch[16 * n..];
                for (k, &want) in t.iter().enumerate() {
                    off |= terms[k..]
                        .iter()
                        .step_by(stride)
                        .take(lines - 1)
                        .fold(0, |acc, &b| acc | (b ^ want));
                }
            }
            if decoded && off == 0 {
                if !known {
                    self.probe = (2 * self.probe).min(BATCH);
                }
                i += span;
                *w += 13 * n * lines;
                self.offset += span;
                self.payload_len += 16 * n * lines;
                self.lines_done += lines - 1;
                if lines > 1 {
                    self.line = 0;
                }
                self.line += 16 * n;
            } else {
                // Some byte of the batch is invalid, or not payload at all
                // (a terminator, trailing whitespace): the slow path re-reads
                // it and reports the first error, if any.
                for &b in batch {
                    self.slow(b, out, w)?;
                }
                return Ok(i + span);
            }
        }
    }

    /// The slow path: one byte, applying spec 0008 §3.5 "Which error is
    /// reported" rules 1 to 5.
    fn slow(&mut self, b: u8, out: &mut [MaybeUninit<u8>], w: &mut usize) -> Result<(), Error> {
        let at = self.offset;
        self.offset += 1;
        if is_whitespace(b) {
            if self.run_len == 0 {
                self.run_start = at;
            }
            if self.run_len < self.run_head.len() {
                self.run_head[self.run_len] = b;
            }
            self.run_len += 1;
            return Ok(());
        }
        // Rule 1: judge the pending run.
        let mut new_line = false;
        if self.run_len > 0 {
            new_line = self.judge_run()?;
            self.run_len = 0;
        }
        // Rules 2 and 3: line structure, before content.
        if !self.term.is_empty() {
            if new_line {
                if self.lines_done == 0 {
                    self.width = self.line;
                    if self.width == 0 || !self.width.is_multiple_of(32) {
                        return Err(Error::InvalidLayout { offset: at });
                    }
                }
                if self.line != self.width {
                    return Err(Error::InvalidLayout { offset: at });
                }
                self.lines_done += 1;
                self.line = 0;
            }
            if self.lines_done > 0 && self.line == self.width {
                return Err(Error::InvalidLayout { offset: at });
            }
            self.line += 1;
        }
        // Rule 4.
        let index = INDEX[b as usize];
        if index == NOT_ALPHABET {
            return Err(Error::InvalidCharacter { offset: at });
        }
        // Rule 5, when a pair completes.
        if self.carry_len.is_multiple_of(2) {
            self.pair_offset = at;
        } else {
            let lo = u32::from(INDEX[self.carry[self.carry_len - 1] as usize]);
            if lo + 91 * u32::from(index) > 0x1FFF {
                return Err(Error::InvalidValue {
                    offset: self.pair_offset,
                });
            }
        }
        self.carry[self.carry_len] = b;
        self.carry_len += 1;
        self.payload_len += 1;
        if self.carry_len == 16 {
            let mut bytes = [0u8; 13];
            // SAFETY: `carry` holds 16 bytes, `bytes` 13.
            let bad = unsafe { decode_block_scalar(self.carry.as_ptr(), bytes.as_mut_ptr()) };
            debug_assert_eq!(bad, 0);
            for &byte in &bytes {
                put(out, w, byte);
            }
            self.carry_len = 0;
        }
        Ok(())
    }

    /// Judges the pending whitespace run, now that a non-whitespace byte
    /// follows it: a line break (`Ok(true)`), or the error at its first byte
    /// that is not part of one (spec 0008 §3.5, rule 1).
    fn judge_run(&self) -> Result<bool, Error> {
        let head = &self.run_head[..self.run_len.min(self.run_head.len())];
        let t = self.term;
        let mut first = 0;
        if !t.is_empty() && self.line > 0 && head.starts_with(t) {
            if self.run_len == t.len() {
                return Ok(true);
            }
            first = t.len();
        }
        let offset = self.run_start + first;
        let starts_terminator = head[first] == b'\n'
            || (head[first] == b'\r' && first + 1 < self.run_len && head[first + 1] == b'\n');
        Err(if starts_terminator {
            Error::InvalidLayout { offset }
        } else {
            Error::InvalidCharacter { offset }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{effective_level, SimdLevel};
    use super::*;
    use base91_testkit::mutate::mutations;
    use base91_testkit::oracle::{self, Classification, DecodeError};
    use base91_testkit::rng::Rng;

    /// The distinct levels this CPU runs.
    fn levels() -> Vec<ArchLevel> {
        let mut levels = Vec::new();
        for level in [SimdLevel::Scalar, SimdLevel::Simd128, SimdLevel::Simd256] {
            let arch = effective_level(level);
            if !levels.contains(&arch) {
                levels.push(arch);
            }
        }
        levels
    }

    fn expected(stream: &[u8]) -> Result<Vec<u8>, Error> {
        oracle::decode(stream).map_err(|e| match e {
            DecodeError::InvalidCharacter { offset } => Error::InvalidCharacter { offset },
            DecodeError::InvalidLayout { offset } => Error::InvalidLayout { offset },
            DecodeError::InvalidLength => Error::InvalidLength,
            DecodeError::InvalidValue { offset } => Error::InvalidValue { offset },
        })
    }

    /// Decodes a SIMD stream with the core, in pieces of the given lengths
    /// (the rest in one piece), each into exactly the documented bound.
    fn decode(stream: &[u8], level: ArchLevel, lengths: &[usize]) -> Result<Vec<u8>, Error> {
        let dash = stream.iter().position(|&b| !is_whitespace(b)).unwrap();
        let mut core = DecodeCore::new(level, dash + 1);
        let mut out: Vec<u8> = Vec::new();
        let mut rest = &stream[dash + 1..];
        let mut lengths = lengths.iter();
        while !rest.is_empty() {
            let n = lengths.next().map_or(rest.len(), |&n| n.min(rest.len()));
            let (piece, tail) = rest.split_at(n);
            let capacity = core.decode_len(piece.len());
            out.reserve_exact(capacity);
            let spare = &mut out.spare_capacity_mut()[..capacity];
            let written = core.decode(piece, spare)?;
            // SAFETY: the core initialized `written` bytes of the spare
            // capacity.
            unsafe { out.set_len(out.len() + written) };
            rest = tail;
        }
        let capacity = core.finish_len()?;
        out.reserve_exact(capacity);
        let spare = &mut out.spare_capacity_mut()[..capacity];
        let written = core.finish(spare)?;
        assert_eq!(written, capacity);
        // SAFETY: as above.
        unsafe { out.set_len(out.len() + written) };
        Ok(out)
    }

    /// Chunkings of a stream of `len` bytes: whole, fixed sizes, random.
    fn chunkings(rng: &mut Rng, len: usize) -> Vec<Vec<usize>> {
        let mut all = vec![Vec::new()];
        for size in [1, 7, 17] {
            all.push(vec![size; len.div_ceil(size)]);
        }
        all.push(rng.split_points(len));
        all
    }

    /// Valid streams and their mutations (spec 0009 §4): the core gives the
    /// oracle's result, bytes or exact error, at every level and for every
    /// chunking.
    #[test]
    fn decode_matches_oracle() {
        let mut rng = Rng::for_test("driver_decode");
        let mut sizes: Vec<usize> = (0..=60).collect();
        sizes.extend((0..6).map(|_| 61 + rng.below(2000)));
        let (mut compared, mut rejected) = (0usize, 0usize);
        for n in sizes {
            let input = rng.bytes(n);
            for width in [0, 32, 64] {
                let stream = oracle::encode(&input, width);
                let mut streams = vec![stream.clone()];
                streams.extend(
                    mutations(&stream, &mut rng, 4)
                        .into_iter()
                        .map(|m| m.stream),
                );
                for s in streams {
                    if oracle::classify(&s) != Classification::Simd {
                        continue;
                    }
                    let want = expected(&s);
                    rejected += usize::from(want.is_err());
                    for lengths in chunkings(&mut rng, s.len()) {
                        for level in levels() {
                            let got = decode(&s, level, &lengths);
                            assert!(
                                got == want,
                                "n={n}, W={width}, {level:?}, chunks {:?}…\n\
                                 stream {:?}\n got {:?}\nwant {:?}",
                                &lengths[..lengths.len().min(3)],
                                String::from_utf8_lossy(&s),
                                got.as_ref().map(|v| v.len()),
                                want.as_ref().map(|v| v.len()),
                            );
                            compared += 1;
                        }
                    }
                }
            }
        }
        assert!(rejected > 0);
        eprintln!("[driver decode] compared {compared}, of which rejected streams {rejected}");
    }

    /// The core's encoding is the oracle's, one-shot and in pieces, and
    /// every call writes exactly the documented length.
    #[test]
    fn encode_matches_oracle() {
        let mut rng = Rng::for_test("driver_encode");
        for n in (0..=200).chain([1000, 4099]) {
            let input = rng.bytes(n);
            for width in [0, 32, 64, 96] {
                let want = oracle::encode(&input, width);
                for lengths in chunkings(&mut rng, n) {
                    for level in levels() {
                        let mut core = EncodeCore::new(level, width);
                        let mut out: Vec<u8> = Vec::new();
                        let mut rest = &input[..];
                        let mut lengths = lengths.iter();
                        while !rest.is_empty() {
                            let k = lengths.next().map_or(rest.len(), |&k| k.min(rest.len()));
                            let (piece, tail) = rest.split_at(k);
                            let capacity = core.encode_len(piece.len());
                            out.reserve_exact(capacity);
                            let written =
                                core.encode(piece, &mut out.spare_capacity_mut()[..capacity]);
                            assert_eq!(written, capacity);
                            // SAFETY: the core initialized `written` bytes.
                            unsafe { out.set_len(out.len() + written) };
                            rest = tail;
                        }
                        let capacity = core.finish_len();
                        out.reserve_exact(capacity);
                        let written = core.finish(&mut out.spare_capacity_mut()[..capacity]);
                        assert_eq!(written, capacity);
                        // SAFETY: as above.
                        unsafe { out.set_len(out.len() + written) };
                        assert!(out == want, "n={n}, W={width}, {level:?}");
                    }
                }
            }
        }
    }
}
