//! Crypto-store setup.
//!
//! The binding owns the durable Olm/Megolm key store (a single SQLite file).
//! Owning it here, behind one process and one writer, is the whole point: it is
//! the source of truth for room keys, so it must never be corrupted or shared.
//! Losing/corrupting it = permanent "Unable to decrypt".
//!
//! `passphrase` (optional) encrypts the store at rest. The plugin derives it
//! from local secret material and never logs it.

use matrix_sdk_sqlite::SqliteCryptoStore;

use crate::errors::{BindingError, Result};

/// Open (creating if absent) the SQLite crypto store at `path`.
pub async fn open_store(path: &str, passphrase: Option<&str>) -> Result<SqliteCryptoStore> {
    SqliteCryptoStore::open(path, passphrase)
        .await
        .map_err(|e| BindingError::Store(e.to_string()))
}
