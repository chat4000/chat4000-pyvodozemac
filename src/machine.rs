//! `PyOlmMachine` — the Python-facing wrapper around matrix-sdk-crypto's
//! `OlmMachine`.
//!
//! Synchronous from Python's perspective: each method `block_on`s the async
//! OlmMachine call on a small Tokio runtime, releasing the GIL for the duration
//! (`py.allow_threads`) so the plugin's asyncio loop isn't starved.
//!
//! ───────────────────────────────────────────────────────────────────────────
//! ⚠️ COMPILER PASS REQUIRED (see BUILD.md). Authored offline against the
//! matrix-sdk-crypto 0.11 API shape. Items marked `(verify)` — constructor arity,
//! `EncryptionSyncChanges`/`DecryptionSettings`/`EncryptionSettings` fields, the
//! `decrypt_room_event` settings arg, and a few ruma response paths — move
//! between minors and must be reconciled on the first networked build. The
//! method set, the JSON boundary, and the call sequence are the contract and are
//! correct.
//! ───────────────────────────────────────────────────────────────────────────

use std::collections::BTreeMap;
use std::mem::ManuallyDrop;

use pyo3::prelude::*;
use ruma::{
    api::client::sync::sync_events::DeviceLists,
    events::AnyToDeviceEvent,
    serde::Raw,
    OneTimeKeyAlgorithm, OwnedUserId, UInt, UserId,
};
use serde_json::Value;
use tokio::runtime::Runtime;

use matrix_sdk_crypto::{
    types::requests::ToDeviceRequest, DecryptionSettings, EncryptionSettings,
    EncryptionSyncChanges, OlmMachine, TrustRequirement,
};

use crate::errors::{BindingError, Result};
use crate::requests::{http_response, to_gateway_req, GatewayReq};
use crate::store::open_store;

#[pyclass]
pub struct PyOlmMachine {
    // ManuallyDrop so we control WHEN the OlmMachine (and its SQLite store's
    // deadpool connection pool) is dropped — inside the Tokio runtime context.
    // `ManuallyDrop<T>` derefs to `T`, so `self.inner.method()` is unchanged.
    inner: ManuallyDrop<OlmMachine>,
    rt: Runtime,
}

impl PyOlmMachine {
    fn user_id(s: &str) -> Result<OwnedUserId> {
        UserId::parse(s).map_err(|e| BindingError::Id(e.to_string()))
    }
}

impl Drop for PyOlmMachine {
    fn drop(&mut self) {
        // The SQLite crypto store's deadpool pool schedules async cleanup on
        // drop, which needs a running Tokio reactor. Enter the runtime first,
        // then drop the machine explicitly — otherwise teardown panics with
        // "there is no reactor running".
        let _guard = self.rt.enter();
        // SAFETY: inner is never used again after this; the surrounding struct
        // is being dropped.
        unsafe { ManuallyDrop::drop(&mut self.inner) };
    }
}

#[pymethods]
impl PyOlmMachine {
    /// Open (or create) the crypto store at `store_path` and build the machine
    /// for `user_id`/`device_id`. Idempotent across restarts: an existing store
    /// restores the device's Olm account + all room keys.
    #[new]
    #[pyo3(signature = (user_id, device_id, store_path, passphrase=None))]
    fn new(
        py: Python<'_>,
        user_id: &str,
        device_id: &str,
        store_path: &str,
        passphrase: Option<&str>,
    ) -> PyResult<Self> {
        let rt = Runtime::new().map_err(|e| BindingError::Machine(e.to_string()))?;
        let uid = Self::user_id(user_id)?;
        let did = device_id.into();

        let inner = py.allow_threads(|| {
            rt.block_on(async {
                let store = open_store(store_path, passphrase).await?;
                // (verify) 0.11 takes a 4th `custom_account: Option<Account>` arg.
                OlmMachine::with_store(&uid, did, store, None)
                    .await
                    .map_err(|e| BindingError::Store(e.to_string()))
            })
        })?;

        Ok(Self { inner: ManuallyDrop::new(inner), rt })
    }

