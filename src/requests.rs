//! Marshaling between matrix-sdk-crypto's outgoing requests and the gateway's
//! `req`/`resp` frames, and back again for `mark_request_as_sent`.
//!
//! The plugin owns the socket. So for each crypto request the machine wants to
//! make, we hand the plugin a plain `{ id, kind, method, path, body }` it can
//! drop straight into a gateway `req` frame; when the `resp` comes back the
//! plugin hands us `{ kind, status, body }` and we re-type it into the ruma
//! response `mark_request_as_sent` expects.

use ruma::api::{MatrixVersion, OutgoingRequest as RumaOutgoingRequest, SendAccessToken};
use ruma::events::EventContent; // brings `.event_type()` into scope on event content
use ruma::TransactionId;
use serde::Serialize;
use serde_json::{json, Value};

use matrix_sdk_crypto::types::requests::{
    AnyOutgoingRequest, OutgoingRequest, RoomMessageRequest, ToDeviceRequest,
};

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

/// A request lowered to (method, path, body).
struct Lowered {
    method: String,
    path: String,
    body: Value,
}

/// Lower one `OutgoingRequest` to a gateway-shippable `GatewayReq`.
pub fn to_gateway_req(req: &OutgoingRequest) -> Result<GatewayReq> {
    let id = req.request_id().to_string();
    let (kind, l) = lower_any(req.request())?;
    Ok(GatewayReq {
        id,
        kind: kind.to_string(),
        method: l.method,
        path: l.path,
        body: l.body,
    })
}

fn lower_any(inner: &AnyOutgoingRequest) -> Result<(&'static str, Lowered)> {
    Ok(match inner {
        // The "pure ruma" requests lower uniformly via ruma's OutgoingRequest.
        AnyOutgoingRequest::KeysUpload(r) => ("keys_upload", lower_ruma(r)?),
        AnyOutgoingRequest::KeysClaim(r) => ("keys_claim", lower_ruma(r)?),
        AnyOutgoingRequest::SignatureUpload(r) => ("signature_upload", lower_ruma(r)?),

        // KeysQuery is matrix-sdk-crypto's OWN wrapper struct (not a ruma
        // request), so we build the C-S call by hand from its fields.
        AnyOutgoingRequest::KeysQuery(r) => ("keys_query", keys_query_parts(r)),

        // ToDevice is matrix-sdk-crypto's own struct — build the path by hand.
        AnyOutgoingRequest::ToDeviceRequest(r) => ("to_device", to_device_parts(r)),

        // RoomMessage (e.g. in-room verification).
        AnyOutgoingRequest::RoomMessage(r) => ("room_message", room_message_parts(r)?),
    })
}

/// Lower any ruma `OutgoingRequest` to method/path/body by building its HTTP
/// form against a placeholder base (we keep only the path; the gateway adds the
/// real host + the socket's access token, so the token here is irrelevant).
///
/// `try_into_http_request` consumes `self`, so we clone the inner ruma request
/// (we only hold a shared `&` to it inside the `AnyOutgoingRequest`).
fn lower_ruma<R: RumaOutgoingRequest + Clone>(r: &R) -> Result<Lowered> {
    let http_req = r
        .clone()
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

/// POST /_matrix/client/v3/keys/query from the crypto wrapper's fields.
fn keys_query_parts(r: &matrix_sdk_crypto::types::requests::KeysQueryRequest) -> Lowered {
    // device_keys: { <user>: [<device>, ...] }. An empty Vec means "all the
    // user's devices", which is exactly the C-S API's semantics, so we forward
    // the map as-is.
    let mut body = json!({ "device_keys": r.device_keys });
    if let Some(timeout) = r.timeout {
        body["timeout"] = json!(timeout.as_millis() as u64);
    }
    Lowered {
        method: "POST".to_string(),
        path: "/_matrix/client/v3/keys/query".to_string(),
        body,
    }
}

/// PUT …/sendToDevice/{eventType}/{txnId} with `{ "messages": … }`.
fn to_device_parts(r: &ToDeviceRequest) -> Lowered {
    let path = format!(
        "/_matrix/client/v3/sendToDevice/{}/{}",
        r.event_type, r.txn_id
    );
    Lowered {
        method: "PUT".to_string(),
        path,
        body: json!({ "messages": r.messages }),
    }
}

/// PUT …/rooms/{roomId}/send/{eventType}/{txnId} with the content as the body.
fn room_message_parts(r: &RoomMessageRequest) -> Result<Lowered> {
    // The event-type string lives in the content (there is no event_type field
    // on the request itself).
    let event_type = r.content.event_type().to_string();
    let path = format!(
        "/_matrix/client/v3/rooms/{}/send/{}/{}",
        r.room_id, event_type, r.txn_id
    );
    let body = serde_json::to_value(&r.content)?;
    Ok(Lowered {
        method: "PUT".to_string(),
        path,
        body,
    })
}

/// Lower a `/keys/claim` produced by `get_missing_sessions` (it returns the
/// ruma request + an explicit txn id, hence its own helper).
pub fn keys_claim_to_gateway(
    txn: &TransactionId,
    req: &ruma::api::client::keys::claim_keys::v3::Request,
) -> Result<GatewayReq> {
    let l = lower_ruma(req)?;
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
    let l = to_device_parts(r);
    Ok(GatewayReq {
        id: r.txn_id.to_string(),
        kind: "to_device".to_string(),
        method: l.method,
        path: l.path,
        body: l.body,
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
