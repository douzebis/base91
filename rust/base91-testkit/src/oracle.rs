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
/// `offset` is the position in the whole input of the first byte that makes
/// the stream invalid, scanning left to right. Implementations are not
/// required to report offsets (§3.5), so tests compare the error class only.
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

    // Everything after `-`, without the trailing whitespace (§3.5).
    let start = dash + 1;
    let end = stream
        .iter()
        .rposition(|b| !WHITESPACE.contains(b))
        .map_or(start, |i| (i + 1).max(start));
    let body = &stream[start..end];
    if body.is_empty() {
        // `-` alone, or `-` followed only by whitespace.
        return Ok(Vec::new());
    }

    let payload = if body[0] == b'\n' || body.starts_with(b"\r\n") {
        parse_wrapped(stream, start)?
    } else {
        parse_unwrapped(stream, start, end)?
    };
    decode_payload(&payload, start)
}

/// Payload characters of an unwrapped stream whose body is
/// `stream[start..end]`.
fn parse_unwrapped(stream: &[u8], start: usize, end: usize) -> Result<Vec<u8>, DecodeError> {
    for (offset, &c) in stream.iter().enumerate().take(end).skip(start) {
        // A line terminator inside an unwrapped payload is a layout error; a
        // CR that does not start a CR LF is just an invalid character.
        if c == b'\n' || (c == b'\r' && stream.get(offset + 1) == Some(&b'\n')) {
            return Err(DecodeError::InvalidLayout { offset });
        }
        if index_of(c).is_none() {
            return Err(DecodeError::InvalidCharacter { offset });
        }
    }
    Ok(stream[start..end].to_vec())
}

/// Payload characters of a wrapped stream whose first terminator starts at
/// `start` (§3.5 grammar: TERM line (TERM line)* [whitespace]).
fn parse_wrapped(stream: &[u8], start: usize) -> Result<Vec<u8>, DecodeError> {
    let term: &[u8] = if stream[start] == b'\n' {
        b"\n"
    } else {
        b"\r\n"
    };
    let end = stream
        .iter()
        .rposition(|b| !WHITESPACE.contains(b))
        .map_or(start, |i| i + 1);

    // Split stream[start + term.len() .. end] into lines on `term`, checking
    // every byte on the way.
    let mut lines: Vec<(usize, Vec<u8>)> = Vec::new(); // (offset of line start, chars)
    let mut line_start = start + term.len();
    let mut line = Vec::new();
    let mut i = line_start;
    while i < end {
        if stream[i..].starts_with(term) {
            lines.push((line_start, std::mem::take(&mut line)));
            i += term.len();
            line_start = i;
            continue;
        }
        let c = stream[i];
        if c == b'\n' || (c == b'\r' && stream.get(i + 1) == Some(&b'\n')) {
            // A terminator of the other kind: mixed LF / CR LF.
            return Err(DecodeError::InvalidLayout { offset: i });
        }
        if index_of(c).is_none() {
            return Err(DecodeError::InvalidCharacter { offset: i });
        }
        line.push(c);
        i += 1;
    }
    lines.push((line_start, line));

    // Line lengths: all but the last equal to W, W a positive multiple of
    // 32, last line 1..=W. With a single line, any nonzero length.
    let width = lines[0].1.len();
    for (k, (offset, chars)) in lines.iter().enumerate() {
        let is_last = k + 1 == lines.len();
        let ok = if lines.len() == 1 {
            !chars.is_empty()
        } else if is_last {
            !chars.is_empty() && chars.len() <= width
        } else {
            width > 0 && width.is_multiple_of(32) && chars.len() == width
        };
        if !ok {
            return Err(DecodeError::InvalidLayout { offset: *offset });
        }
    }
    Ok(lines.into_iter().flat_map(|(_, chars)| chars).collect())
}

/// Unpacks validated payload characters (§3.2 in reverse, §3.3).
///
/// Note: spec 0008 does not yet say what a decoder does with a character
/// pair worth more than 8191 (values 8192..=8280 are expressible with two
/// characters but not with 13 bits). This model rejects such a pair as
/// [`DecodeError::InvalidValue`]; pending confirmation in spec 0008.
/// Padding bits (§3.2 rule 4) are ignored, as §3.5 allows.
fn decode_payload(payload: &[u8], offset_base: usize) -> Result<Vec<u8>, DecodeError> {
    let Some(n) = decoded_len(payload.len()) else {
        return Err(DecodeError::InvalidLength);
    };

    let mut bits: Vec<u32> = Vec::new();
    for (k, pair) in payload.chunks(2).enumerate() {
        let lo = index_of(pair[0]).expect("validated");
        let value = match pair.get(1) {
            Some(&c) => lo + 91 * index_of(c).expect("validated"),
            None => lo,
        };
        if value > 8191 {
            return Err(DecodeError::InvalidValue {
                offset: offset_base + 2 * k,
            });
        }
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