    /// The device's published identity keys (curve25519 + ed25519), as JSON.
    /// The plugin includes these nowhere on the wire directly — they're for
    /// logging/debugging device identity.
    fn identity_keys(&self, py: Python<'_>) -> PyResult<String> {
        let keys = py.allow_threads(|| {
            let k = self.inner.identity_keys();
            serde_json::json!({ "curve25519": k.curve25519.to_base64(), "ed25519": k.ed25519.to_base64() })
        });
        Ok(keys.to_string())
    }

    /// Step 1 of the loop. Feed in everything a sliding-sync frame carried that
    /// the crypto layer cares about: `to_device` events (Olm-encrypted room keys
    /// live here), the `device_lists` changed/left sets, the one-time-key counts,
    /// and the unused fallback-key algorithms. Returns the decrypted to-device
    /// events as a JSON array (the plugin forwards verification events to the UI;
    /// room-key events are consumed internally).
    ///
    /// IMPORTANT (anti-UTD): the plugin MUST persist the store (this call mutates
    /// it) and only THEN send `sync_ack` to the gateway. Acking before this
    /// returns risks the homeserver deleting room keys the store never saved.
    #[pyo3(signature = (to_device_events, changed_devices, one_time_key_counts, unused_fallback_keys=None, next_batch=None))]
    fn receive_sync_changes(
        &self,
        py: Python<'_>,
        to_device_events: &str,
        changed_devices: &str,
        one_time_key_counts: &str,
        unused_fallback_keys: Option<&str>,
        next_batch: Option<String>,
    ) -> PyResult<String> {
        let to_device: Vec<Raw<AnyToDeviceEvent>> = serde_json::from_str(to_device_events)
            .map_err(BindingError::from)?;
        let device_lists: DeviceLists =
            serde_json::from_str(changed_devices).map_err(BindingError::from)?;

        // {"signed_curve25519": 50} → BTreeMap<OneTimeKeyAlgorithm, UInt>
        let raw_counts: BTreeMap<String, u64> =
            serde_json::from_str(one_time_key_counts).map_err(BindingError::from)?;
        let otk_counts: BTreeMap<OneTimeKeyAlgorithm, UInt> = raw_counts
            .into_iter()
            .filter_map(|(k, v)| UInt::try_from(v).ok().map(|u| (OneTimeKeyAlgorithm::from(k), u)))
            .collect();

        let fallback: Option<Vec<OneTimeKeyAlgorithm>> = match unused_fallback_keys {
            Some(s) => Some(
                serde_json::from_str::<Vec<String>>(s)
                    .map_err(BindingError::from)?
                    .into_iter()
                    .map(OneTimeKeyAlgorithm::from)
                    .collect(),
            ),
            None => None,
        };

        let decrypted = py.allow_threads(|| {
            self.rt.block_on(async {
                let changes = EncryptionSyncChanges {
                    to_device_events: to_device,
                    changed_devices: &device_lists,
                    one_time_keys_counts: &otk_counts,
                    unused_fallback_keys: fallback.as_deref(),
                    next_batch_token: next_batch,
                };
                self.inner
                    .receive_sync_changes(changes)
                    .await
                    .map_err(|e| BindingError::Machine(e.to_string()))
            })
        })?;

        // (verify) 0.11 returns (Vec<ProcessedToDeviceEvent>, Vec<RoomKeyInfo>).
        // We serialize the to-device half for the plugin.
        let out = serde_json::to_string(&decrypted.0).map_err(BindingError::from)?;
        Ok(out)
    }

    /// Step 2. Drain the crypto requests the machine wants made (key upload/query/
    /// claim, to-device, signatures). Each is returned as a JSON `GatewayReq`
    /// `{ id, kind, method, path, body }` the plugin ships as a gateway `req`.
    ///
    /// May return the same request twice if called again before `mark_request_as_sent`;
    /// the plugin should drain + mark in a single critical section.
    fn outgoing_requests(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        let reqs = py.allow_threads(|| {
            self.rt.block_on(async {
                self.inner
                    .outgoing_requests()
                    .await
                    .map_err(|e| BindingError::Machine(e.to_string()))
            })
        })?;

        let mut out = Vec::with_capacity(reqs.len());
        for r in &reqs {
            let g: GatewayReq = to_gateway_req(r)?;
            out.push(serde_json::to_string(&g).map_err(BindingError::from)?);
        }
        Ok(out)
    }

