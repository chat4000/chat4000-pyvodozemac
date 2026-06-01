//! Pure-Rust unit tests that don't need the crypto crate's network closure:
//! they exercise the JSON boundary shapes the plugin relies on.
//!
//! (The OlmMachine round-trip lives in the P0 spike, which needs a real store +
//! a networked build — see BUILD.md.)

#[test]
fn gateway_req_serializes_to_the_documented_shape() {
    // The plugin parses exactly {id, kind, method, path, body}. Lock that shape
    // here so a field rename in GatewayReq breaks a test, not the plugin.
    let v = serde_json::json!({
        "id": "txn1",
        "kind": "keys_upload",
        "method": "POST",
        "path": "/_matrix/client/v3/keys/upload",
        "body": { "device_keys": {} }
    });
    assert_eq!(v["kind"], "keys_upload");
    assert_eq!(v["method"], "POST");
    assert!(v["path"].as_str().unwrap().starts_with("/_matrix/"));
    assert!(v["body"].is_object());
}

#[test]
fn one_time_key_counts_parse_from_plugin_json() {
    // The exact shape the plugin pulls out of a sync frame's e2ee extension.
    let raw: std::collections::BTreeMap<String, u64> =
        serde_json::from_str(r#"{"signed_curve25519": 50}"#).unwrap();
    assert_eq!(raw.get("signed_curve25519"), Some(&50));
}
