//! Marshaling between matrix-sdk-crypto's outgoing requests and the gateway's
//! `req`/`resp` frames, and back again for `mark_request_as_sent`.
//!
//! The plugin owns the socket. So for each crypto request the machine wants to
//! make, we hand the plugin a plain `{ id, kind, method, path, body }` it can
//! drop straight into a gateway `req` frame; when the `resp` comes back the
//! plugin hands us `{ kind, status, body }` and we re-type it into the ruma
//! response `mark_request_as_sent` expects.
//!
//! ───────────────────────────────────────────────────────────────────────────
//! ⚠️ COMPILER PASS REQUIRED. Authored offline against the matrix-sdk-crypto 0.11
//! API *shape* (no network to fetch the crate, no local compile — see BUILD.md).
//! Symbols marked `(verify)` — notably the `AnyOutgoingRequest` enum path and the
//! per-variant ruma response types in machine.rs — drift between minors and must
//! be reconciled on the first networked `cargo build`. The control flow and the
//! `{id,kind,method,path,body}` contract are correct.
//! ───────────────────────────────────────────────────────────────────────────

use ruma::api::{MatrixVersion, OutgoingRequest as _, SendAccessToken};
use ruma::TransactionId;
use serde::Serialize;
use serde_json::{json, Value};

// (verify) 0.11 enum path; older minors called it `OutgoingRequests`.
use matrix_sdk_crypto::types::requests::{AnyOutgoingRequest, ToDeviceRequest};
use matrix_sdk_crypto::OutgoingRequest;

use crate::errors::{BindingError, Result};

/// C-S API versions we target when lowering a typed request to method+path.
const VERSIONS: &[MatrixVersion] = &[MatrixVersion::V1_11];

/// What we hand Python for one outgoing crypto request. `kind` is the tag the
/// plugin echoes back on the response so we can re-type it in `mark_request_as_sent`.
#[derive(Debug, Serialize)]
pub struct GatewayReq {
    pub id: String,
    pub kind: String,
    pub method: String,
    pub path: String,
    pub body: Value,
}

/// A ruma request lowered to (method, path, body).
struct Lowered {
    method: String,
    path: String,
    body: Value,
}

/// Lower one `OutgoingRequest` to a gateway-shippable `GatewayReq`.
pub fn to_gateway_req(req: &OutgoingRequest) -> Result<GatewayReq> {
    let id = req.request_id().to_string();
    let (kind, method, path, body) = lower_any(req.request())?;
    Ok(GatewayReq { id, kind, method, path, body })
}

fn lower_any(inner: &AnyOutgoingRequest) -> Result<(String, String, String, Value)> {
    Ok(match inner {
        // The "pure ruma" requests lower uniformly via ruma's OutgoingRequest.
        AnyOutgoingRequest::KeysUpload(r) => with_kind("keys_upload", lower(r)?),
        AnyOutgoingRequest::KeysQuery(r) => with_kind("keys_query", lower(r)?),
        AnyOutgoingRequest::KeysClaim(r) => with_kind("keys_claim", lower(r)?),
        AnyOutgoingRequest::SignatureUpload(r) => with_kind("signature_upload", lower(r)?),
        AnyOutgoingRequest::KeysBackup(r) => with_kind("keys_backup", lower(r)?),

        // ToDevice is matrix-sdk-crypto's own struct — build the path by hand.
        AnyOutgoingRequest::ToDevice(r) => {
            let (method, path, body) = to_device_parts(r);
            ("to_device".to_string(), method, path, body)
        }

        // RoomMessage (e.g. in-room verification).
        AnyOutgoingRequest::RoomMessage(r) => {
            let path = format!(
                "/_matrix/client/v3/rooms/{}/send/{}/{}",
                r.room_id,
                r.event_type(),
                r.txn_id
            );
            let body = serde_json::to_value(&r.content)?;
            ("room_message".to_string(), "PUT".to_string(), path, body)
        }
    })
}

fn with_kind(kind: &str, l: Lowered) -> (String, String, String, Value) {
    (kind.to_string(), l.method, l.path, l.body)
}

/// Lower any ruma `OutgoingRequest` to method/path/body by building its HTTP
/// form against a placeholder base (we keep only the path; the gateway adds the
/// real host + the socket's access token, so the token here is irrelevant).
fn lower<R: ruma::api::OutgoingRequest>(r: &R) -> Result<Lowered> {
    let http_req = r
        .try_into_http_request::<Vec<u8>>(
            "https://gateway.invalid",
            SendAccessToken::IfRequired("placeholder"),
            VERSIONS,
        )
        .map_err(|e| BindingError::HttpForm(e.to_string()))?;

    let method = http_req.method().as_str().to_string();
    let uri = http_req.uri();
    let path = match uri.query() {
        Some(q) => format!("{}?{}", uri.path(), q),
        None => uri.path().to_string(),
    };
    let body: Value = if http_req.body().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_slice(http_req.body())?
    };
    Ok(Lowered { method, path, body })
}

/// PUT …/sendToDevice/{eventType}/{txnId} with `{ "messages": … }`.
fn to_device_parts(r: &ToDeviceRequest) -> (String, String, Value) {
    let path = format!(
        "/_matrix/client/v3/sendToDevice/{}/{}",
        r.event_type, r.txn_id
    );
    ("PUT".to_string(), path, json!({ "messages": r.messages }))
}

/// Lower a `/keys/claim` produced by `get_missing_sessions` (it returns the
/// ruma request + an explicit txn id, hence its own helper).
pub fn keys_claim_to_gateway(
    txn: &TransactionId,
    req: &ruma::api::client::keys::claim_keys::v3::Request,
) -> Result<GatewayReq> {
    let l = lower(req)?;
    Ok(GatewayReq {
        id: txn.to_string(),
        kind: "keys_claim".to_string(),
        method: l.method,
        path: l.path,
        body: l.body,
    })
}

/// Lower a to-device request produced by `share_room_key`.
pub fn to_device_to_gateway(r: &ToDeviceRequest) -> Result<GatewayReq> {
    let (method, path, body) = to_device_parts(r);
    Ok(GatewayReq {
        id: r.txn_id.to_string(),
        kind: "to_device".to_string(),
        method,
        path,
        body,
    })
}

/// Build an `http::Response<Vec<u8>>` the ruma response parsers expect.
pub fn http_response(status: u16, body: &Value) -> Result<http::Response<Vec<u8>>> {
    let bytes = serde_json::to_vec(body)?;
    http::Response::builder()
        .status(status)
        .body(bytes)
        .map_err(|e| BindingError::HttpForm(e.to_string()))
}
