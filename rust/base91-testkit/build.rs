// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

fn main() {
    // -----------------------------------------------------------------------
    // Upstream reference — only when the `reference` feature is enabled.
    //
    // Compiles the vendored, unmodified basE91 0.6.0 sources with the cc
    // crate. Their global symbols (the five functions and the two tables) are
    // renamed with a `ref_` prefix through -D defines, which apply to both
    // base91.c and the base91.h it includes. The sources stay pristine, and
    // cannot clash with base91-rs's `c-compat-tests` object.
    // -----------------------------------------------------------------------
    #[cfg(feature = "reference")]
    compile_upstream_reference();
}

#[cfg(feature = "reference")]
fn compile_upstream_reference() {
    const DIR: &str = "reference/basE91-0.6.0";
    println!("cargo:rerun-if-changed={DIR}/base91.c");
    println!("cargo:rerun-if-changed={DIR}/base91.h");

    let mut build = cc::Build::new();
    build.file(format!("{DIR}/base91.c")).include(DIR);
    for sym in [
        "basE91_init",
        "basE91_encode",
        "basE91_encode_end",
        "basE91_decode",
        "basE91_decode_end",
        "enctab",
        "dectab",
    ] {
        build.define(sym, Some(format!("ref_{sym}").as_str()));
    }
    build.compile("base91_upstream_ref");
}
