<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# base91-cli

**base91, but faster: a SIMD variant at 25× the speed.**

base91 encodes binary data as printable ASCII, like base64, but more
compactly: the overhead is at most 23% (base64: 33%). `base91` is the
command: a drop-in replacement for Joachim Henke's C tool (same options,
same output), with `--simd` for a fixed-width variant that SIMD
instructions (SSE4.1, AVX2, NEON) encode and decode at several GiB/s.

## Speed

| Configuration | Encode | Decode |
|---|---|---|
| SIMD variant, AVX2 | **16.6 GiB/s** | **19.6 GiB/s** |
| SIMD variant, AVX2, lines of 64 | 13.9 GiB/s | 18.3 GiB/s |
| SIMD variant, SSE4.1 / NEON | 8.45 GiB/s | 10.7 GiB/s |
| SIMD variant, scalar | 2.59 GiB/s | 2.42 GiB/s |
| Henke's format | 894 MiB/s | 1287 MiB/s |
| Henke's format, the `base91` crate 0.1.0 | 703 MiB/s | 720 MiB/s |

The library's throughput (base91-rs 0.3.3) on an Intel Core Ultra 7 165U,
one performance core, 1 MiB of random data, criterion (`base91-bench`);
decode speed is measured on the encoded bytes.

## Features

- `base91`, and `b91enc` / `b91dec` (encode without line wrapping,
  decode): Henke's three commands, his options and defaults.
- `--simd`: the SIMD variant, streamed in bounded memory. `-d` decodes
  either format.
- Strict decoding of SIMD streams: damage is reported with its offset
  (`base91: invalid character at offset 1234`), exit status 1.
- A man page and bash, zsh, fish, elvish and PowerShell completions,
  printed by the binary.

## Install

```sh
cargo install base91-cli
```

or `nix-env -iA nixpkgs.base91` (nixpkgs).

## Examples

```sh
$ printf 'Hello, world!' | b91enc
>OwJh>}A"=r@@Y?F
$ printf 'Hello, world!' | base91 --simd -w 0
-r2T-Er|#{eyjjUg/
$ printf '%s' '-r2T-Er|#{eyjjUg/' | base91 -d
Hello, world!

$ base91 file.bin > file.b91             # Henke's format, lines of 76
$ base91 --simd -w 0 file.bin > file.b91s
$ tar czf - dir/ | b91enc > archive.b91
```

Options: `-d` (decode), `-o FILE`, `-w COLS` (line width; 0 for none;
default 76, or 64 with `--simd`, where it must be a multiple of 32),
`-m SIZE` (buffer, e.g. `64K`), `-v` (statistics on standard error),
`--simd`. Without FILE, or with `-`, `base91` reads standard input.

## Which format?

- **Henke's format**, for any base91 decoder.
- **The SIMD variant**, for speed. Its streams start with `-` and contain
  no `'`, so they are safe in single-quoted shell strings. `base91 -d`
  recognizes it; other decoders of Henke's format do not.

## Shell integration

```sh
# Man page (per user)
mkdir -p ~/.local/share/man/man1
base91 --man > ~/.local/share/man/man1/base91.1

# Completions
base91 --completions bash > ~/.local/share/bash-completion/completions/base91
base91 --completions zsh  > ~/.zfunc/_base91    # ~/.zfunc in $fpath
base91 --completions fish > ~/.config/fish/completions/base91.fish
```

`--completions` also takes `elvish` and `powershell`.

## License

MIT.

## Acknowledgements

The basE91 format and its original C implementation are by Joachim Henke
(<http://base91.sourceforge.net/>, BSD license).
