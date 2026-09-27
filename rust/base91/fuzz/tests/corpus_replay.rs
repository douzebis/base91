// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Replays the fuzz corpus without a fuzzing engine (spec 0009 §6): every
//! file under `corpus/<target>/` goes through that target's check function,
//! in every CI test run. A crash that libFuzzer finds once, and that is added
//! to the corpus, stays a regression test for good.

use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

type Check = fn(&[u8]);

const TARGETS: [(&str, Check); 4] = [
    ("fuzz_decode", base91_fuzz::check_decode),
    ("fuzz_roundtrip", base91_fuzz::check_roundtrip),
    ("fuzz_unchecked", base91_fuzz::check_unchecked),
    ("fuzz_legacy", base91_fuzz::check_legacy),
];

#[test]
fn corpus_replays_cleanly() {
    base91_testkit::pending::quiet_expected_panics();
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let mut failed = Vec::new();
    for (target, check) in TARGETS {
        let dir = corpus.join(target);
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        assert!(!files.is_empty(), "empty corpus: {}", dir.display());
        for file in &files {
            let data = std::fs::read(file).unwrap();
            if panic::catch_unwind(AssertUnwindSafe(|| check(&data))).is_err() {
                failed.push(format!(
                    "{target}/{}",
                    file.file_name().unwrap().to_string_lossy()
                ));
            }
        }
        eprintln!("[corpus] {target}: {} inputs", files.len());
    }
    assert!(failed.is_empty(), "corpus inputs failing: {failed:#?}");
}
