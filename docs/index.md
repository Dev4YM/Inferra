# Inferra documentation

Inferra is a local-first runtime failure explanation system: it reads operational signals, stores them in SQLite, builds deterministic incident hypotheses, and optionally uses an Ollama-compatible model for operator-facing language.

## How to read these docs

- **In GitHub or your editor:** start here, then open the **Operator guides** section in the navigation (or open files under [`docs/operations/`](operations/) in the repo: `install.md`, `troubleshooting.md`, and so on).
- **As a local website (recommended):** from the repository root, install doc tooling and serve:

```powershell
python -m pip install -e ".[docs]"
mkdocs serve
```

Then open the URL MkDocs prints (typically [http://127.0.0.1:8000](http://127.0.0.1:8000)).

- **Pre-built HTML:** after `mkdocs build`, open `site/index.html` in a browser (paths are relative; serving via `mkdocs serve` is smoother).

Deployment-focused guides include [Install](operations/install.md), **[Windows exe — start here](operations/windows_exe_build.md)** (PyInstaller checklist), and [Troubleshooting](operations/troubleshooting.md).

The root [README.md](../README.md) has the shortest quick start (`pip install`, `setup`, `serve`) and links back here for deeper install paths.

Architecture decisions are under **Architecture Decision Records**. Design notes live under **Planning**, especially [Architecture overview](planning/architecture_overview.md), the [implementation index](planning/implementation_index.md), and the [full build architecture plan](planning/full_build_architecture_plan.md).
