<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# base91

Multi-language implementation of [Joachim Henke's basE91](http://base91.sourceforge.net/)
binary-to-text encoding, with a high-performance Rust library, CLI, Go package,
Python bindings, and C reference.

basE91 encodes binary data into printable ASCII using only **~1.23 bytes per
input byte** — versus Base64's 1.33.  The 8% overhead reduction means smaller
payloads and fewer bytes on the wire at every hop.

## Components

| Component | Language | Description |
|---|---|---|
| `rust/base91` | Rust | `base91-rs` library crate — `no_std`, SIMD-accelerated |
| `rust/base91-cli` | Rust | `base91` CLI binary — drop-in replacement for the C CLI |
| `go/` | Go | `base91` Go package |
| `src/` | C | Henke's original C reference implementation (unmodified) |
| Python bindings | Python/Rust | `pybase91` PyPI package via PyO3 + maturin |

## Performance (Rust)

Benchmarked on an Intel Core Ultra 7 165U, on one performance core, 1 MiB
deterministic random input, version 0.3.1, one run. Absolute
figures vary by up to about 10 % between sessions on this laptop chip;
the ratios within one run hold.

### Henke (wire-compatible) path

| Variant | Encode | Decode |
|---|---|---|
| `encode` / `decode` | 886 MiB/s | 1182 MiB/s |
| `encode_slice` / `decode_slice` | 859 MiB/s | 1241 MiB/s |
| `base91` v0.1.0 (dnsl48) | 657 MiB/s | 814 MiB/s |

### SIMD fixed-width variant (`--simd`)

A non-Henke, fixed-width 13-bit block format that enables SIMD parallelism.
Output begins with `-` and uses a contiguous 91-char alphabet (0x23–0x26,
0x28–0x7E) that omits `'`, making output safe to single-quote in any POSIX
shell.

| Kernel | Encode | Decode |
|---|---|---|
| scalar | 2.51 GiB/s | 2.37 GiB/s |
| SSE4.1 / NEON | 6.63 GiB/s | 9.97 GiB/s |
| AVX2 | 12.7 GiB/s | 16.5 GiB/s |

SIMD streams are decoded strictly: damage is reported with its offset
instead of producing wrong data.

## CLI quick start

```sh
base91 file.bin > file.b91          # encode (Henke, wrap at 76 cols)
base91 -d file.b91 > file.bin       # decode
base91 --simd file.bin > file.b91s  # encode with SIMD variant
base91 -d file.b91s > file.bin      # decode either format (auto-detected)
b91enc < file.bin > file.b91        # encode with no line wrapping
b91dec < file.b91 > file.bin        # decode
```

## Rust library quick start

```toml
[dependencies]
base91-rs = "0.3"
```

```rust
// Henke one-shot
let encoded = base91::encode(b"Hello, world!");
let decoded = base91::decode(&encoded);

// SIMD one-shot
use base91::simd::{self, SimdLevel};
let encoded = simd::encode(b"Hello, world!", SimdLevel::default(), 0).unwrap();
let decoded = simd::decode(&encoded, SimdLevel::default()).unwrap(); // Result<_, base91::Error>
```

## Go quick start

```go
import "github.com/douzebis/base91/go"

encoded := base91.Encode([]byte("Hello, world!"))
decoded, _ := base91.Decode(encoded)
```

## Python quick start

```sh
pip install pybase91
```

```python
import pybase91
encoded = pybase91.encode(b"Hello, world!")
decoded = pybase91.decode(encoded)
```

## Building with Nix

```sh
nix-build          # builds all components and runs all tests
nix-shell          # development shell
```

## License

MIT — see `LICENSES/MIT.txt`.
Original C implementation © Joachim Henke, preserved unchanged in `src/`.
