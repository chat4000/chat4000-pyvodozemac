"""Typed contract for chat4000-pyvodozemac.

THIS .pyi IS THE INTEGRATION CONTRACT between the Rust binding and the Python
plugin. The plugin (chat4000-hermes-plugin) imports `OlmMachine` and drives it
with the push/pull loop below. The binding does no networking; the plugin owns
the gateway WebSocket + sliding sync and sends every request the binding emits.

All payloads cross the boundary as JSON strings (Matrix events and C-S request/
response bodies are already JSON), so there is nothing to keep in sync but the
shapes documented here.

The loop (mirrors matrix-sdk-crypto's OlmMachine "no network IO" design):

    m = OlmMachine(user_id, device_id, store_path)

    # On every sliding-sync `sync` frame, BEFORE sending sync_ack:
    decrypted_to_device = m.receive_sync_changes(
        to_device_events,           # JSON array of the sync's to_device events
        changed_devices,            # JSON {"changed":[...],"left":[...]}
        one_time_key_counts,        # JSON {"signed_curve25519": 50}
        unused_fallback_keys,       # JSON ["signed_curve25519"] or None
        next_batch,                 # the sync pos token, or None
    )
    persist_store_then_send_sync_ack()   # ORDER MATTERS (anti-UTD, see below)

    # Whenever there may be crypto work to push:
    for req_json in m.outgoing_requests():
        req = json.loads(req_json)          # {id, kind, method, path, body}
        status, body = gateway_req(req["method"], req["path"], req["body"])
        m.mark_request_as_sent(req["id"], req["kind"], status, json.dumps(body))

    # Before sending into room R whose members are `users`:
    claim = m.get_missing_sessions(users)   # -> req_json | None  (a /keys/claim)
    if claim: send_and_mark(claim)
    for td_json in m.share_room_key(room_id, users):  # to-device key shares
        send_and_mark(td_json)
    enc = m.encrypt_room_event(room_id, "m.room.message", json.dumps(content))
    gateway_req("PUT", f".../send/m.room.encrypted/{txn}", json.loads(enc))

    # On an inbound m.room.encrypted timeline event:
    clear = m.decrypt_room_event(json.dumps(event), room_id)

ANTI-UTD INVARIANT (critical): `receive_sync_changes` mutates the durable store
(it ingests Olm-encrypted Megolm room keys from to-device). The plugin MUST let
this call complete (store written) BEFORE it sends `sync_ack` to the gateway.
Acking first lets the homeserver delete to-device keys the store never saved →
permanent "Unable to decrypt". (And note: the *deployed* gateway does not yet
implement `sync_ack` — it auto-advances the cursor. That is a hard upstream
dependency, tracked as X-sync-ack in the plugin's pushback list.)
"""

from __future__ import annotations

class OlmMachine:
    """Wrapper around matrix-sdk-crypto's OlmMachine. Synchronous from Python;
    each method blocks on the underlying async crypto op (GIL released)."""

    def __init__(
        self,
        user_id: str,
        device_id: str,
        store_path: str,
        passphrase: str | None = None,
    ) -> None:
        """Open/create the SQLite crypto store and build the machine. Idempotent
        across restarts — an existing store restores the Olm account + room keys.
        `passphrase` (optional) encrypts the store at rest."""
        ...

    def identity_keys(self) -> str:
        """JSON `{"curve25519": "...", "ed25519": "..."}` — for logging/debug."""
        ...

    def receive_sync_changes(
        self,
        to_device_events: str,
        changed_devices: str,
        one_time_key_counts: str,
        unused_fallback_keys: str | None = None,
        next_batch: str | None = None,
    ) -> str:
        """Feed in a sync frame's crypto-relevant data. Returns a JSON array of
        decrypted to-device events (forward verification events to the UI; room
        keys are consumed internally). MUST precede sync_ack — see module doc."""
        ...

    def outgoing_requests(self) -> list[str]:
        """Drain pending crypto requests. Each item is a JSON
        `{id, kind, method, path, body}`; `kind` is one of `keys_upload`,
        `keys_query`, `keys_claim`, `signature_upload`, `to_device`,
        `keys_backup`, `room_message`. Send each as a gateway `req`, then call
        `mark_request_as_sent`. May re-emit until marked — drain+mark together."""
        ...

    def mark_request_as_sent(
        self, request_id: str, kind: str, status: int, body: str
    ) -> None:
        """Report a gateway `resp` for a drained request. `kind` is the tag from
        the request; `status` the HTTP status; `body` the response JSON string."""
        ...

    def get_missing_sessions(self, user_ids: list[str]) -> str | None:
        """A `/keys/claim` `req_json` to establish Olm sessions with member
        devices we lack, or None if all present. Send+mark before share_room_key."""
        ...

    def share_room_key(self, room_id: str, user_ids: list[str]) -> list[str]:
        """To-device `req_json`s sharing the room's Megolm session to the users'
        devices. Empty if already shared this rotation. Send+mark each. TOFU:
        shares to ALL devices (no cross-signing yet — see X1 pushback)."""
        ...

    def encrypt_room_event(self, room_id: str, event_type: str, content: str) -> str:
        """Encrypt cleartext content JSON → `m.room.encrypted` content JSON the
        plugin PUTs to `…/send/m.room.encrypted/{txn}`."""
        ...

    def decrypt_room_event(self, event: str, room_id: str) -> str:
        """Decrypt a full inbound `m.room.encrypted` event JSON → cleartext event
        JSON."""
        ...

    def update_tracked_users(self, user_ids: list[str]) -> None:
        """Track device lists for these users (room members) so `/keys/query`
        fires for them. Call on join / membership change."""
        ...

__version__: str
