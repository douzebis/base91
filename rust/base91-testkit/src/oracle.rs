// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Bit-by-bit model of the SIMD wire format, written from spec 0008 alone.
//!
//! This module deliberately shares no code with `base91-rs`. It favors
//! obviousness over speed: bits are handled one at a time, exactly as the
//! spec states them. Review it against `docs/specs/0008-wire-format.md`,
//! section by section; each function names the section it models.

/// The SIMD alphabet in index order, copied from spec 0008 §3.1.
pub const ALPHABET: &[u8; 91] =
    b"#$%&()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

/// Bytes treated as whitespace (spec 0008 §1).
pub const WHITESPACE: [u8; 4] = [b' ', b'\t', b'\r', b'\n'];

/// Number of characters for the tail of `n mod 13` input bytes (§3.3, T).
pub const TAIL_CHARS: [usize; 13] = [0, 2, 3, 4, 5, 7, 8, 9, 10, 12, 13, 14, 15];

/// How a decoder classifies its input (§2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classification {
    /// Nothing but whitespace: decodes to zero bytes.
    Empty,
    /// First non-whitespace byte is `-`.
    Simd,
    /// Anything else: decoded by the legacy (Henke) rules, not modeled here.
    Legacy,
}

/// Why a SIMD stream is rejected (§3.5).
///
/// The error is the first one reading left to right, and `offset` is the
/// position in the whole input that §3.5 "Which error is reported" assigns
/// to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// A byte that is neither an alphabet character nor part of a valid
    /// line terminator.
    InvalidCharacter { offset: usize },
    /// Lines of unequal length, a width that is not a multiple of 32, mixed
    /// terminators, an empty line, or a terminator in the middle of an
    /// unwrapped payload.
    InvalidLayout { offset: usize },
    /// A payload length L with L mod 16 in {1, 6, 11} (§3.3).
    InvalidLength,
    /// A character pair whose value exceeds 8191, which no 13-bit group can
    /// produce (see the note on `decode_payload`).
    InvalidValue { offset: usize },
}

/// Index of an alphabet character, by linear search in [`ALPHABET`].
pub fn index_of(c: u8) -> Option<u32> {
    ALPHABET.iter().position(|&a| a == c).map(|i| i as u32)
}

/// Payload length P(n) for n input bytes (§3.3).
pub fn payload_len(n: usize) -> usize {
    16 * (n / 13) + TAIL_CHARS[n % 13]
}

/// Encoded stream length E(n, W) (§3.4), including the final LF when
/// wrapped.
pub fn encoded_len(n: usize, width: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let p = payload_len(n);
    if width == 0 {
        1 + p
    } else {
        2 + p + p.div_ceil(width)
    }
}

/// Decoded length D(L) for a payload of L characters (§3.3), or `None` for
/// an invalid length.
pub fn decoded_len(payload: usize) -> Option<usize> {
    let rem = payload % 16;
    let m = TAIL_CHARS.iter().position(|&t| t == rem)?;
    Some(13 * (payload / 16) + m)
}

/// Encoded payload (no prefix, no line breaks) of `input` (§3.2).
pub fn encode_payload(input: &[u8]) -> Vec<u8> {
    // Rule 1: the bit stream, least-significant bit of each byte first.
    let bits: Vec<u32> = input
        .iter()
        .flat_map(|&b| (0..8).map(move |j| u32::from((b >> j) & 1)))
        .collect();

    let mut out = Vec::new();
    // Rule 2: consecutive 13-bit groups, least-significant bit first.
    for group in bits.chunks(13) {
        let value: u32 = group.iter().enumerate().map(|(j, &bit)| bit << j).sum();
        let lo = ALPHABET[(value % 91) as usize];
        let hi = ALPHABET[(value / 91) as usize];
        if group.len() == 13 {
            // Rule 3: a full group is two characters, low one first.
            out.extend([lo, hi]);
        } else if group.len() <= 6 {
            // Rule 4: a partial group of r <= 6 bits is one character.
            assert_eq!(value / 91, 0);
            out.push(lo);
        } else {
            // Rule 4: a partial group of r >= 7 bits is two characters.
            out.extend([lo, hi]);
        }
    }
    out
}

/// Canonical encoded stream of `input` with line width `width` (§3.4).
///
/// `width` is 0 (unwrapped) or a positive multiple of 32; anything else
/// panics, since encoders must reject it.
pub fn encode(input: &[u8], width: usize) -> Vec<u8> {
    assert!(
        width.is_multiple_of(32),
        "line width must be 0 or a multiple of 32"
    );
    if input.is_empty() {
        return Vec::new();
    }
    let payload = encode_payload(input);
    let mut out = vec![b'-'];
    if width == 0 {
        out.extend(&payload);
    } else {
        out.push(b'\n');
        for line in payload.chunks(width) {
            out.extend(line);
            out.push(b'\n');
        }
    }
    out
}

/// Classification of a decoder's input (§2).
pub fn classify(stream: &[u8]) -> Classification {
    match stream.iter().find(|b| !WHITESPACE.contains(b)) {
        None => Classification::Empty,
        Some(b'-') => Classification::Simd,
        Some(_) => Classification::Legacy,
    }
}

