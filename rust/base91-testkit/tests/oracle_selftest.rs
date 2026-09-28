// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Self-tests of the SIMD format oracle against hand-derived vectors.
//!
//! Every expected value below is derived by hand from spec 0008, in the
//! comment next to it, without running any implementation.

use base91_testkit::oracle::{
    classify, decode, decoded_len, encode, encoded_len, payload_len, Classification, DecodeError,
    ALPHABET,
};

fn zeros(n: usize) -> Vec<u8> {
    vec![0; n]
}

fn repeat(c: u8, n: usize) -> Vec<u8> {
    vec![c; n]
}

fn stream(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

// ---------------------------------------------------------------------------
// §3.1 Alphabet
// ---------------------------------------------------------------------------

#[test]
fn alphabet_matches_its_formula() {
    // §3.1: char(i) = 0x23 + i + (i >= 4).
    for (i, &c) in ALPHABET.iter().enumerate() {
        assert_eq!(c, 0x23 + i as u8 + u8::from(i >= 4), "index {i}");
    }
}

#[test]
fn alphabet_has_91_ascending_printable_characters_without_quote() {
    assert_eq!(ALPHABET.len(), 91);
    assert!(ALPHABET.windows(2).all(|w| w[0] < w[1]));
    assert!(!ALPHABET.contains(&b'\''));
    assert!(!ALPHABET.contains(&b'"'));
    assert!(ALPHABET.contains(&b'\\'));
    assert!(ALPHABET.iter().all(|c| (0x23..=0x7E).contains(c)));
}

// ---------------------------------------------------------------------------
// §3.2 Bit packing, §3.4 layout: hand-derived encodings
// ---------------------------------------------------------------------------

#[test]
fn encode_empty_is_empty() {
    // §3.4: empty input encodes to the empty stream.
    assert_eq!(encode(&[], 0), b"");
    assert_eq!(encode(&[], 64), b"");
}

#[test]
fn encode_single_zero_byte() {
    // 8 zero bits: r = 8 >= 7, so two characters of value 0: '#', '#'.
    assert_eq!(encode(&[0x00], 0), b"-##");
}

#[test]
fn encode_single_one_bit() {
    // Bit 0 set: v = 1, lo = 1 -> '$', hi = 0 -> '#'.
    assert_eq!(encode(&[0x01], 0), b"-$#");
}

#[test]
fn encode_single_ff_byte() {
    // v = 255: lo = 255 mod 91 = 73 -> 0x24 + 73 = 0x6D 'm'; hi = 2 -> '%'.
    assert_eq!(encode(&[0xFF], 0), b"-m%");
}

#[test]
fn encode_checks_bit_order_across_bytes() {
    // Bit 0 of byte 1 is stream bit 8: group 0 = 2^8 = 256.
    // lo = 256 mod 91 = 74 -> 0x24 + 74 = 0x6E 'n'; hi = 2 -> '%'.
    // 16 bits = 13 + 3: the remaining 3 zero bits (r = 3 <= 6) give one '#'.
    assert_eq!(encode(&[0x00, 0x01], 0), b"-n%#");
}

#[test]
fn encode_two_ff_bytes() {
    // Group 0 = 13 one-bits = 8191: lo = 8191 - 91*90 = 1 -> '$'; hi = 90 -> '~'.
    // Remaining r = 3 one-bits = 7: one character, 0x24 + 7 = 0x2B '+'.
    assert_eq!(encode(&[0xFF, 0xFF], 0), b"-$~+");
}

#[test]
fn encode_one_block_of_ff() {
    // 13 bytes = 8 full groups of 8191, each "$~".
    let expected = stream(&[b"-", &b"$~".repeat(8)]);
    assert_eq!(encode(&repeat(0xFF, 13), 0), expected);
}

#[test]
fn encode_wrapped_exactly_one_line() {
    // 26 zero bytes: P = 32 characters '#'. W = 32: "-" LF, one full line, LF.
    let expected = stream(&[b"-\n", &repeat(b'#', 32), b"\n"]);
    assert_eq!(encode(&zeros(26), 32), expected);
}

#[test]
fn encode_wrapped_two_lines() {
    // 27 zero bytes: P = 16*2 + T[1] = 34: a line of 32 and a line of 2.
    let expected = stream(&[b"-\n", &repeat(b'#', 32), b"\n##\n"]);
    assert_eq!(encode(&zeros(27), 32), expected);
}

#[test]
#[should_panic(expected = "multiple of 32")]
fn encode_rejects_width_16() {
    // §3.4: encoders must reject a nonzero width that is not a multiple of 32.
    encode(&[0], 16);
}

// ---------------------------------------------------------------------------
// §3.3, §3.4 Lengths
// ---------------------------------------------------------------------------

#[test]
fn lengths_match_the_definition() {
    for n in 0..=400usize {
        let input: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
        for width in [0, 32, 64, 96] {
            let encoded = encode(&input, width);
            assert_eq!(encoded.len(), encoded_len(n, width), "n={n} W={width}");
        }
        assert_eq!(decoded_len(payload_len(n)), Some(n), "n={n}");
    }
}

#[test]
fn invalid_payload_lengths_are_1_6_11_mod_16() {
    for length in 0..=64usize {
        let invalid = [1, 6, 11].contains(&(length % 16));
        assert_eq!(decoded_len(length).is_none(), invalid, "L={length}");
    }
}

// ---------------------------------------------------------------------------
// §2 Classification
// ---------------------------------------------------------------------------

#[test]
fn classification() {
    assert_eq!(classify(b""), Classification::Empty);
    assert_eq!(classify(b" \t\r\n"), Classification::Empty);
    assert_eq!(classify(b"-"), Classification::Simd);
    assert_eq!(classify(b"\t\n -##"), Classification::Simd);
    assert_eq!(classify(b"abc"), Classification::Legacy);
    assert_eq!(classify(b" x-"), Classification::Legacy);
}

// ---------------------------------------------------------------------------
// §3.5 Accepted streams
// ---------------------------------------------------------------------------

#[test]
fn decode_hand_vectors() {
    assert_eq!(decode(b"-##"), Ok(vec![0x00]));
    assert_eq!(decode(b"-$#"), Ok(vec![0x01]));
    assert_eq!(decode(b"-m%"), Ok(vec![0xFF]));
    assert_eq!(decode(b"-n%#"), Ok(vec![0x00, 0x01]));
    assert_eq!(decode(b"-$~+"), Ok(vec![0xFF, 0xFF]));
}

#[test]
fn decode_empty_forms() {
    // §3.5: `-` alone, and `-` followed only by whitespace, decode to empty.
    for s in [&b""[..], b"   ", b"-", b"-\n", b"  -  \r\n"] {
        assert_eq!(decode(s), Ok(vec![]), "{s:?}");
    }
}

#[test]
fn decode_tolerates_leading_and_trailing_whitespace() {
    assert_eq!(decode(b"\t \n-##"), Ok(vec![0]));
    assert_eq!(decode(b"-##\r\n \n"), Ok(vec![0]));
}

#[test]
fn decode_wrapped_with_and_without_final_lf() {
    let with = stream(&[b"-\n", &repeat(b'#', 32), b"\n##\n"]);
    let without = stream(&[b"-\n", &repeat(b'#', 32), b"\n##"]);
    assert_eq!(decode(&with), Ok(zeros(27)));
    assert_eq!(decode(&without), Ok(zeros(27)));
}

#[test]
fn decode_wrapped_crlf() {
    let crlf = stream(&[b"-\r\n", &repeat(b'#', 32), b"\r\n##\r\n"]);
    assert_eq!(decode(&crlf), Ok(zeros(27)));
}

#[test]
fn decode_single_line_of_any_length() {
    // With only one line, W is unknown and any length is accepted.
    let s = stream(&[b"-\n", &repeat(b'#', 34), b"\n"]);
    assert_eq!(decode(&s), Ok(zeros(27)));
}

#[test]
fn decode_ignores_padding_bits() {
    // Payload length 3 decodes to 2 bytes = 16 bits: the pair "$~" (8191,
    // thirteen one-bits) gives stream bits 0..=12, and the lone last
    // character gives bits 13.. of which only r = 16 - 13 = 3 are data.
    //
    // '+' is index 7 = 0b111: data bits 1, 1, 1 -> bytes FF FF.
    assert_eq!(decode(b"-$~+"), Ok(vec![0xFF, 0xFF]));
    // '0' is index 12 = 0b1100: data bits (0..=2) are 0, 0, 1; bit 3 is
    // padding and ignored. Byte 1 = stream bits 8..=15
    // = 1,1,1,1,1 (from "$~"), then 0,0,1 = 0b1001_1111 = 0x9F.
    assert_eq!(decode(b"-$~0"), Ok(vec![0xFF, 0x9F]));
}

// ---------------------------------------------------------------------------
// §3.5 Rejected streams
// ---------------------------------------------------------------------------

#[test]
fn reject_invalid_lengths() {
    for length in [1, 6, 11, 17] {
        let s = stream(&[b"-", &repeat(b'#', length)]);
        assert_eq!(decode(&s), Err(DecodeError::InvalidLength), "L={length}");
    }
}

#[test]
fn reject_characters_outside_the_alphabet() {
    assert_eq!(
        decode(b"-#\"#"),
        Err(DecodeError::InvalidCharacter { offset: 2 })
    );
    assert_eq!(
        decode(b"-## #"),
        Err(DecodeError::InvalidCharacter { offset: 3 })
    );
    assert_eq!(
        decode(b"-##'#"),
        Err(DecodeError::InvalidCharacter { offset: 3 })
    );
}

#[test]
fn reject_lone_cr() {
    // A CR that is not part of a CR LF terminator is an invalid character.
    assert_eq!(
        decode(b"-##\r##"),
        Err(DecodeError::InvalidCharacter { offset: 3 })
    );
}

#[test]
fn reject_terminator_inside_unwrapped_payload() {
    assert_eq!(
        decode(b"-##\n##"),
        Err(DecodeError::InvalidLayout { offset: 3 })
    );
}

#[test]
fn reject_width_16() {
    // Two lines of 16: W = 16 is not a multiple of 32, which shows where
    // the second line starts.
    let s = stream(&[b"-\n", &repeat(b'#', 16), b"\n", &repeat(b'#', 16), b"\n"]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidLayout { offset: 19 }));
}

