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

- **`no_std`** compatible (disable default features)
- **SIMD-accelerated** — optional fixed-width variant with SSE4.1, AVX2, and
  NEON kernels; up to **7.68 GiB/s encode / 8.57 GiB/s decode** on x86_64
- **Streaming API** — feed data in chunks, no full-buffer requirement
- **`std::io` adapters** — `EncoderWriter<W>` and `DecoderReader<R>` (feature `io`, on by default)
- **Unsafe unchecked variants** — `encode_unchecked` / `decode_unchecked` for callers that
  pre-allocate with the provided size hints
- **C-compatible ABI** — `basE91_*` functions with layout identical to the C reference

## Performance

Benchmarked on an Intel Core Ultra 7 165U, pinned to P-cores (no HT contention),
1 MiB deterministic random input (seed `0xdeadbeef_cafebabe`).
Decode throughput for the SIMD variant is measured on encoded bytes (≈1.23× input).

### Henke (wire-compatible) path

| Crate / variant | Encode | Decode |
|---|---|---|
| **`base91-rs` safe (`encode`)** | **841 MiB/s** | **872 MiB/s** |
| **`base91-rs` unchecked (`encode_unchecked`)** | **993 MiB/s** | **1.15 GiB/s** |
| `base91` v0.1.0 (dnsl48) | 624 MiB/s | 784 MiB/s |

**~1.35× faster encode, ~1.47× faster decode** vs the next most popular crate
on the safe API.  The unchecked API reaches **~1.59× / ~1.47×**.

### SIMD fixed-width variant (`--simd` flag / `simd` module)

Uses a non-Henke, fixed-width 13-bit block layout that makes the block
boundaries predictable — unlocking SIMD parallelism that is impossible on
the Henke path.  The encoded stream is prefixed with `-` to distinguish it
from Henke output.  The alphabet (0x23–0x26, 0x28–0x7E) omits `'` so
output is safe to single-quote in any POSIX shell.

| Path | Encode | Decode | vs. Henke unchecked |
|---|---|---|---|
| scalar fixed-width | ~1.52 GiB/s | ~2.31 GiB/s | 1.5× / 2.0× |
| simd128 (SSE4.1 / NEON) | ~4.40 GiB/s | ~6.25 GiB/s | 4.4× / 5.4× |
| simd256 (AVX2) | ~7.68 GiB/s | ~8.57 GiB/s | 7.7× / 7.5× |

The SIMD kernel auto-selects the best available level at runtime (AVX2 →
SSE4.1 → NEON → scalar).

## Usage

```toml
[dependencies]
base91-rs = "0.2"
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

let encoded = simd::encode(b"some input", SimdLevel::default(), 0);
let decoded = simd::decode(&encoded, SimdLevel::default()).unwrap();
assert_eq!(decoded, b"some input");
```

### Streaming SIMD

```rust
use base91::simd::{self, Encoder as SimdEncoder, SimdLevel};

let input = b"some large input";
let mut out = Vec::new();
let mut enc = SimdEncoder::new(SimdLevel::default());
enc.encode(input, &mut out);
enc.finish(&mut out);
```

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

```toml
[dependencies]
base91-rs = { version = "0.2", default-features = false }
```

## License

MIT
