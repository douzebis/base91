<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# base91-rs

**base91, but faster: a SIMD variant at 25× the speed.**

[![crates.io](https://img.shields.io/crates/v/base91-rs.svg)](https://crates.io/crates/base91-rs)
[![docs.rs](https://img.shields.io/docsrs/base91-rs)](https://docs.rs/base91-rs)
[![license](https://img.shields.io/crates/l/base91-rs.svg)](https://github.com/douzebis/base91/blob/main/LICENSES/MIT.txt)

base91 encodes binary data as printable ASCII, like base64, but more
compactly: the overhead is at most 23% (base64: 33%). This crate
implements Joachim Henke's basE91 format, and a fixed-width variant of it
that SIMD instructions (SSE4.1, AVX2, NEON) encode and decode at several
GiB/s.

## Speed

| Configuration | Encode | Decode |
|---|---|---|
| SIMD variant, AVX2 | **16.6 GiB/s** | **19.6 GiB/s** |
| SIMD variant, AVX2, lines of 64 | 13.9 GiB/s | 18.3 GiB/s |
| SIMD variant, SSE4.1 / NEON | 8.45 GiB/s | 10.7 GiB/s |
| SIMD variant, scalar | 2.59 GiB/s | 2.42 GiB/s |
| Henke's format | 894 MiB/s | 1287 MiB/s |
| Henke's format, the `base91` crate 0.1.0 | 703 MiB/s | 720 MiB/s |

Intel Core Ultra 7 165U, one performance core, 1 MiB of random data,
base91-rs 0.3.3, criterion (`base91-bench`); decode speed is measured on
the encoded bytes.

## Features

- Two formats: Henke's, byte-compatible with every base91 decoder, and the
  SIMD variant. The SIMD kernel is picked at run time.
- `no_std`, and without an allocator: the slice API writes into your
  buffers, and checks their size first.
- Streaming encoders and decoders, and `std::io` adapters.
- Strict decoding of SIMD streams: damage is an error, with its offset.
- Also: the command ([`base91-cli`](https://crates.io/crates/base91-cli):
  `base91`, `b91enc`, `b91dec`), Python bindings
  ([`pybase91`](https://pypi.org/project/pybase91/)) and a C library
  (`libbase91`, with Henke's basE91 API, binary compatible).

## Examples

```sh
cargo add base91-rs   # imported as `base91`
```

The SIMD variant:

```rust
use base91::simd::{self, SimdLevel};

let encoded = simd::encode(b"Hello, world!", SimdLevel::default(), 0).unwrap();
assert_eq!(encoded, b"-r2T-Er|#{eyjjUg/");

let decoded = simd::decode(&encoded, SimdLevel::default()).unwrap();
assert_eq!(decoded, b"Hello, world!");
```

Henke's format:

```rust
let encoded = base91::encode(b"Hello, world!");
assert_eq!(encoded, b">OwJh>}A\"=r@@Y?F");

assert_eq!(base91::decode(&encoded), b"Hello, world!");
```

Streaming, slices without an allocator, errors, `std::io` and `no_std`:
see [the documentation](https://docs.rs/base91-rs).

## Which format?

- **Henke's format**, for any base91 decoder.
- **The SIMD variant**, for speed. Its streams start with `-` and contain
  no `'`, so they are safe in single-quoted shell strings. Read by this
  crate, the `base91` command and the bindings; the decoders of
  `base91::simd` read both formats and tell them apart.

More: [the SIMD variant's wire format](https://github.com/douzebis/base91/blob/main/docs/specs/0008-wire-format.md),
[the changelog](https://github.com/douzebis/base91/blob/main/CHANGELOG.md),
[migrating from 0.2](https://github.com/douzebis/base91/blob/main/docs/migrating-0.2.md).

## License

MIT.

## Acknowledgements

The basE91 format and its original C implementation are by Joachim Henke
(<http://base91.sourceforge.net/>, BSD license).
