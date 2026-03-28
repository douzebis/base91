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

## Features

- **`no_std`** compatible (disable default features)
- **Streaming API** — feed data in chunks, no full-buffer requirement
- **`std::io` adapters** — `EncoderWriter<W>` and `DecoderReader<R>` (feature `io`, on by default)
- **Unsafe unchecked variants** — `encode_unchecked` / `decode_unchecked` for callers that
  pre-allocate with the provided size hints
- **C-compatible ABI** — `basE91_*` functions with layout identical to the C reference

## Performance

Benchmarked on an Intel Core Ultra 7 165U (AC, performance mode), 1 MiB random input:

| Crate | Encode | Decode |
|---|---|---|
| **`base91-rs` (this crate, safe)** | **830 MiB/s** | **1013 MiB/s** |
| **`base91-rs` (unchecked)** | **928 MiB/s** | **1220 MiB/s** |
| `base91` v0.1.0 (dnsl48) | 612 MiB/s | 767 MiB/s |

**~1.35× faster** on encode, **~1.32× faster** on decode vs the next most popular crate,
using the safe public API. The unchecked API reaches **~1.52×** / **~1.59×**.

## Usage

```toml
[dependencies]
base91-rs = "0.2"
```

### One-shot

```rust
let encoded: Vec<u8> = base91::encode(b"some input");
let decoded: Vec<u8> = base91::decode(&encoded);
```

### Streaming

```rust
use base91::{Encoder, encode_size_hint};

let input = b"some large input";
let mut out = Vec::with_capacity(encode_size_hint(input.len()));
let mut enc = Encoder::new();
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
