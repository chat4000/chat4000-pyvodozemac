# chat4000 v2 migration — implementation plan (Part 1)

Cross-repo plan. This repo (`chat4000-pyvodozemac`) is the crypto binding; the
work spans it and the Python plugin (`chat4000-hermes-plugin`).

## Architecture (decided: O2)

- **`chat4000-pyvodozemac` (Rust, this repo)** — in-process PyO3 binding wrapping
  matrix-sdk-crypto's `OlmMachine`. Zero networking. Ships as a maturin wheel the
  plugin depends on.
- **`chat4000-hermes-plugin` (Python)** — owns the gateway WS, sliding sync,
  rooms/turns/tools, registrar HTTP, Hermes integration; calls this binding for
  all crypto.

## Phases

- **P0 — Spike (de-risk).** Networked `cargo build` of this crate; reconcile the
  `(verify)` symbols. Then: plugin connects gateway → `auth` → sliding-sync →
  `OlmMachine` round-trip (device-key publish, Megolm share, decrypt) across 2
  devices with `sync_ack` discipline. **Gate: zero UTD over a 1k-message soak.**
- **P1 — Spine.** Plugin: gateway client (`auth/reauth/req/resp/sync/sync_ack`),
  sliding-sync loop, the crypto driver (drives this binding), store persistence,
  reconnect. Binding: this crate, compiling + unit-tested.
- **P2 — Onboarding & rooms.** Plugin registrar client (self-onboard `kind=plugin`,
  `/pair/register`+poll, `/version` gate); create space + encrypted control room
  (`chat4000.room_kind`), invite user, session rooms.
- **P3 — Turns.** Inbound decrypt → Hermes `handle_message`; turn anchor +
  `m.replace` streaming; `chat4000.push` discipline; `chat4000.status` state.
- **P4 — Tools.** `chat4000.tool` events (2 sends/tool) via reworked dispatcher.
- **P5 — Commands.** Control-room-only `session.*` + results; `plugin.update_check`
  only (defer `plugin.update` — owner model undefined, pushback X4).
- **P6 — Media.** Encrypted attachment up/download over the HTTP media path (D.3)
  for inbound image/audio (vision/STT) + outbound.
- **P7 — Packaging & harden.** cibuildwheel/maturin wheels for this crate; CLI/
  wizard rewrite for 6-digit OTP; TOFU key-sharing + key backup; version gate;
  full zero-UTD soak.

## Open pushbacks blocking/limiting phases (to backend team)

- **X1** no cross-signing → key-sharing is exposed to homeserver device injection (P7).
- **X2** shared `REGISTRAR_SERVICE_TOKEN` on user machines, self-asserted `plugin_id` (P2).
- **X3** bot-token rotation undefined; re-onboard destroys identity (P1/P2).
- **X4** owner model for `plugin.update` undefined → feature deferred (P5).
- **X-sync-ack** the deployed gateway has no `sync_ack` frame (protocol.rs) and
  auto-advances the cursor → UTD hazard. Hard dependency for P0/P1.
- **X5** streaming-via-edits vs 900 msg/min cap + keep-forever storage (P3).
