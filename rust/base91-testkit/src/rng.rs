// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Reproducible randomness for tests (spec 0009, decision 5).
//!
//! One seed per test run: `BASE91_TEST_SEED` if set (any string; a decimal
//! number is used as is, anything else is hashed, e.g. a commit hash), else
//! a fixed default. Tests print the seed they used, so a failure can be
//! replayed with `BASE91_TEST_SEED=<seed> cargo test`.

use std::sync::OnceLock;

/// Seed used when `BASE91_TEST_SEED` is not set.
pub const DEFAULT_SEED: u64 = 0x5EED_BA5E_0091_0008;

/// The run's seed (see the module documentation).
pub fn seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(|| {
        let seed = match std::env::var("BASE91_TEST_SEED") {
            Ok(text) => text.trim().parse().unwrap_or_else(|_| fnv1a(text.trim())),
            Err(_) => DEFAULT_SEED,
        };
        eprintln!("[base91-testkit] BASE91_TEST_SEED={seed}");
        seed
    })
}

/// Whether to run the full differential matrix (`BASE91_TEST_FULL=1`)
/// instead of the rotating subset.
pub fn full_matrix() -> bool {
    std::env::var("BASE91_TEST_FULL").is_ok_and(|v| v == "1")
}

fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A small xorshift64* generator. Not for cryptography; only for
/// reproducible test data.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator for the run's seed, specialized by `stream` so that
    /// independent tests draw independent sequences.
    pub fn for_test(stream: &str) -> Self {
        Self::new(seed() ^ fnv1a(stream))
    }

    pub fn new(seed: u64) -> Self {
        // xorshift must not start from zero.
        Self(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in `0..bound` (`bound` > 0), up to a negligible bias.
    pub fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next_u64() as u8).collect()
    }

    /// Splits `0..len` into consecutive random chunk lengths (each >= 1).
    pub fn split_points(&mut self, len: usize) -> Vec<usize> {
        let mut lengths = Vec::new();
        let mut rest = len;
        while rest > 0 {
            let take = 1 + self.below(rest.min(64));
            lengths.push(take);
            rest -= take;
        }
        lengths
    }
}
