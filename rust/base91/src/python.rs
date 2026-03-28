// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! PyO3 bindings for the `pybase91` Python extension module.
//!
//! Exposes:
//! - `encode(data: bytes) -> bytes` and `decode(data: bytes) -> bytes`
//!   for one-shot use.
//! - `Encoder` and `Decoder` classes for streaming use.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3_stub_gen::derive::{gen_stub_pyclass, gen_stub_pyfunction, gen_stub_pymethods};
use pyo3_stub_gen::StubInfo;

// ---------------------------------------------------------------------------
// One-shot functions
// ---------------------------------------------------------------------------

/// Encode `data` to basE91.  Returns the encoded bytes.
///
/// The output contains only printable ASCII characters from the basE91
/// alphabet; it is safe to embed in text protocols.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn encode<'py>(py: Python<'py>, data: &Bound<'py, PyBytes>) -> Bound<'py, PyBytes> {
    PyBytes::new(py, &crate::encode(data.as_bytes()))
}

/// Decode `data` from basE91.  Returns the decoded bytes.
///
/// Non-alphabet bytes (e.g. newlines, spaces) are silently ignored,
/// matching the behaviour of the C reference implementation.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn decode<'py>(py: Python<'py>, data: &Bound<'py, PyBytes>) -> Bound<'py, PyBytes> {
    PyBytes::new(py, &crate::decode(data.as_bytes()))
}

// ---------------------------------------------------------------------------
// Streaming encoder
// ---------------------------------------------------------------------------

/// Streaming basE91 encoder.
///
/// Feed data in chunks with `update()`; call `finish()` to flush the
/// remaining bits and collect the complete encoded output.
///
/// ```python
/// enc = pybase91.Encoder()
/// enc.update(b"Hello, ")
/// enc.update(b"world!")
/// result = enc.finish()
/// ```
#[gen_stub_pyclass]
#[pyclass]
pub struct Encoder {
    inner: crate::Encoder,
    buf: Vec<u8>,
}

#[gen_stub_pymethods]
#[pymethods]
impl Encoder {
    #[new]
    pub fn new() -> Self {
        Encoder {
            inner: crate::Encoder::new(),
            buf: Vec::new(),
        }
    }

    /// Feed a chunk of data into the encoder.
    ///
    /// Partial output is buffered internally; call `finish()` to collect
    /// the complete result.
    pub fn update(&mut self, data: &Bound<'_, PyBytes>) {
        self.inner.encode(data.as_bytes(), &mut self.buf);
    }

    /// Flush remaining bits and return all encoded output accumulated
    /// since construction.
    ///
    /// The encoder is consumed by this call; further use raises
    /// `AttributeError`.
    pub fn finish<'py>(&mut self, py: Python<'py>) -> Bound<'py, PyBytes> {
        core::mem::take(&mut self.inner).finish(&mut self.buf);
        PyBytes::new(py, &core::mem::take(&mut self.buf))
    }
}

// ---------------------------------------------------------------------------
// Streaming decoder
// ---------------------------------------------------------------------------

/// Streaming basE91 decoder.
///
/// Feed encoded data in chunks with `update()`; call `finish()` to flush
/// any remaining partial value and collect the complete decoded output.
///
/// Non-alphabet bytes (e.g. whitespace) are silently ignored.
///
/// ```python
/// dec = pybase91.Decoder()
/// dec.update(encoded[:10])
/// dec.update(encoded[10:])
/// result = dec.finish()
/// ```
#[gen_stub_pyclass]
#[pyclass]
pub struct Decoder {
    inner: crate::Decoder,
    buf: Vec<u8>,
}

#[gen_stub_pymethods]
#[pymethods]
impl Decoder {
    #[new]
    pub fn new() -> Self {
        Decoder {
            inner: crate::Decoder::new(),
            buf: Vec::new(),
        }
    }

    /// Feed a chunk of encoded data into the decoder.
    ///
    /// Partial output is buffered internally; call `finish()` to collect
    /// the complete result.
    pub fn update(&mut self, data: &Bound<'_, PyBytes>) {
        self.inner.decode(data.as_bytes(), &mut self.buf);
    }

    /// Flush any remaining partial value and return all decoded output
    /// accumulated since construction.
    ///
    /// The decoder is consumed by this call; further use raises
    /// `AttributeError`.
    pub fn finish<'py>(&mut self, py: Python<'py>) -> Bound<'py, PyBytes> {
        core::mem::take(&mut self.inner).finish(&mut self.buf);
        PyBytes::new(py, &core::mem::take(&mut self.buf))
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

/// The `pybase91` Python extension module.
#[pymodule]
pub fn pybase91(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(encode, m)?)?;
    m.add_function(wrap_pyfunction!(decode, m)?)?;
    m.add_class::<Encoder>()?;
    m.add_class::<Decoder>()?;
    Ok(())
}

/// Return stub generation metadata for `pybase91`.
///
/// Called by `src/bin/post_build.rs` to generate `pybase91.pyi`.
pub fn stub_info() -> pyo3_stub_gen::Result<StubInfo> {
    StubInfo::from_project_root(
        "pybase91".to_string(),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        false,
        Default::default(),
    )
}
