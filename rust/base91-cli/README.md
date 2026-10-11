<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# base91-cli

Command-line encoder/decoder for [Joachim Henke's basE91](http://base91.sourceforge.net/)
binary-to-text encoding.

basE91 encodes binary data into printable ASCII using only **~1.23 bytes per
input byte** — versus Base64's 1.33.  Drop-in replacement for the C reference
`base91` binary, with an optional SIMD-accelerated fixed-width variant that
reaches **12.7 GiB/s encode / 16.5 GiB/s decode** on AVX2 hardware.

## Installation

```sh
cargo install base91-cli
```

Or with a package manager (nix, brew, apt, …):

```sh
nix-env -iA base91          # NixOS / nix
```

## Usage

```
base91 [OPTIONS] [FILE]
```

| Flag | Description |
|---|---|
| `-d`, `--decode` | Decode instead of encoding |
| `-o`, `--output FILE` | Write output to FILE instead of stdout |
| `-w`, `--wrap COLS` | Wrap encoded lines at COLS chars (0 = no wrap; default 76, or 64 with `--simd`) |
| `--simd` | Use the SIMD fixed-width variant (faster; different wire format) |
| `-m`, `--buffer SIZE` | I/O buffer size (suffixes: b, K, M; default 64K) |
| `-v`, `--verbose` | Print statistics to stderr (`-vv` for extra detail) |
| `--completions SHELL` | Print shell completions for SHELL and exit |

With no FILE, or when FILE is `-`, reads standard input.

## Examples

```sh
# Encode / decode a file (Henke wire format)
base91 file.bin > file.b91
base91 -d file.b91 > file.bin

# Encode / decode with the SIMD variant
base91 --simd file.bin > file.b91s
base91 -d file.b91s > file.bin

# Pipe-friendly aliases (no line wrapping)
b91enc < file.bin > file.b91
b91dec < file.b91 > file.bin

# Encode a tar stream
tar czf - dir/ | b91enc > archive.b91
```

`b91enc` and `b91dec` are symlinks to (or copies of) the `base91` binary.
When invoked as `b91enc`, the default is encode with no line wrapping.
When invoked as `b91dec`, the default is decode.

## SIMD variant

`--simd` selects a non-Henke fixed-width 13-bit block format that enables
SIMD parallelism.  The output is **not wire-compatible** with the Henke format,
but the decoder auto-detects the format — plain `base91 -d` decodes both.

- Output begins with `-` to distinguish it from Henke streams.
- Alphabet: 0x23–0x26, 0x28–0x7E (91 contiguous printable ASCII chars,
  omitting `'`).  Output is safe to single-quote in any POSIX shell.
- Default wrap: 64 columns (multiple of 32 required; 0 = no wrap).
- Encoding streams: memory use stays bounded whatever the input size.
- Decoding is strict: a damaged SIMD stream (a byte outside the alphabet, a
  broken line layout, a truncated payload) is reported with its offset,
  e.g. `base91: invalid character at offset 1234`, and `base91 -d` exits
  with status 1. Legacy (Henke) streams are decoded as the C reference
  does, ignoring bytes outside the alphabet.

Library throughput (Intel Core Ultra 7 165U, one performance core, 1 MiB,
0.3.2; decode measured on encoded bytes):

| Kernel | Encode | Decode |
|---|---|---|
| scalar | 2.45 GiB/s | 2.31 GiB/s |
| SSE4.1 / NEON | 8.18 GiB/s | 10.2 GiB/s |
| AVX2 | 16.0 GiB/s | 18.8 GiB/s |

The best available kernel is selected automatically at runtime.

## Man page

The binary prints its own man page (roff) with the hidden `--man` option:

```sh
# Per-user
mkdir -p ~/.local/share/man/man1
base91 --man > ~/.local/share/man/man1/base91.1
mandb   # or: man --update-cache  (may not be needed on macOS)

# System-wide (requires root)
base91 --man | sudo install -Dm644 /dev/stdin /usr/local/share/man/man1/base91.1
```

## Shell completions

The binary prints its completion scripts with the hidden `--completions SHELL`
option:

```sh
# Bash
base91 --completions bash > ~/.local/share/bash-completion/completions/base91

# Zsh  (ensure ~/.zfunc is in $fpath)
base91 --completions zsh  > ~/.zfunc/_base91

# Fish
base91 --completions fish > ~/.config/fish/completions/base91.fish

# Elvish
base91 --completions elvish > ~/.config/elvish/lib/completions/base91.elv

# PowerShell
base91 --completions powershell >> $PROFILE
```

## License

MIT — algorithm by Joachim Henke, Rust port by Frederic Ruget.
