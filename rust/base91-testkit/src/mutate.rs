// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Mutations of valid SIMD streams (spec 0009 §4): the kinds of damage that
//! editors, mail software and line-ending conversions cause, plus random
//! byte edits. The expected outcome of each mutation is whatever the oracle
//! says (bytes, or an error class).

use crate::oracle::{ALPHABET, WHITESPACE};
use crate::rng::Rng;

/// A named mutated stream.
pub struct Mutation {
    pub name: String,
    pub stream: Vec<u8>,
}

fn named(name: impl Into<String>, stream: Vec<u8>) -> Mutation {
    Mutation {
        name: name.into(),
        stream,
    }
}

/// Bytes outside the alphabet that are worth trying.
const FOREIGN: [u8; 6] = [b'"', b'\'', b' ', b'\t', 0x00, 0x80];

/// Payload characters of a stream: everything except `-` and whitespace.
fn payload(stream: &[u8]) -> Vec<u8> {
    stream
        .iter()
        .copied()
        .filter(|b| *b != b'-' && !WHITESPACE.contains(b))
        .collect()
}

/// `-` LF, then `payload` in lines of `width` characters, each followed by
/// LF (any width, including invalid ones).
fn rewrap(payload: &[u8], width: usize) -> Vec<u8> {
    let mut out = b"-\n".to_vec();
    for line in payload.chunks(width) {
        out.extend(line);
        out.push(b'\n');
    }
    out
}

/// All mutations of `stream`, a valid SIMD stream produced by an encoder.
/// Random positions come from `rng`; `edits` random edits are drawn per
/// kind.
pub fn mutations(stream: &[u8], rng: &mut Rng, edits: usize) -> Vec<Mutation> {
    let mut out = Vec::new();
    if stream.is_empty() {
        return out;
    }
    let len = stream.len();

    for _ in 0..edits {
        let at = rng.below(len);
        let foreign = FOREIGN[rng.below(FOREIGN.len())];
        let mut s = stream.to_vec();
        s[at] = foreign;
        out.push(named(
            format!("replace [{at}] with foreign {foreign:#04x}"),
            s,
        ));

        let at = rng.below(len);
        let own = ALPHABET[rng.below(ALPHABET.len())];
        let mut s = stream.to_vec();
        s[at] = own;
        out.push(named(format!("replace [{at}] with alphabet {own:#04x}"), s));

        let at = rng.below(len + 1);
        let byte = if rng.below(2) == 0 { foreign } else { b'\n' };
        let mut s = stream.to_vec();
        s.insert(at, byte);
        out.push(named(format!("insert {byte:#04x} at [{at}]"), s));

        let at = rng.below(len);
        let mut s = stream.to_vec();
        s.remove(at);
        out.push(named(format!("delete [{at}]"), s));
    }

    // Line-ending conversions.
    let crlf: Vec<u8> = stream
        .iter()
        .flat_map(|&b| {
            if b == b'\n' {
                vec![b'\r', b'\n']
            } else {
                vec![b]
            }
        })
        .collect();
    out.push(named("LF -> CR LF everywhere", crlf));
    if let Some(first) = stream.iter().position(|&b| b == b'\n') {
        let mut s = stream.to_vec();
        s.insert(first, b'\r');
        out.push(named("LF -> CR LF at the first LF only", s));
    }

    // Re-wrapping, valid and invalid widths.
    let chars = payload(stream);
    for width in [16, 48, 50, 64, 76] {
        out.push(named(
            format!("re-wrapped at {width}"),
            rewrap(&chars, width),
        ));
    }

    // Whitespace around the stream.
    out.push(named("leading whitespace", [b"\n\t ", stream].concat()));
    out.push(named("trailing whitespace", [stream, b" \r\n\n"].concat()));

    // Quoted-printable soft line break inside the payload.
    let at = 1 + rng.below(len);
    let mut s = stream.to_vec();
    s.splice(at..at, *b"=\r\n");
    out.push(named(format!("quoted-printable soft break at [{at}]"), s));

    // Truncations.
    if len <= 64 {
        for at in 0..len {
            out.push(named(format!("truncated to {at}"), stream[..at].to_vec()));
        }
    } else {
        for _ in 0..edits {
            let at = rng.below(len);
            out.push(named(format!("truncated to {at}"), stream[..at].to_vec()));
        }
    }

    out.push(named("lone dash", b"-".to_vec()));
    out
}