#[test]
fn reject_last_line_longer_than_width() {
    // The 33rd character of the second line, where a terminator was due.
    let s = stream(&[b"-\n", &repeat(b'#', 32), b"\n", &repeat(b'#', 33), b"\n"]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidLayout { offset: 67 }));
}

#[test]
fn reject_empty_line() {
    let s = stream(&[b"-\n", &repeat(b'#', 32), b"\n\n##"]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidLayout { offset: 35 }));
}

#[test]
fn reject_mixed_terminators() {
    let s = stream(&[b"-\n", &repeat(b'#', 32), b"\r\n##"]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidLayout { offset: 34 }));
    let s = stream(&[b"-\r\n", &repeat(b'#', 32), b"\n##"]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidLayout { offset: 35 }));
}

#[test]
fn reject_pair_value_above_8191() {
    // '~' is index 90: 90 + 91 * 90 = 8280 > 8191.
    assert_eq!(decode(b"-~~"), Err(DecodeError::InvalidValue { offset: 1 }));
}

#[test]
fn reject_pair_value_above_8191_at_its_stream_offset() {
    // In a wrapped stream the offset counts the terminators too: the pair
    // starts at 2 + 32 + 1 = 35, not at its payload index.
    let s = stream(&[b"-\n", &repeat(b'#', 32), b"\n~~\n"]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidValue { offset: 35 }));
}

