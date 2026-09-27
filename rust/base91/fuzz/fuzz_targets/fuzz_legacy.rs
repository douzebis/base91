// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! libFuzzer target; the checks live in `base91_fuzz::check_legacy` (spec 0009 §6).

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| base91_fuzz::check_legacy(data));
