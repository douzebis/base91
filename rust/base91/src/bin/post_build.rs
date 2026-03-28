// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! Post-build helper for the `base91` extension module.
//!
//! Generates `pybase91.pyi` type stubs via pyo3_stub_gen.
//!
//! Run (from the workspace root `rust/`):
//!   cargo run --release -p base91 --bin post_build --features python

fn main() -> pyo3_stub_gen::Result<()> {
    base91::stub_info()?.generate()?;
    Ok(())
}