    /// Step 3. Report the gateway's `resp` for a previously-drained request, so
    /// the machine advances its state (e.g. records uploaded keys, ingests a
    /// `/keys/query` device list). `kind` is the tag from the `GatewayReq`.
    fn mark_request_as_sent(
        &self,
        py: Python<'_>,
        request_id: &str,
        kind: &str,
        status: u16,
        body: &str,
    ) -> PyResult<()> {
        let body_val: Value = serde_json::from_str(body).map_err(BindingError::from)?;
        let http = http_response(status, &body_val)?;
        let txn_id: ruma::OwnedTransactionId = request_id.into();

        py.allow_threads(|| {
            self.rt.block_on(async {
                use ruma::api::IncomingResponse as _;
                // Parse into the ruma response matching `kind`, then hand the
                // machine a borrowed IncomingResponse. (verify) the exact module
                // paths of these Response types per pinned ruma.
                macro_rules! mark {
                    ($resp_ty:path) => {{
                        let resp = <$resp_ty>::try_from_http_response(http)
                            .map_err(|e| BindingError::HttpForm(e.to_string()))?;
                        self.inner
                            .mark_request_as_sent(&txn_id, &resp)
                            .await
                            .map_err(|e| BindingError::Machine(e.to_string()))
                    }};
                }
                match kind {
                    "keys_upload" => mark!(ruma::api::client::keys::upload_keys::v3::Response),
                    "keys_query" => mark!(ruma::api::client::keys::get_keys::v3::Response),
                    "keys_claim" => mark!(ruma::api::client::keys::claim_keys::v3::Response),
                    "signature_upload" => {
                        mark!(ruma::api::client::keys::upload_signatures::v3::Response)
                    }
                    "to_device" => {
                        mark!(ruma::api::client::to_device::send_event_to_device::v3::Response)
                    }
                    "keys_backup" => {
                        mark!(ruma::api::client::backup::add_backup_keys::v3::Response)
                    }
                    other => Err(BindingError::UnknownRequestType(other.to_string())),
                }
            })
        })?;
        Ok(())
    }

    /// Before sending into a room: establish Olm sessions with any of the room's
    /// member devices we don't yet have. Returns a single `GatewayReq` (a
    /// `/keys/claim`) to send, or `None` if every device already has a session.
    fn get_missing_sessions(&self, py: Python<'_>, user_ids: Vec<String>) -> PyResult<Option<String>> {
        let users: Vec<OwnedUserId> =
            user_ids.iter().map(|s| Self::user_id(s)).collect::<Result<_>>()?;

        let claim = py.allow_threads(|| {
            self.rt.block_on(async {
                self.inner
                    .get_missing_sessions(users.iter().map(|u| u.as_ref()))
                    .await
                    .map_err(|e| BindingError::Machine(e.to_string()))
            })
        })?;

        match claim {
            // (verify) returns Option<(OwnedTransactionId, KeysClaimRequest)>.
            Some((txn, req)) => {
                let g = crate::requests::keys_claim_to_gateway(&txn, &req)?;
                Ok(Some(serde_json::to_string(&g).map_err(BindingError::from)?))
            }
            None => Ok(None),
        }
    }

    /// Share the room's outbound Megolm session to the given users' devices.
    /// Returns the to-device `GatewayReq`s to send. No-op (empty) if the current
    /// session was already shared. TOFU: shares to ALL of each user's devices —
    /// see X1 in the plugin docs (no cross-signing yet; a hostile homeserver
    /// injecting a device is the accepted residual risk until verification lands).
    fn share_room_key(
        &self,
        py: Python<'_>,
        room_id: &str,
        user_ids: Vec<String>,
    ) -> PyResult<Vec<String>> {
        let rid = ruma::RoomId::parse(room_id).map_err(|e| BindingError::Id(e.to_string()))?;
        let users: Vec<OwnedUserId> =
            user_ids.iter().map(|s| Self::user_id(s)).collect::<Result<_>>()?;

        let to_device_reqs: Vec<std::sync::Arc<ToDeviceRequest>> = py.allow_threads(|| {
            self.rt.block_on(async {
                // (verify) EncryptionSettings fields/defaults; default = Megolm v1
                // AES-SHA2 with the standard rotation. TOFU sharing strategy.
                let settings = EncryptionSettings::default();
                self.inner
                    .share_room_key(&rid, users.iter().map(|u| u.as_ref()), settings)
                    .await
                    .map_err(|e| BindingError::Machine(e.to_string()))
            })
        })?;

        let mut out = Vec::with_capacity(to_device_reqs.len());
        for r in &to_device_reqs {
            let g = crate::requests::to_device_to_gateway(r)?;
            out.push(serde_json::to_string(&g).map_err(BindingError::from)?);
        }
        Ok(out)
    }

