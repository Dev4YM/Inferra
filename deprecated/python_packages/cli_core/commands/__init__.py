"""Per-domain CLI command handler modules.

Handlers live here in domain-grouped modules. They use late binding to
``inferra_legacy.cli`` (imported as ``cli`` inside the function body) so that test
suites that monkeypatch helpers like ``cli._local_api_json`` continue to work.

``inferra_legacy.cli`` registers each handler with its argparse subcommand and
re-exports them under their original ``_handle_*`` names for backward compatibility.
"""

from __future__ import annotations

__all__: list[str] = []
