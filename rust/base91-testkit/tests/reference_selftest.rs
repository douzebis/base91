// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Self-tests of the vendored upstream basE91 0.6.0 and of its wrapper.

use sha2::{Digest, Sha256};
use std::path::Path;

fn sha256hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// sha256 of the vendored files, as listed in
/// `reference/basE91-0.6.0/PROVENANCE` (and archived by Software Heritage).
const VENDORED: [(&str, &str); 4] = [
    (
        "base91.c",
        "ce6c825e24342271b525f2cecc5f70294047246963cc5361003326c9d6ce8290",
    ),
    (
        "base91.h",
        "30d476141130aa6391a240c9b94e42e60f71b38f056a2a349fec9bc96562b6bf",
    ),
    (
        "cli.c",
        "a37ce4cebb71ad7e6d0c23157e651ec6848770fd6fb931706b70a2c044ba87d5",
    ),
    (
        "LICENSE",
        "51f70a0b8d3df8662f97469234af3c8fbf780b8d853568ff9d32b56d6d4b996b",
    ),
];

#[test]
fn vendored_reference_is_pristine() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("reference/basE91-0.6.0");
    for (name, expected) in VENDORED {
        let data = std::fs::read(dir.join(name))
            .unwrap_or_else(|e| panic!("cannot read vendored {name}: {e}"));
        assert_eq!(
            sha256hex(&data),
            expected,
            "{name} differs from upstream basE91 0.6.0"
        );
    }
}

#[cfg(feature = "reference")]
mod upstream {
    use super::sha256hex;
    use base91_testkit::reference;

    /// Fixtures shared with base91-rs (`tests/fixtures`, from upstream's
    /// `test.sh`), and the sha256 of upstream's output for them.
    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../base91/tests/fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
    }

    #[test]
    fn encodes_fixtures_like_upstream_test_suite() {
        for (name, len, sha) in [
            (
                "bit0.dat",
                1_198_370,
                "7a422b32e93f19ac3ce5404eef902ebc5d455ee3908372be39ef89d05f561885",
            ),
            (
                "bit1.dat",
                1_290_552,
                "f39d828129685524431b58c7cfb9db12b64d578fc7aa2b8f556aaa04f84aaeb0",
            ),
            (
                "rnd0.dat",
                2_633,
                "c1bc60717b1b59a9f78fdfb4224ba893f9e205cf614667380d8efd1d7db516df",
            ),
            (
                "rnd1.dat",
                770,
                "535df3e3603624890f0b16b5186c7e22b00a5cd33c38dc06fb4090b9346134fc",
            ),
        ] {
            let encoded = reference::encode(&fixture(name));
            assert_eq!(encoded.len(), len, "{name}");
            assert_eq!(sha256hex(&encoded), sha, "{name}");
        }
    }

    #[test]
    fn decodes_fixture_like_upstream_test_suite() {
        // rnd0.dat is itself basE91 text; upstream decodes it to rnd1.dat's
        // content.
        let decoded = reference::decode(&fixture("rnd0.dat"));
        assert_eq!(decoded.len(), 626);
        assert_eq!(
            sha256hex(&decoded),
            "dc5ce0f754128136c7ff8cebb79fbd0758f6e5007040ddce2c220021b5b59d2e"
        );
    }

    #[test]
    fn streaming_matches_one_shot() {
        let input = fixture("bit1.dat");
        let encoded = reference::encode(&input);
        for chunk in [1, 2, 3, 7, 13, 4096] {
            let mut enc = reference::Encoder::new();
            let mut out = Vec::new();
            for piece in input.chunks(chunk) {
                enc.encode(piece, &mut out);
            }
            enc.finish(&mut out);
            assert!(out == encoded, "encode, chunk {chunk}");

            let mut dec = reference::Decoder::new();
            let mut back = Vec::new();
            for piece in encoded.chunks(chunk) {
                dec.decode(piece, &mut back);
            }
            dec.finish(&mut back);
            assert!(back == input, "decode, chunk {chunk}");
        }
    }

    #[test]
    fn empty_is_empty() {
        assert_eq!(reference::encode(b""), b"");
        assert_eq!(reference::decode(b""), b"");
    }

    #[test]
    fn ignores_bytes_outside_the_alphabet() {
        // Upstream skips anything outside its alphabet, including line breaks.
        let encoded = reference::encode(b"hello, world");
        let mut noisy = Vec::new();
        for &c in &encoded {
            noisy.extend([c, b'\n', b' ', b'-']);
        }
        assert_eq!(reference::decode(&noisy), b"hello, world");
    }
}