/// Decodes a stream classified as [`Classification::Simd`] or
/// [`Classification::Empty`] (§2, §3.5).
///
/// Panics on a legacy stream: those are decoded by the upstream reference,
/// not by this model.
pub fn decode(stream: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let Some(dash) = stream.iter().position(|b| !WHITESPACE.contains(b)) else {
        return Ok(Vec::new());
    };
    assert_eq!(stream[dash], b'-', "legacy stream given to the SIMD oracle");

    // The header TERM, if any, makes the stream wrapped (§3.4).
    let after = dash + 1;
    let term: Option<&[u8]> = if stream[after..].starts_with(b"\n") {
        Some(b"\n")
    } else if stream[after..].starts_with(b"\r\n") {
        Some(b"\r\n")
    } else {
        None
    };
    let start = after + term.map_or(0, <[u8]>::len);
    let payload = scan(stream, start, term)?;
    decode_payload(&payload)
}

/// Whether a terminator (LF, or CR LF) starts at `stream[i]`.
fn starts_terminator(stream: &[u8], i: usize) -> bool {
    stream[i] == b'\n' || stream[i..].starts_with(b"\r\n")
}

/// The error for a whitespace run followed by non-whitespace, at the run's
/// byte `i` (§3.5 "Which error is reported", rule 1).
fn run_error(stream: &[u8], i: usize) -> DecodeError {
    if starts_terminator(stream, i) {
        DecodeError::InvalidLayout { offset: i }
    } else {
        DecodeError::InvalidCharacter { offset: i }
    }
}

/// Reads the payload of a SIMD stream from `stream[start..]`, left to right,
/// and returns its characters with their offsets. `term` is the stream's
/// terminator if it is wrapped. Applies rules 1 to 5 of §3.5 "Which error
/// is reported", in stream order; rule 6 (the length) is left to the
/// caller.
fn scan(stream: &[u8], start: usize, term: Option<&[u8]>) -> Result<Vec<(usize, u8)>, DecodeError> {
    let mut payload: Vec<(usize, u8)> = Vec::new();
    // Wrapped only: the length of the current line, of the first line once
    // known, and whether a line has been completed yet.
    let mut line = 0usize;
    let mut width: Option<usize> = None;
    let mut lines_done = 0usize;
    // Start of the pending whitespace run.
    let mut run: Option<usize> = None;

    for (i, &c) in stream.iter().enumerate().skip(start) {
        if WHITESPACE.contains(&c) {
            run.get_or_insert(i);
            continue;
        }
        // A non-whitespace byte: judge the pending run (rule 1).
        let mut new_line = false;
        if let Some(r) = run.take() {
            match term {
                Some(t) if line > 0 && stream[r..i] == *t => new_line = true,
                Some(t) if line > 0 && stream[r..].starts_with(t) => {
                    return Err(run_error(stream, r + t.len()));
                }
                _ => return Err(run_error(stream, r)),
            }
        }
        if term.is_some() {
            // Rule 2, then rule 3: structure before content.
            if new_line {
                let w = *width.get_or_insert(line);
                if lines_done == 0 && !(w > 0 && w.is_multiple_of(32)) {
                    return Err(DecodeError::InvalidLayout { offset: i });
                }
                if line != w {
                    return Err(DecodeError::InvalidLayout { offset: i });
                }
                lines_done += 1;
                line = 0;
            }
            if width.is_some_and(|w| line == w) {
                return Err(DecodeError::InvalidLayout { offset: i });
            }
            line += 1;
        }
        // Rule 4.
        if index_of(c).is_none() {
            return Err(DecodeError::InvalidCharacter { offset: i });
        }
        payload.push((i, c));
        // Rule 5: a pair is complete at its second character.
        if payload.len().is_multiple_of(2) {
            let (lo_at, lo) = payload[payload.len() - 2];
            let value = index_of(lo).unwrap() + 91 * index_of(c).unwrap();
            if value > 8191 {
                return Err(DecodeError::InvalidValue { offset: lo_at });
            }
        }
    }
    // A pending run at the end is trailing whitespace, and the last line may
    // be shorter than W: nothing more to check here.
    Ok(payload)
}

/// Unpacks validated payload characters (§3.2 in reverse, §3.3).
///
/// Pair values were checked by [`scan`] (a pair worth 8192..=8280 is
/// expressible with two characters but not with 13 bits). Padding bits
/// (§3.2 rule 4) are ignored, as §3.5 allows.
fn decode_payload(payload: &[(usize, u8)]) -> Result<Vec<u8>, DecodeError> {
    let Some(n) = decoded_len(payload.len()) else {
        return Err(DecodeError::InvalidLength);
    };

    let mut bits: Vec<u32> = Vec::new();
    for pair in payload.chunks(2) {
        let lo = index_of(pair[0].1).expect("validated");
        let value = match pair.get(1) {
            Some(&(_, c)) => lo + 91 * index_of(c).expect("validated"),
            None => lo,
        };
        assert!(value <= 8191, "checked by scan");
        bits.extend((0..13).map(|j| (value >> j) & 1));
    }

    // Rule 1 in reverse: the first 8n bits, least-significant bit first.
    Ok(bits[..8 * n]
        .chunks(8)
        .map(|byte| {
            byte.iter()
                .enumerate()
                .map(|(j, &bit)| (bit as u8) << j)
                .sum()
        })
        .collect())
}
