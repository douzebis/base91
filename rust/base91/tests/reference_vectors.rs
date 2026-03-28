// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Reference-vector tests derived from `src/test/test.sh`.
//!
//! Each test encodes or decodes a fixture file and checks the result
//! byte-for-byte against the expected output produced by the C reference
//! implementation.  Expected SHA-256 hashes are computed from the reference
//! output and stored as constants.

use sha2::{Digest, Sha256};
use std::fs;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
}

fn sha256hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

// ---------------------------------------------------------------------------
// Expected SHA-256 hashes (computed from C reference output)
// ---------------------------------------------------------------------------

// Encoded fixtures (no-wrap, matching `b91enc` default):
const SHA256_BIT0_B91: &str = "7a422b32e93f19ac3ce5404eef902ebc5d455ee3908372be39ef89d05f561885";
const SHA256_BIT1_B91: &str = "f39d828129685524431b58c7cfb9db12b64d578fc7aa2b8f556aaa04f84aaeb0";
const SHA256_RND0_B91: &str = "c1bc60717b1b59a9f78fdfb4224ba893f9e205cf614667380d8efd1d7db516df";
const SHA256_RND1_B91: &str = "535df3e3603624890f0b16b5186c7e22b00a5cd33c38dc06fb4090b9346134fc";

// Decoded fixture (rnd0.dat decoded → rnd1.dat):
const SHA256_RND1_DAT: &str = "dc5ce0f754128136c7ff8cebb79fbd0758f6e5007040ddce2c220021b5b59d2e";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn encode_bit0_matches_reference() {
    let data = fixture("bit0.dat");
    let encoded = base91::encode(&data);
    assert_eq!(encoded.len(), 1_198_370, "encoded length mismatch for bit0");
    assert_eq!(sha256hex(&encoded), SHA256_BIT0_B91);
}

#[test]
fn encode_bit1_matches_reference() {
    let data = fixture("bit1.dat");
    let encoded = base91::encode(&data);
    assert_eq!(encoded.len(), 1_290_552, "encoded length mismatch for bit1");
    assert_eq!(sha256hex(&encoded), SHA256_BIT1_B91);
}

#[test]
fn encode_rnd0_matches_reference() {
    let data = fixture("rnd0.dat");
    let encoded = base91::encode(&data);
    assert_eq!(encoded.len(), 2_633, "encoded length mismatch for rnd0");
    assert_eq!(sha256hex(&encoded), SHA256_RND0_B91);
}

#[test]
fn encode_rnd1_matches_reference() {
    let data = fixture("rnd1.dat");
    let encoded = base91::encode(&data);
    assert_eq!(encoded.len(), 770, "encoded length mismatch for rnd1");
    assert_eq!(sha256hex(&encoded), SHA256_RND1_B91);
}

#[test]
fn decode_rnd0_matches_reference() {
    // rnd0.dat is itself a base91-encoded file; decoding it yields rnd1.dat.
    let data = fixture("rnd0.dat");
    let decoded = base91::decode(&data);
    assert_eq!(decoded.len(), 626, "decoded length mismatch");
    assert_eq!(sha256hex(&decoded), SHA256_RND1_DAT);
}

#[test]
fn round_trip_bit0() {
    let data = fixture("bit0.dat");
    assert_eq!(base91::decode(&base91::encode(&data)), data);
}

#[test]
fn round_trip_bit1() {
    let data = fixture("bit1.dat");
    assert_eq!(base91::decode(&base91::encode(&data)), data);
}

#[test]
fn round_trip_rnd0() {
    let data = fixture("rnd0.dat");
    assert_eq!(base91::decode(&base91::encode(&data)), data);
}

#[test]
fn round_trip_rnd1() {
    let data = fixture("rnd1.dat");
    assert_eq!(base91::decode(&base91::encode(&data)), data);
}
