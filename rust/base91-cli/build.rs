// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Build script: generates the man page and shell-completion files for base91.
//!
//! Outputs are written to `$OUT_DIR`.

use std::io;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, Command, ValueHint};
use clap_complete::{generate_to, Shell};
use clap_mangen::Man;

fn build_command() -> Command {
    Command::new("base91")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Joachim Henke (algorithm); Frederic Ruget (Rust port)")
        .about("basE91 encode or decode FILE, or standard input, to standard output.")
        .long_about(
            "basE91 encode or decode FILE, or standard input, to standard output.\n\n\
             With no FILE, or when FILE is -, read standard input.",
        )
        .after_help(
            "Examples:\n  \
             base91 file.bin > file.b91            encode binary file\n  \
             base91 -d file.b91 > file.bin         decode\n  \
             b91enc < file.bin > file.b91          encode with no line wrapping\n  \
             b91dec < file.b91 > file.bin          decode\n  \
             base91 --simd file.bin > file.b91s    encode with SIMD variant\n  \
             tar czf - dir/ | b91enc               encode tar stream",
        )
        .arg(
            Arg::new("decode")
                .short('d')
                .long("decode")
                .help("Decode data instead of encoding.")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_name("FILE")
                .help("Write output to FILE instead of standard output.")
                .value_hint(ValueHint::FilePath),
        )
        .arg(
            Arg::new("buffer")
                .short('m')
                .long("buffer")
                .value_name("SIZE")
                .help(
                    "Use SIZE bytes of memory for I/O buffers (suffixes: b, K, M). Default: 64K.",
                ),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Print statistics to stderr. Repeat (-vv) for extra verbosity.")
                .action(ArgAction::Count),
        )
        .arg(
            Arg::new("wrap")
                .short('w')
                .long("wrap")
                .value_name("COLS")
                .help(
                    "Wrap encoded lines after COLS characters (0 = no wrap; default 76). \
                     With --simd, COLS must be a multiple of 32. \
                     Has no effect when decoding.",
                ),
        )
        .arg(
            Arg::new("simd")
                .long("simd")
                .help(
                    "Use the SIMD fixed-width variant for encoding. \
                     Output begins with '-' and uses the SIMD alphabet \
                     (0x23-0x26, 0x28-0x7E); not compatible with legacy \
                     Henke decoders. Ignored when decoding.",
                )
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("file")
                .value_name("FILE")
                .help("Input file (default: stdin).")
                .value_hint(ValueHint::FilePath),
        )
}

fn generate_man(cmd: &Command, out_dir: &Path) -> io::Result<()> {
    let man = Man::new(cmd.clone())
        .title("BASE91")
        .section("1")
        .date("2026-03")
        .source("base91")
        .manual("User Commands");

    let mut buf = Vec::new();
    man.render(&mut buf)?;

    std::fs::write(out_dir.join("base91.1"), &buf)?;
    Ok(())
}

fn generate_completions(cmd: &mut Command, out_dir: &Path) -> io::Result<()> {
    for shell in [
        Shell::Bash,
        Shell::Zsh,
        Shell::Fish,
        Shell::Elvish,
        Shell::PowerShell,
    ] {
        generate_to(shell, cmd, "base91", out_dir)?;
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let cmd = build_command();

    // --- Man page ---
    generate_man(&cmd, &out_dir)?;

    // --- Shell completions ---
    let mut cmd = cmd;
    generate_completions(&mut cmd, &out_dir)?;

    Ok(())
}
