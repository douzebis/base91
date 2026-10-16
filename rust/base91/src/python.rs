// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! PyO3 bindings for the `pybase91` Python extension module.
//!
//! Exposes:
//! - Henke's format: `encode(data: bytes) -> bytes` and
//!   `decode(data: bytes) -> bytes` for one-shot use, `Encoder` and
//!   `Decoder` classes for streaming use.
//! - The SIMD variant (spec 0014 §2): `simd_encode`, `simd_decode` (either
//!   format, detected), `SimdEncoder`, `SimdDecoder`, `simd_level`, and the
//!   `Error` exception.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3_stub_gen::derive::{gen_stub_pyclass, gen_stub_pyfunction, gen_stub_pymethods};
use pyo3_stub_gen::StubInfo;

use crate::simd::{self, SimdLevel};

/// An invalid SIMD stream, or an invalid line width.
///
/// `kind` names the error: "invalid_width", "invalid_character",
/// "invalid_layout", "invalid_length" or "invalid_value".  `offset` is the
/// position of the offending byte in the input, or None.
#[gen_stub_pyclass]
#[pyclass(extends = PyValueError, module = "pybase91")]
pub struct Error {
    message: String,
    /// The kind of error.
    #[pyo3(get)]
    kind: String,
    /// The position of the offending byte in the input, or None.
    #[pyo3(get)]
    offset: Option<usize>,
}

#[gen_stub_pymethods]
#[pymethods]
impl Error {
    #[new]
    #[pyo3(signature = (message, kind, offset = None))]
    pub fn new(message: String, kind: String, offset: Option<usize>) -> Self {
        Error {
            message,
            kind,
            offset,
        }
    }

    pub fn __str__(&self) -> String {
        self.message.clone()
    }
}

/// `Error`, with its `kind` and `offset`, for a Rust error.
fn to_py_err(py: Python<'_>, e: crate::Error) -> PyErr {
    let (kind, offset) = match e {
        crate::Error::BufferTooSmall { .. } => ("buffer_too_small", None),
        crate::Error::InvalidWidth { .. } => ("invalid_width", None),
        crate::Error::InvalidCharacter { offset } => ("invalid_character", Some(offset)),
        crate::Error::InvalidLayout { offset } => ("invalid_layout", Some(offset)),
        crate::Error::InvalidLength => ("invalid_length", None),
        crate::Error::InvalidValue { offset } => ("invalid_value", Some(offset)),
    };
    // Through the type's constructor, so that the ValueError base is
    // initialized with the arguments, as for an `Error(...)` raised from
    // Python.
    match py.get_type::<Error>().call1((e.to_string(), kind, offset)) {
        Ok(value) => PyErr::from_value(value),
        Err(err) => err,
    }
}

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

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
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

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SIMD variant (spec 0014 §2)
// ---------------------------------------------------------------------------

/// Encode `data` with the SIMD variant of base91, at the best SIMD level of
/// the CPU.  `wrap` is the line width: 0 (no line breaks) or a multiple of
/// 32.
///
/// The output starts with `-` and uses an alphabet without `'`.  Raises
/// `Error` (kind "invalid_width") for another width.
#[gen_stub_pyfunction]
#[pyfunction]
#[pyo3(signature = (data, wrap = 0))]
pub fn simd_encode<'py>(
    py: Python<'py>,
    data: &Bound<'py, PyBytes>,
    wrap: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    let encoded =
        simd::encode(data.as_bytes(), SimdLevel::default(), wrap).map_err(|e| to_py_err(py, e))?;
    Ok(PyBytes::new(py, &encoded))
}

/// Decode `data`, in either format: after leading whitespace, `-` starts a
/// SIMD stream, anything else is Henke's format.
///
/// A SIMD stream is checked strictly: a byte outside its alphabet, a broken
/// line layout, an invalid length or value raises `Error`, with the offset
/// of the offending byte.  Henke's format ignores bytes outside its
/// alphabet, as `decode` does.
#[gen_stub_pyfunction]
#[pyfunction]
pub fn simd_decode<'py>(
    py: Python<'py>,
    data: &Bound<'py, PyBytes>,
) -> PyResult<Bound<'py, PyBytes>> {
    let decoded =
        simd::decode(data.as_bytes(), SimdLevel::default()).map_err(|e| to_py_err(py, e))?;
    Ok(PyBytes::new(py, &decoded))
}

