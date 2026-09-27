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

    compile_upstream_cli(DIR);
}

/// Builds upstream's own command-line tool (`cli.c` + `base91.c`, no
/// renaming) into an executable in OUT_DIR, and publishes its path to the
/// crate as `BASE91_REFERENCE_CLI` (spec 0009 §9).
#[cfg(feature = "reference")]
fn compile_upstream_cli(dir: &str) {
    println!("cargo:rerun-if-changed={dir}/cli.c");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let exe = format!("{out_dir}/base91-upstream-0.6.0");
    // The target's C compiler, as configured for this build (a cross
    // compiler when cross-compiling; the executable is then only built, not
    // run).
    let status = cc::Build::new()
        .get_compiler()
        .to_command()
        .args(["-O2", "-o", &exe])
        .arg(format!("{dir}/cli.c"))
        .arg(format!("{dir}/base91.c"))
        .arg(format!("-I{dir}"))
        .status()
        .expect("cannot run the C compiler");
    assert!(status.success(), "building the upstream CLI failed");
    println!("cargo:rustc-env=BASE91_REFERENCE_CLI={exe}");
}