// ---------------------------------------------------------------------------
// §3.5 Which error is reported
// ---------------------------------------------------------------------------

#[test]
fn whitespace_is_judged_when_content_follows() {
    // Trailing whitespace of any kind is accepted.
    assert_eq!(decode(b"-##  \t\r\n \n"), Ok(vec![0]));
    assert_eq!(decode(b"-\n\n"), Ok(Vec::new()));
    assert_eq!(decode(b"-\r\n\r\n"), Ok(Vec::new()));
    // Followed by content, the run fails at its first byte: a space is an
    // invalid character, a terminator an invalid layout.
    assert_eq!(
        decode(b"-##  \n##"),
        Err(DecodeError::InvalidCharacter { offset: 3 })
    );
    assert_eq!(
        decode(b"-##\n  ##"),
        Err(DecodeError::InvalidLayout { offset: 3 })
    );
    // Inside a wrapped line.
    let s = stream(&[b"-\n", &repeat(b'#', 16), b" ", &repeat(b'#', 16)]);
    assert_eq!(
        decode(&s),
        Err(DecodeError::InvalidCharacter { offset: 18 })
    );
    // An empty first line followed by content.
    assert_eq!(
        decode(b"-\n\n##"),
        Err(DecodeError::InvalidLayout { offset: 2 })
    );
}

#[test]
fn short_line_fails_where_the_next_line_starts() {
    // Line 2 has 10 characters but is not the last: the error is at the
    // first byte of line 3, 2 + 33 + 11 = 46.
    let s = stream(&[
        b"-\n",
        &repeat(b'#', 32),
        b"\n",
        &repeat(b'#', 10),
        b"\n",
        &repeat(b'#', 32),
    ]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidLayout { offset: 46 }));
}

#[test]
fn earliest_error_wins() {
    // The short line (error at 46) comes before the invalid character in
    // the next line (at 50).
    let s = stream(&[
        b"-\n",
        &repeat(b'#', 32),
        b"\n",
        &repeat(b'#', 10),
        b"\n####\"",
        &repeat(b'#', 27),
    ]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidLayout { offset: 46 }));
    // An invalid value (at 1) before an invalid character (at 5).
    assert_eq!(
        decode(b"-~~##\"#"),
        Err(DecodeError::InvalidValue { offset: 1 })
    );
}

#[test]
fn structure_before_content() {
    // A full line followed by a byte that is neither a terminator nor in
    // the alphabet: the layout error wins.
    let s = stream(&[b"-\n", &repeat(b'#', 32), b"\n", &repeat(b'#', 32), b"\""]);
    assert_eq!(decode(&s), Err(DecodeError::InvalidLayout { offset: 67 }));
}

// ---------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------

#[test]
fn round_trip() {
    for n in 0..=400usize {
        let input: Vec<u8> = (0..n).map(|i| (i * 73 + 5) as u8).collect();
        for width in [0, 32, 64, 96] {
            assert_eq!(
                decode(&encode(&input, width)),
                Ok(input.clone()),
                "n={n} W={width}"
            );
        }
    }
}
