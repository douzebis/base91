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
    // the C reference and the Rust C API side-by-side without a name clash.
    //
    // Requires: gcc (or compatible CC), objcopy, ar on PATH.
    // -----------------------------------------------------------------------
    #[cfg(feature = "c-compat-tests")]
    compile_c_reference();

    // -----------------------------------------------------------------------
    // cbindgen — generate include/base91.h from src/c_api.rs.
    //
    // Writes to OUT_DIR/base91.h (always writable) and mirrors the result into
    // CARGO_MANIFEST_DIR/include/ for in-tree use.  The mirror is best-effort:
    // silently skipped when the manifest dir is read-only (Nix sandbox builds).
    // The whole step is skipped gracefully when cbindgen is not on PATH.
    // -----------------------------------------------------------------------
    println!("cargo:rerun-if-changed=src/c_api.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let header_path = format!("{out_dir}/base91.h");

    if let Ok(cbindgen) = which_cbindgen() {
        std::process::Command::new(cbindgen)
            .args([
                "--config",
                "cbindgen.toml",
                "--crate",
                "base91",
                "--output",
                &header_path,
            ])
            .current_dir(&manifest_dir)
            .status()
            .expect("cbindgen failed");

        // Mirror into include/ for in-tree use.
        let mirror_dir = format!("{manifest_dir}/include");
        if std::fs::create_dir_all(&mirror_dir).is_ok() {
            let _ = std::fs::copy(&header_path, format!("{mirror_dir}/base91.h"));
        }
    }
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
    // Remove any stale archive first so `ar` creates a fresh one.
    let _ = std::fs::remove_file(&lib_path);
    std::process::Command::new("ar")
        .args(["crs", &lib_path, &obj_path])
        .status()
        .expect("ar crs failed");

    println!("cargo:rustc-link-search=native={out_dir}");
    println!("cargo:rustc-link-lib=static=base91_c_ref");
}

/// Find cbindgen on PATH; return its path or an error if not found.
fn which_cbindgen() -> Result<std::ffi::OsString, ()> {
    std::env::var_os("PATH")
        .unwrap_or_default()
        .to_string_lossy()
        .split(':')
        .map(|dir| std::path::Path::new(dir).join("cbindgen"))
        .find(|p| p.exists())
        .map(|p| p.into_os_string())
        .ok_or(())
}
