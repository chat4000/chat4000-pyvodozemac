# chat4000-pyvodozemac

A Python binding around [`matrix-sdk-crypto`](https://github.com/matrix-org/matrix-rust-sdk)'s
`OlmMachine` — the Matrix end-to-end-encryption state machine — for the
[chat4000 Hermes plugin](../chat4000-hermes-plugin).

## Why

The chat4000 plugin is Python (Hermes loads it in-process). The only
production-grade Matrix E2EE implementation is Rust (`matrix-sdk-crypto`, built
on `vodozemac`); **libolm is deprecated** and there is **no maintained Python
binding** to the `OlmMachine`. This crate is that binding.

The name nods to vodozemac (the Rust crypto vodozemac sits underneath), but what
it wraps is the higher-level **`OlmMachine`** — the audited state machine — not
the raw primitives. Wrapping the primitives would mean re-implementing the
E2EE state machine in Python, which is the exact thing not to do.

## What it is (and isn't)

- **Is:** a transport-agnostic crypto state machine. You push sync-derived data
  in; you pull the C-S requests it needs made; you report the results back.
  Device keys, Olm sessions, Megolm room keys, encrypt/decrypt.
- **Isn't:** a Matrix client. It does **zero networking**. The plugin owns the
  gateway WebSocket, sliding sync, rooms, and all I/O.

## The contract

See [`python/chat4000_pyvodozemac/__init__.pyi`](python/chat4000_pyvodozemac/__init__.pyi)
— it is the integration contract. The push/pull loop:

```
receive_sync_changes(...)         # ← to-device, device lists, OTK counts
  → (persist store) → sync_ack    #   ANTI-UTD: persist before acking
outgoing_requests() → send → mark_request_as_sent(...)
get_missing_sessions → share_room_key → encrypt_room_event   # outbound
decrypt_room_event(...)           # inbound
```

## Build

See [`BUILD.md`](BUILD.md). Short version: `maturin develop --release` (needs
network + Rust). **Note:** this checkout was authored offline and not yet
compiled — a first networked `cargo build` will reconcile the `(verify)` symbols.

## Architecture decision

This is **Architecture O2** from the plugin's design: an in-process PyO3 binding,
with the Python plugin owning transport. The alternative (a separate Rust sidecar
over IPC) was option O1. O2 keeps everything in one process — preserving Hermes'
synchronous in-process tool hooks — at the cost of maintaining this binding.

## License

GPL-3.0-or-later. © 2026 NeonNode Ltd.
