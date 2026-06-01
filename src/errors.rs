//! Error type that converts cleanly into a Python exception.
//!
//! Everything the binding can fail on (bad JSON at the boundary, a crypto-store
//! error, an OlmMachine operation, an unknown request type) collapses into one
//! `BindingError` that PyO3 turns into a `chat4000_pyvodozemac.CryptoError`.

use pyo3::exceptions::PyValueError;
use pyo3::PyErr;

#[derive(Debug, thiserror::Error)]
pub enum BindingError {
    #[error("invalid JSON at the Python<->Rust boundary: {0}")]
    Json(#[from] serde_json::Error),

    #[error("crypto store error: {0}")]
    Store(String),

    #[error("OlmMachine error: {0}")]
    Machine(String),

    #[error("invalid Matrix identifier: {0}")]
    Id(String),

    #[error("unknown or unsupported outgoing-request type: {0}")]
    UnknownRequestType(String),

    #[error("could not build HTTP form of request: {0}")]
    HttpForm(String),
}

impl From<BindingError> for PyErr {
    fn from(e: BindingError) -> Self {
        // One Python-visible exception class keeps the plugin's error handling
        // simple; the message carries the specific cause.
        PyValueError::new_err(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, BindingError>;
