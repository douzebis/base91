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
reaches **7.68 GiB/s encode / 8.57 GiB/s decode** on AVX2 hardware.

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
base91 --simd -d file.b91s > file.bin

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
SIMD parallelism.  The output is **not wire-compatible** with the Henke format
and must be decoded with `base91 --simd -d`.

- Output begins with `-` to distinguish it from Henke streams.
- Alphabet: 0x23–0x26, 0x28–0x7E (91 contiguous printable ASCII chars,
  omitting `'`).  Output is safe to single-quote in any POSIX shell.
- Default wrap: 64 columns (multiple of 32 required; 0 = no wrap).

| Kernel | Encode | Decode |
|---|---|---|
| scalar | ~1.52 GiB/s | ~2.31 GiB/s |
| SSE4.1 / NEON | ~4.40 GiB/s | ~6.25 GiB/s |
| AVX2 | ~7.68 GiB/s | ~8.57 GiB/s |

The best available kernel is selected automatically at runtime.

## Man page

The man page is pre-generated and included in the repository at
[`man/man1/base91.1`](man/man1/base91.1).

To install it:

```sh
# System-wide (requires root)
sudo cp man/man1/base91.1 /usr/local/share/man/man1/

# Per-user
mkdir -p ~/.local/share/man/man1
cp man/man1/base91.1 ~/.local/share/man/man1/
mandb  # or: man --update-cache  (may not be needed on macOS)
```

## Shell completions

Pre-generated completion files are included at
[`completions/`](completions/):

| Shell | File |
|---|---|
| Bash | `completions/bash/base91.bash` |
| Zsh | `completions/zsh/_base91` |
| Fish | `completions/fish/base91.fish` |
| Elvish | `completions/elvish/base91.elv` |
| PowerShell | `completions/powershell/_base91.ps1` |

### Bash

```sh
cp completions/bash/base91.bash ~/.local/share/bash-completion/completions/base91
```

### Zsh

```sh
cp completions/zsh/_base91 ~/.zfunc/_base91
# Ensure ~/.zfunc is in your $fpath (add to ~/.zshrc if needed):
# fpath=(~/.zfunc $fpath)
```

### Fish

```sh
cp completions/fish/base91.fish ~/.config/fish/completions/base91.fish
```

You can also generate completions at runtime for the installed binary:

```sh
base91 --completions bash   > ~/.local/share/bash-completion/completions/base91
base91 --completions zsh    > ~/.zfunc/_base91
base91 --completions fish   > ~/.config/fish/completions/base91.fish
```

## License

MIT — algorithm by Joachim Henke, Rust port by Frederic Ruget.
