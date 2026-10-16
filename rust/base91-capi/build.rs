// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

fn main() {
    // The shared library's SONAME (spec 0014 §3): the Henke part of the ABI
    // is stable, the new API not yet promised, hence 0. On macOS, the
    // install name is set when the library is installed.
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "linux" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libbase91.so.0");
    }
}
