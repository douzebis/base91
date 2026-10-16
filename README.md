<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# base91

**base91, but faster: a SIMD variant at 25× the speed.**

base91 encodes binary data as printable ASCII with about 23% overhead
(base64: 33%). This project implements Joachim Henke's basE91 format, and a
fixed-width SIMD variant that SSE4.1, AVX2 and NEON encode and decode at
several GiB/s: a Rust library, the `base91` command, Python and C bindings,
and a Go package.

## Speed

| Configuration | Encode | Decode |
|---|---|---|
| SIMD variant, AVX2 | **16.6 GiB/s** | **19.6 GiB/s** |
| SIMD variant, AVX2, lines of 64 | 13.9 GiB/s | 18.3 GiB/s |
| SIMD variant, SSE4.1 / NEON | 8.45 GiB/s | 10.7 GiB/s |
| SIMD variant, scalar | 2.59 GiB/s | 2.42 GiB/s |
| Henke's format | 894 MiB/s | 1287 MiB/s |
| Henke's format, the `base91` crate 0.1.0 | 703 MiB/s | 720 MiB/s |

The Rust library (base91-rs 0.3.3) on an Intel Core Ultra 7 165U, one
performance core, 1 MiB of random data, criterion (`base91-bench`); decode
speed is measured on the encoded bytes.

## Components

| Component | Language | Description |
|---|---|---|
| `rust/base91` | Rust | `base91-rs` library crate — `no_std`, SIMD-accelerated |
| `rust/base91-cli` | Rust | `base91` CLI binary — drop-in replacement for the C CLI |
| `go/` | Go | `base91` Go package |
| Python bindings | Python/Rust | `pybase91` PyPI package via PyO3 + maturin |
| `rust/base91-capi` | C/Rust | `libbase91`: Henke's basE91 API, binary compatible, and an API for both formats |
| `src/` | C | Henke's original C implementation, lightly modified (the C reference) |

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

encoded = pybase91.simd_encode(b"Hello, world!")  # the SIMD variant
decoded = pybase91.simd_decode(encoded)           # either format
encoded = pybase91.encode(b"Hello, world!")       # Henke's format
```

## C quick start

`libbase91` (`rust/base91-capi`): Henke's basE91 API, binary compatible,
and the `base91_*` API for both formats (`pkg-config base91`).

```c
#include <base91.h>

char out[64];
size_t n;
if (base91_encode(BASE91_SIMD, "Hello, world!", 13, 0, out, sizeof out, &n) != BASE91_OK)
	/* base91_strerror(code) says why */;
```

## Building with Nix

```sh
nix-build          # builds all components and runs all tests
nix-shell          # development shell
```

## License

MIT (`LICENSES/MIT.txt`), except Henke's C code: in `src/` (lightly
modified) and, unmodified, in `rust/base91-testkit/reference/`
(BSD-3-Clause, `LICENSES/BSD-3-Clause.txt`).

## Acknowledgements

The basE91 format and its original C implementation are by Joachim Henke
(<http://base91.sourceforge.net/>).
