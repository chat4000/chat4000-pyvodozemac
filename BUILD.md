# Building chat4000-pyvodozemac

## Status of this checkout (read first)

This crate was **authored offline** — no network to fetch crates, and
`matrix-sdk-crypto` is **not** in the local cargo cache — so it has **not been
compiled here**. The code is written against the matrix-sdk-crypto **0.11** API
*shape*; the call sequence, the JSON boundary, and the `{id,kind,method,path,body}`
request contract are correct, but a set of version-sensitive symbols (marked
`(verify)` in `src/`) need a first compiler pass to reconcile against the exact
pinned release. They are concentrated in:

- `src/requests.rs` — the `AnyOutgoingRequest` enum path/variants.
- `src/machine.rs` — `OlmMachine::with_store` arity, `EncryptionSyncChanges` /
  `DecryptionSettings` / `EncryptionSettings` fields, the `decrypt_room_event`
  settings arg, and the ruma response paths in `mark_request_as_sent`.

Expect the first `cargo build` to surface a handful of renamed paths/fields. The
design does not change; only symbol names do.

## Prerequisites

- Rust ≥ 1.82 (workspace uses 1.96).
- `maturin` ≥ 1.7 (`pipx install maturin` or `uv tool install maturin`).
- **Network** — to fetch `matrix-sdk-crypto`, `matrix-sdk-sqlite`, `vodozemac`,
  `ruma`, `pyo3` and their closure.

## Dev build (editable, into the current venv)

```bash
maturin develop --release
python -c "from chat4000_pyvodozemac import OlmMachine, __version__; print(__version__)"
```

## Release wheels (what the plugin ships)

```bash
# One abi3 wheel per platform, CPython >=3.11.
maturin build --release
# Cross-platform in CI: use cibuildwheel or maturin-action for
#   manylinux2014 + musllinux (Linux)  and  universal2 (macOS, + notarization).
```

The plugin depends on this wheel; pin its version in the plugin's `pyproject.toml`
once published (private index or bundled).

## Pinning policy

`matrix-sdk-crypto` minors move the `OlmMachine` API. Treat a bump as deliberate:
re-resolve the `(verify)` symbols, run the crypto round-trip test (P0), then pin.
Never float the version.

## Tests

```bash
cargo test            # pure-Rust unit tests (marshaling shapes)
maturin develop && pytest tests/   # Python-level smoke (needs the built module)
```
