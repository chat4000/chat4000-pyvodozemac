"""chat4000-pyvodozemac — Python binding around matrix-sdk-crypto's OlmMachine.

The native extension (`_native`) is a Rust cdylib built by maturin. This shim
re-exports its one public class so callers do:

    from chat4000_pyvodozemac import OlmMachine

See `__init__.pyi` for the typed contract. The plugin owns the gateway socket
and sliding sync; this module does ZERO networking — it is a pure crypto state
machine driven by the documented push/pull loop.
"""

from __future__ import annotations

from ._native import PyOlmMachine as OlmMachine  # noqa: F401
from ._native import __version__  # noqa: F401

__all__ = ["OlmMachine", "__version__"]
