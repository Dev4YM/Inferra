# Inferra Documentation

Inferra is a local-first AI-integrated runtime intelligence control plane. It reads operational signals, stores them in SQLite, builds deterministic incident hypotheses, maps runtime behavior back to local workspace context, and uses AI to guide safe investigation without mutating observed systems.

## How To Read These Docs

- **In GitHub or your editor:** start here, then open the **Operator guides** section in the navigation, beginning with [Install](operations/install.md) or [Troubleshooting](operations/troubleshooting.md).
- **As a local website (recommended):** from the repository root, install doc tooling and serve:

```powershell
python -m pip install -e ".[docs]"
mkdocs serve
```

Then open the URL MkDocs prints, typically [http://127.0.0.1:8000](http://127.0.0.1:8000).

- **Pre-built HTML:** after `mkdocs build`, open `site/index.html` in a browser. Serving through `mkdocs serve` is smoother for local navigation.

Deployment-focused guides include [Install](operations/install.md), **[Windows exe - start here](operations/windows_exe_build.md)**, and [Troubleshooting](operations/troubleshooting.md).

The repository root README has the shortest quick start (`pip install`, `onboard`, `guide`, `serve`) and links back here for deeper install paths.

Architecture decisions are under **Architecture Decision Records**. Design notes live under **Planning**, especially [Architecture overview](planning/architecture_overview.md), the [implementation index](planning/implementation_index.md), and the [full build architecture plan](planning/full_build_architecture_plan.md).
