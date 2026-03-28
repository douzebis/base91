// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

fn main() {
    // -----------------------------------------------------------------------
    // C reference — only when the `c-compat-tests` feature is enabled.
    //
    // Compiles src/base91.c (two levels up from the crate root) to an object
    // file, renames its five public symbols from `basE91_*` to `c_basE91_*`
    // with objcopy, then packs it into a static lib so the bench can call both
    // the C reference and the Rust implementation side-by-side without a name
    // clash.
    //
    // Requires: gcc (or compatible CC), objcopy, ar on PATH.
    // -----------------------------------------------------------------------
    #[cfg(feature = "c-compat-tests")]
    compile_c_reference();
}

#[cfg(feature = "c-compat-tests")]
fn compile_c_reference() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let obj_path = format!("{out_dir}/base91_c_ref.o");
    let lib_path = format!("{out_dir}/libbase91_c_ref.a");

    println!("cargo:rerun-if-changed=../../src/base91.c");
    println!("cargo:rerun-if-changed=../../src/base91.h");

    // 1. Compile to an object file via the cc crate's compiler detection.
    let compiler = cc::Build::new()
        .file("../../src/base91.c")
        .opt_level(2)
        .flag("-fno-plt")
        .get_compiler();

    std::process::Command::new(compiler.path())
        .args(compiler.args())
        .args(["-c", "../../src/base91.c", "-o", &obj_path])
        .status()
        .expect("C compilation failed");

    // 2. Rename symbols: basE91_* → c_basE91_*
    // Two separate objcopy passes: first rename (adds new name, keeps old),
    // then strip the now-redundant old names.
    let syms = [
        "basE91_init",
        "basE91_encode",
        "basE91_encode_end",
        "basE91_decode",
        "basE91_decode_end",
    ];
    let mut rename_args: Vec<String> = Vec::new();
    let mut strip_args: Vec<String> = Vec::new();
    for sym in syms {
        rename_args.extend(["--redefine-sym".into(), format!("{sym}=c_{sym}")]);
        strip_args.extend(["--strip-symbol".into(), sym.into()]);
    }
    rename_args.push(obj_path.clone());
    strip_args.push(obj_path.clone());
    std::process::Command::new("objcopy")
        .args(&rename_args)
        .status()
        .expect("objcopy --redefine-sym failed");
    std::process::Command::new("objcopy")
        .args(&strip_args)
        .status()
        .expect("objcopy --strip-symbol failed");

    // 3. Pack into a static lib and tell cargo to link it.
    let _ = std::fs::remove_file(&lib_path);
    std::process::Command::new("ar")
        .args(["crs", &lib_path, &obj_path])
        .status()
        .expect("ar crs failed");

    println!("cargo:rustc-link-search=native={out_dir}");
    println!("cargo:rustc-link-lib=static=base91_c_ref");
}
