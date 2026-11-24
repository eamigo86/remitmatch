//! PyO3 bindings for remitmatch. Exposes the native module `_core`.

use pyo3::prelude::*;

/// Native extension module for remitmatch. Not part of the public API.
#[pymodule]
mod _core {
    use pyo3::prelude::*;

    /// Return the version of the native core.
    ///
    /// Returns:
    ///     The semantic version string of the remitmatch crate.
    #[pyfunction]
    fn version() -> &'static str {
        remitmatch::version()
    }
}