/// The SIMD kernel the CPU runs: "avx2", "sse4.1", "neon" or "scalar".
#[gen_stub_pyfunction]
#[pyfunction]
pub fn simd_level() -> &'static str {
    match simd::detect() {
        SimdLevel::Simd256 => "avx2",
        SimdLevel::Simd128 if cfg!(target_arch = "x86_64") => "sse4.1",
        SimdLevel::Simd128 => "neon",
        SimdLevel::Scalar => "scalar",
    }
}

/// Streaming encoder for the SIMD variant.
///
/// Feed data with `update()`; `finish()` returns all the encoded output and
/// resets the encoder.  The output equals `simd_encode` of the whole input,
/// however it is split.
///
/// ```python
/// enc = pybase91.SimdEncoder(wrap=64)
/// enc.update(b"Hello, ")
/// enc.update(b"world!")
/// result = enc.finish()
/// ```
#[gen_stub_pyclass]
#[pyclass]
pub struct SimdEncoder {
    inner: simd::Encoder,
    wrap: usize,
    buf: Vec<u8>,
}

#[gen_stub_pymethods]
#[pymethods]
impl SimdEncoder {
    /// A new encoder; `wrap` as in `simd_encode`.
    #[new]
    #[pyo3(signature = (wrap = 0))]
    pub fn new(py: Python<'_>, wrap: usize) -> PyResult<Self> {
        let inner = simd::Encoder::new(SimdLevel::default(), wrap).map_err(|e| to_py_err(py, e))?;
        Ok(SimdEncoder {
            inner,
            wrap,
            buf: Vec::new(),
        })
    }

    /// Feed a chunk of data into the encoder.
    pub fn update(&mut self, data: &Bound<'_, PyBytes>) {
        self.inner.encode(data.as_bytes(), &mut self.buf);
    }

    /// Return all the encoded output since construction (or the last
    /// `finish()`), and reset the encoder.
    pub fn finish<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let fresh =
            simd::Encoder::new(SimdLevel::default(), self.wrap).map_err(|e| to_py_err(py, e))?;
        core::mem::replace(&mut self.inner, fresh).finish(&mut self.buf);
        Ok(PyBytes::new(py, &core::mem::take(&mut self.buf)))
    }
}

/// Streaming decoder, for either format (detected as in `simd_decode`).
///
/// Feed encoded data with `update()`; `finish()` returns all the decoded
/// output and resets the decoder.  An invalid SIMD stream raises `Error`
/// from the call that detects it, and from every later call until
/// `finish()`.
///
/// ```python
/// dec = pybase91.SimdDecoder()
/// dec.update(encoded[:10])
/// dec.update(encoded[10:])
/// result = dec.finish()
/// ```
#[gen_stub_pyclass]
#[pyclass]
pub struct SimdDecoder {
    inner: simd::Decoder,
    buf: Vec<u8>,
}

#[gen_stub_pymethods]
#[pymethods]
impl SimdDecoder {
    #[new]
    pub fn new() -> Self {
        SimdDecoder {
            inner: simd::Decoder::new(SimdLevel::default()),
            buf: Vec::new(),
        }
    }

    /// Feed a chunk of encoded data into the decoder.
    pub fn update(&mut self, py: Python<'_>, data: &Bound<'_, PyBytes>) -> PyResult<()> {
        self.inner
            .decode(data.as_bytes(), &mut self.buf)
            .map_err(|e| to_py_err(py, e))
    }

    /// Check the end of the stream, return all the decoded output since
    /// construction (or the last `finish()`), and reset the decoder.
    pub fn finish<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let result = core::mem::take(&mut self.inner).finish(&mut self.buf);
        let out = core::mem::take(&mut self.buf);
        result.map_err(|e| to_py_err(py, e))?;
        Ok(PyBytes::new(py, &out))
    }
}

impl Default for SimdDecoder {
    fn default() -> Self {
        Self::new()
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
    m.add_function(wrap_pyfunction!(simd_encode, m)?)?;
    m.add_function(wrap_pyfunction!(simd_decode, m)?)?;
    m.add_function(wrap_pyfunction!(simd_level, m)?)?;
    m.add_class::<SimdEncoder>()?;
    m.add_class::<SimdDecoder>()?;
    m.add_class::<Error>()?;
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