    /// Encrypt one room event. `content` is the cleartext inner content JSON
    /// (e.g. the `chat4000.tool` payload or an `m.room.message`); returns the
    /// `m.room.encrypted` content JSON the plugin PUTs to `…/send/m.room.encrypted`.
    fn encrypt_room_event(
        &self,
        py: Python<'_>,
        room_id: &str,
        event_type: &str,
        content: &str,
    ) -> PyResult<String> {
        let rid = ruma::RoomId::parse(room_id).map_err(|e| BindingError::Id(e.to_string()))?;
        let raw: Raw<ruma::events::AnyMessageLikeEventContent> =
            Raw::from_json_string(content.to_string()).map_err(BindingError::from)?;

        let encrypted = py.allow_threads(|| {
            self.rt.block_on(async {
                // (verify) name: `encrypt_room_event_raw(room_id, event_type, &Raw)`.
                self.inner
                    .encrypt_room_event_raw(&rid, event_type, &raw)
                    .await
                    .map_err(|e| BindingError::Machine(e.to_string()))
            })
        })?;

        Ok(encrypted.json().get().to_string())
    }

    /// Decrypt one inbound `m.room.encrypted` event. Returns the cleartext event
    /// JSON. `event` is the full event object (not just content) as it arrived in
    /// the sync `rooms` timeline.
    fn decrypt_room_event(&self, py: Python<'_>, event: &str, room_id: &str) -> PyResult<String> {
        let rid = ruma::RoomId::parse(room_id).map_err(|e| BindingError::Id(e.to_string()))?;
        let raw: Raw<ruma::events::AnyTimelineEvent> =
            Raw::from_json_string(event.to_string()).map_err(BindingError::from)?;
        // SAFETY of cast: matrix-sdk-crypto wants Raw<EncryptedEvent>; the JSON
        // is a superset, so reinterpreting the Raw is sound. (verify) the exact
        // expected Raw type for the pinned version.
        let raw_enc = raw.cast();

        let decrypted = py.allow_threads(|| {
            self.rt.block_on(async {
                // TOFU: accept events from untrusted (unverified) sender devices.
                let settings = DecryptionSettings {
                    sender_device_trust_requirement: TrustRequirement::Untrusted,
                };
                // (verify) 0.11 signature: decrypt_room_event(&raw, room_id, &settings).
                self.inner
                    .decrypt_room_event(&raw_enc, &rid, &settings)
                    .await
                    .map_err(|e| BindingError::Machine(e.to_string()))
            })
        })?;

        // (verify) DecryptedRoomEvent exposes the cleartext as `.event` Raw.
        Ok(decrypted.event.json().get().to_string())
    }

    /// Tell the machine which users we're tracking device lists for (the room's
    /// members), so `/keys/query` is issued for them. Call on room join /
    /// membership change.
    fn update_tracked_users(&self, py: Python<'_>, user_ids: Vec<String>) -> PyResult<()> {
        let users: Vec<OwnedUserId> =
            user_ids.iter().map(|s| Self::user_id(s)).collect::<Result<_>>()?;
        py.allow_threads(|| {
            self.rt.block_on(async {
                self.inner
                    .update_tracked_users(users.iter().map(|u| u.as_ref()))
                    .await
                    .map_err(|e| BindingError::Machine(e.to_string()))
            })
        })?;
        Ok(())
    }
}
