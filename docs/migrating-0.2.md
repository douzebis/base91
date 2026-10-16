<!--
SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)

SPDX-License-Identifier: MIT
-->

# Migrating base91-rs from 0.2 to 0.3

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

