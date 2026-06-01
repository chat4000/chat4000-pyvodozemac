//! chat4000-pyvodozemac — a Python binding around matrix-sdk-crypto's
//! `OlmMachine`, the Matrix E2EE state machine.
//!
//! ## Why this exists
//! The chat4000 Hermes plugin is Python (Hermes loads it in-process). The only
//! production-grade Matrix E2EE implementation is Rust (matrix-sdk-crypto, on
//! vodozemac); libolm is deprecated and there is no maintained Python binding to
//! the `OlmMachine`. This crate is that binding: a thin, transport-agnostic
//! crypto core the plugin drives.
//!
//! ## The contract (mirrors OlmMachine's "no network IO" design)
//! The plugin owns the gateway WebSocket + sliding sync. This binding does ZERO
//! networking. The loop is:
//!   1. plugin feeds sync-derived data in   -> `receive_sync_changes`
//!   2. plugin drains crypto requests       -> `outgoing_requests`  (KeysUpload/
//!      Query/Claim, ToDevice, SignatureUpload, RoomMessage)
//!   3. plugin sends each over the gateway and reports the result back
//!                                           -> `mark_request_as_sent`
//!   4. before sending to a room: `get_missing_sessions` -> claim, then
//!      `share_room_key`, then `encrypt_room_event`
//!   5. inbound room events                  -> `decrypt_room_event`
//!
//! Everything crosses the boundary as JSON strings (Matrix events and C-S
//! request/response bodies are already JSON), keeping the PyO3 surface tiny.

mod errors;
mod machine;
mod requests;
mod store;

use pyo3::prelude::*;

/// The importable native module: `chat4000_pyvodozemac._native`.
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<machine::PyOlmMachine>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
