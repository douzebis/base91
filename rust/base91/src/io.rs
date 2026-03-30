// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! `std::io` adapters: [`EncoderWriter`] and [`DecoderReader`].

use std::io::{self, BufWriter, Read, Write};

use crate::codec::Encoder;
use crate::encode_size_hint;
use crate::simd::Decoder;

const BUF_SIZE: usize = 8 * 1024;

// ---------------------------------------------------------------------------
// EncoderWriter
// ---------------------------------------------------------------------------

/// A [`Write`] adapter that basE91-encodes everything written to it and
/// forwards the encoded bytes to an inner writer.
///
/// Data is buffered internally (8 KiB) to amortise the cost of small writes
/// against the inner writer.  Call [`finish`][EncoderWriter::finish] to flush
/// the encoder state and the internal buffer; dropping without calling
/// `finish` will silently discard any buffered output.
///
/// # Example
///
/// ```rust
/// use base91::io::EncoderWriter;
/// use std::io::Write;
///
/// let mut enc = EncoderWriter::new(Vec::new());
/// enc.write_all(b"Hello, world!").unwrap();
/// let encoded = enc.finish().unwrap();
/// assert_eq!(base91::decode(&encoded), b"Hello, world!");
/// ```
pub struct EncoderWriter<W: Write> {
    encoder: Encoder,
    // BufWriter amortises writes to the inner sink.
    inner: BufWriter<W>,
    // Scratch buffer: holds encoded output before it is passed to BufWriter.
    scratch: Vec<u8>,
}

impl<W: Write> EncoderWriter<W> {
    /// Create a new `EncoderWriter` wrapping `inner`.
    pub fn new(inner: W) -> Self {
        Self {
            encoder: Encoder::new(),
            inner: BufWriter::with_capacity(BUF_SIZE, inner),
            scratch: Vec::with_capacity(encode_size_hint(BUF_SIZE)),
        }
    }

    /// Flush the encoder state (emit the trailing 0–2 bytes), flush the
    /// internal buffer, and return the inner writer.
    ///
    /// Must be called to ensure all encoded output is written; dropping
    /// without calling `finish` will silently lose trailing bytes.
    pub fn finish(mut self) -> io::Result<W> {
        self.scratch.clear();
        self.encoder.finish(&mut self.scratch);
        self.inner.write_all(&self.scratch)?;
        self.inner.flush()?;
        // BufWriter::into_inner flushes and unwraps, returning any write error.
        self.inner.into_inner().map_err(|e| e.into_error())
    }
}

impl<W: Write> Write for EncoderWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.scratch.clear();
        self.encoder.encode(buf, &mut self.scratch);
        self.inner.write_all(&self.scratch)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

// ---------------------------------------------------------------------------
// DecoderReader
// ---------------------------------------------------------------------------

/// A [`Read`] adapter that decodes data pulled from an inner reader.
///
/// The format is detected automatically from the first byte: a leading `-`
/// selects the SIMD fixed-width variant; anything else is decoded as a Henke
/// stream.  Non-alphabet bytes (e.g. newlines) are silently skipped.
///
/// # Example
///
/// ```rust
/// use base91::io::DecoderReader;
/// use std::io::Read;
///
/// let encoded = base91::encode(b"Hello, world!");
/// let mut dec = DecoderReader::new(encoded.as_slice());
/// let mut out = Vec::new();
/// dec.read_to_end(&mut out).unwrap();
/// assert_eq!(out, b"Hello, world!");
/// ```
pub struct DecoderReader<R: Read> {
    decoder: Decoder,
    inner: R,
    // Decoded-but-not-yet-consumed bytes.
    decoded: Vec<u8>,
    decoded_pos: usize,
    // Read buffer for pulling from the inner reader.
    read_buf: Box<[u8; BUF_SIZE]>,
    done: bool,
}

impl<R: Read> DecoderReader<R> {
    /// Create a new `DecoderReader` wrapping `inner`.
    pub fn new(inner: R) -> Self {
        Self {
            decoder: Decoder::new(crate::simd::SimdLevel::default()),
            inner,
            decoded: Vec::with_capacity(BUF_SIZE),
            decoded_pos: 0,
            read_buf: Box::new([0u8; BUF_SIZE]),
            done: false,
        }
    }

    /// Unwrap and return the inner reader.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for DecoderReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Serve from the decoded buffer first.
        if self.decoded_pos < self.decoded.len() {
            let available = self.decoded.len() - self.decoded_pos;
            let n = available.min(buf.len());
            buf[..n].copy_from_slice(&self.decoded[self.decoded_pos..self.decoded_pos + n]);
            self.decoded_pos += n;
            return Ok(n);
        }

        if self.done {
            return Ok(0);
        }

        // Refill: pull a chunk from the inner reader and decode it.
        self.decoded.clear();
        self.decoded_pos = 0;

        let n = self.inner.read(self.read_buf.as_mut())?;
        if n == 0 {
            // Inner reader is exhausted: flush decoder state.
            // Replace the decoder with a fresh default so we can take
            // ownership of the old one (finish() consumes self).
            core::mem::take(&mut self.decoder).finish(&mut self.decoded);
            self.done = true;
        } else {
            self.decoder.decode(&self.read_buf[..n], &mut self.decoded);
        }

        // Serve from the freshly decoded buffer.
        let available = self.decoded.len().min(buf.len());
        buf[..available].copy_from_slice(&self.decoded[..available]);
        self.decoded_pos = available;
        Ok(available)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn encode_via_writer(input: &[u8]) -> Vec<u8> {
        let mut enc = EncoderWriter::new(Vec::new());
        enc.write_all(input).unwrap();
        enc.finish().unwrap()
    }

    fn decode_via_reader(input: &[u8]) -> Vec<u8> {
        let mut dec = DecoderReader::new(input);
        let mut out = Vec::new();
        dec.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn round_trip_empty() {
        assert_eq!(decode_via_reader(&encode_via_writer(b"")), b"");
    }

    #[test]
    fn round_trip_all_bytes() {
        let input: Vec<u8> = (0u8..=255).collect();
        assert_eq!(decode_via_reader(&encode_via_writer(&input)), input);
    }

    #[test]
    fn writer_matches_one_shot() {
        let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        assert_eq!(encode_via_writer(&input), crate::encode(&input));
    }

    #[test]
    fn reader_matches_one_shot() {
        let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let encoded = crate::encode(&input);
        assert_eq!(decode_via_reader(&encoded), crate::decode(&encoded));
    }

    #[test]
    fn writer_chunked_matches_one_shot() {
        let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let expected = crate::encode(&input);

        for chunk_size in [1, 7, 13, 64, 512, 1024, 4096] {
            let mut enc = EncoderWriter::new(Vec::new());
            for chunk in input.chunks(chunk_size) {
                enc.write_all(chunk).unwrap();
            }
            assert_eq!(enc.finish().unwrap(), expected, "chunk_size={chunk_size}");
        }
    }

    #[test]
    fn reader_chunked_matches_one_shot() {
        let input: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let encoded = crate::encode(&input);
        let expected = crate::decode(&encoded);

        // Read in small chunks to exercise the refill path.
        for read_size in [1, 7, 13, 64, 512] {
            let mut dec = DecoderReader::new(encoded.as_slice());
            let mut out = Vec::new();
            let mut tmp = vec![0u8; read_size];
            loop {
                let n = dec.read(&mut tmp).unwrap();
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&tmp[..n]);
            }
            assert_eq!(out, expected, "read_size={read_size}");
        }
    }
}
