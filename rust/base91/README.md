<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# base91-rs

A fast, `no_std`-compatible Rust implementation of
[Joachim Henke's basE91](http://base91.sourceforge.net/) binary-to-text
encoding. Wire-format compatible with the C reference implementation.

## Quick start

```rust
let encoded = base91::encode(b"Hello, world!");
let decoded = base91::decode(&encoded);
assert_eq!(decoded, b"Hello, world!");
```

## Why basE91?

basE91 encodes binary data into printable ASCII using only **~1.23 bytes per
input byte** — versus Base64's 1.33.  That 8% overhead reduction means
smaller payloads, fewer bytes on the wire, and less energy consumed at every
hop from RAM to NIC to network switch.

## Features

- **`no_std`** compatible, with or without `alloc` (see [below](#no_std))
- **SIMD-accelerated** — optional fixed-width variant with SSE4.1, AVX2, and
  NEON kernels; up to **12.7 GiB/s encode / 16.5 GiB/s decode** on x86_64
- **Strict SIMD decoding** — damaged SIMD streams are reported as an
  `Error`, with the kind of damage and its offset, never decoded to garbage
- **Slice API** — `encode_slice` / `decode_slice` write into caller buffers,
  with no allocator (sizes checked before writing)
- **Streaming API** — feed data in chunks, no full-buffer requirement
- **`std::io` adapters** — `EncoderWriter<W>` and `DecoderReader<R>` (feature `io`, on by default)

## Performance

Benchmarked on an Intel Core Ultra 7 165U, on one performance core (no
hyperthread contention), 1 MiB deterministic random input (seed
`0xdeadbeef_cafebabe`), `cargo bench --bench throughput`, version 0.3.2,
one run. Absolute
figures vary by up to about 10 % between sessions on this laptop chip;
the ratios within one run hold.
Decode throughput is measured on encoded bytes (≈1.23× input).

### Henke (wire-compatible) path

| Crate / variant | Encode | Decode |
|---|---|---|
| **`base91-rs` (`encode` / `decode`)** | **892 MiB/s** | **1244 MiB/s** |
| **`base91-rs` slice API (`encode_slice` / `decode_slice`)** | **929 MiB/s** | **1250 MiB/s** |
| `base91` v0.1.0 (dnsl48) | 660 MiB/s | 760 MiB/s |

**~1.4× faster encode, ~1.6× faster decode** than the next most popular
crate.

### SIMD fixed-width variant (`--simd` flag / `simd` module)

Uses a non-Henke, fixed-width 13-bit block layout that makes the block
boundaries predictable — unlocking SIMD parallelism that is impossible on
the Henke path.  The encoded stream is prefixed with `-` to distinguish it
from Henke output.  The alphabet (0x23–0x26, 0x28–0x7E) omits `'` so
output is safe to single-quote in any POSIX shell.

| Path | Encode | Decode | vs. Henke slice API |
|---|---|---|---|
| scalar fixed-width | 2.45 GiB/s | 2.31 GiB/s | 2.7× / 1.9× |
| simd128 (SSE4.1 / NEON) | 8.18 GiB/s | 10.2 GiB/s | 9.0× / 8.3× |
| simd256 (AVX2) | 16.0 GiB/s | 18.8 GiB/s | 17.6× / 15.4× |
| simd256, lines of 64 | 13.1 GiB/s | 17.6 GiB/s | 14.5× / 14.4× |

Decoding includes the strict validation. The SIMD kernel auto-selects the
best available level at runtime (AVX2 → SSE4.1 → NEON → scalar). On
efficiency cores, whose AVX2 units are half-width, AVX2 is about as fast as
SSE4.1.

## Usage

```toml
[dependencies]
base91-rs = "0.3"
```

### One-shot (Henke)

```rust
let encoded: Vec<u8> = base91::encode(b"some input");
let decoded: Vec<u8> = base91::decode(&encoded);
```

### Streaming (Henke)

```rust
use base91::{Encoder, encode_size_hint};

let input = b"some large input";
let mut out = Vec::with_capacity(encode_size_hint(input.len()));
let mut enc = Encoder::new();
enc.encode(input, &mut out);
enc.finish(&mut out);
```

### One-shot SIMD

```rust
use base91::simd::{self, SimdLevel};

// Lines of 64 characters; 0 for none. A width must be a multiple of 32.
let encoded = simd::encode(b"some input", SimdLevel::default(), 64).unwrap();
let decoded = simd::decode(&encoded, SimdLevel::default()).unwrap();
assert_eq!(decoded, b"some input");
```

`simd::decode` also decodes legacy (Henke) streams: the format is detected
from the first byte that is not whitespace (`-` for the SIMD variant).

### Errors

SIMD streams are decoded strictly. Damage is reported, with its offset in
the input:

```rust
use base91::{simd, Error};

let result = simd::decode(b"-##\"##", simd::SimdLevel::default());
assert_eq!(result, Err(Error::InvalidCharacter { offset: 3 }));
```

The classes are `InvalidCharacter`, `InvalidLayout` (a line break where
none belongs, lines of the wrong length, mixed LF / CR LF),
`InvalidLength` and `InvalidValue`; `Error` implements `Display` and
`std::error::Error`. Legacy streams never fail: like the C reference, the
decoder ignores bytes outside the alphabet.

### Without an allocator: the slice API

```rust
use base91::simd::{self, SimdLevel};

let input = b"some input";
let mut encoded = [0u8; 64];
let n = simd::encode_slice(input, SimdLevel::default(), 0, &mut encoded).unwrap();

let mut decoded = [0u8; 64];
let m = simd::decode_slice(&encoded[..n], SimdLevel::default(), &mut decoded).unwrap();
assert_eq!(&decoded[..m], input);
```

Each call checks the buffer against the size it needs before writing; a
buffer too small gives `Error::BufferTooSmall { needed }` and leaves the
buffer untouched. `simd::encoded_len(n, wrap)` is the exact encoded size,
`simd::decode_size_hint(len)` a bound on the decoded one. The legacy format
has `base91::encode_slice` / `decode_slice`, and every streaming encoder
and decoder has `encode_slice` / `decode_slice` and `finish_slice`.

### Streaming SIMD

```rust
use base91::simd::{Encoder as SimdEncoder, SimdLevel};

let input = b"some large input";
let mut out = Vec::new();
let mut enc = SimdEncoder::new(SimdLevel::default(), 64).unwrap();
for chunk in input.chunks(4) {
    enc.encode(chunk, &mut out);
}
enc.finish(&mut out);
```

The output is the one-shot encoding, however the input is split; the same
holds for `simd::Decoder`, errors included.

### `std::io` adapters

```rust
use base91::io::{EncoderWriter, DecoderReader};
use std::io::{Write, Read};

let mut enc = EncoderWriter::new(Vec::new());
enc.write_all(b"Hello, world!").unwrap();
let encoded = enc.finish().unwrap();

let mut dec = DecoderReader::new(encoded.as_slice());
let mut decoded = Vec::new();
dec.read_to_end(&mut decoded).unwrap();
assert_eq!(decoded, b"Hello, world!");
```

### `no_std`

The crate has three levels, selected with Cargo features
(`std` ⊃ `alloc` ⊃ core):

| Features | Environment | API |
|---|---|---|
| default (`std`, `io`) | hosted | everything, including the `io` adapters |
| `default-features = false, features = ["alloc"]` | `no_std` with a heap | everything but the `io` adapters |
| `default-features = false` | `no_std`, no heap | everything that does not return a `Vec` |

```toml
[dependencies]
base91-rs = { version = "0.3", default-features = false }
```

Without `alloc`, the whole slice API is available (one-shot and
streaming, both formats), with `Error`, the length functions and
`simd::detect()`: only the functions returning a `Vec` need `alloc`.

**SIMD selection.** With `std`, the SIMD kernel is chosen at run time from
the CPU's features. Without `std`, it is chosen at compile time from the
target features, so the SIMD variant uses the scalar kernel unless you
enable them, e.g. `-C target-cpu=native` or
`-C target-feature=+ssse3,+sse4.1` (x86_64), `+neon` (aarch64).

**Soft-float targets.** On targets such as `x86_64-unknown-none` or
`aarch64-unknown-none-softfloat`, the environment (typically a kernel) has
not enabled the SIMD registers. Do not enable SIMD target features there:
keep the target's defaults. The compiler then lowers the SIMD kernels to
scalar code, which is correct but slower than the scalar kernel.

## Migrating from 0.2

- `simd::encode` returns `Result<Vec<u8>, Error>`: add `?` or `.unwrap()`.
  A line width other than 0 or a multiple of 32 is now an error.
- `simd::decode` returns `Result<Vec<u8>, Error>` instead of an `Option`
  that was always `Some`. Damaged SIMD streams are now errors instead of
  garbage.
- `simd::Decoder::decode` and `finish` return `Result<(), Error>`.
- `simd::Encoder::new(level)` becomes `simd::Encoder::new(level, wrap)`,
  returning `Result` (0 for no wrapping, as before).
- `io::DecoderReader` fails with `io::ErrorKind::InvalidData` on a damaged
  SIMD stream; the `Error` is inside (`io::Error::into_inner`).
- `encode_unchecked` / `decode_unchecked` (both formats) are deprecated:
  use `encode_slice` / `decode_slice`, which check the buffer size instead
  of relying on the caller.
- `simd::scalar` is gone: `SimdLevel::Scalar` selects the portable path.
- Nothing changes for the legacy (Henke) format.

## License

MIT
