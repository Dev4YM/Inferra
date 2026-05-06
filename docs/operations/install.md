# Install Inferra

This guide covers pip installs, Windows service, systemd, Docker, Helm, and macOS LaunchDaemon. **Browse all operator docs** from the documentation sidebar (**Operator guides**) or from the `docs/operations/` directory in the repo. To view them as a site, run `mkdocs serve` from the repo root (see [Documentation home](../index.md)).

## Command-line on PATH (all platforms)

- **Windows (pip):** the `inferra` launcher is in Python’s `Scripts` folder. If `inferra` is not found, use `py -m cli …` or add `Scripts` to your user PATH. When installing the **PyInstaller** service with `deploy/windows/install-service.ps1`, pass **`-AddCliToPath`** to copy `inferra.exe` under `%ProgramData%\Inferra\bin` and append that directory to the **machine** PATH (requires Administrator, same as the service).
- **Linux (.deb / .rpm from `deploy/linux/fpm-package.sh`):** installs `/usr/bin/inferra` automatically.
- **Linux / macOS (pip only):** `python3 -m pip install --user .` puts `inferra` in `~/.local/bin`; ensure that directory is on your PATH (add `export PATH="$HOME/.local/bin:$PATH"` to `~/.profile` or `~/.zshrc`). Alternatively use [`pipx`](https://pipx.pypa.io/) for an isolated CLI on PATH.
- **macOS (`deploy/macos/install.sh`):** ensures `/usr/local/bin/inferra` points at your installed `inferra` when it is not already there.

See also [Troubleshooting](troubleshooting.md) if the dashboard or CLI commands fail after install.

## Local Python (any OS)

From a Git checkout or sdist:

```powershell
python -m pip install -e ".[dev]"
inferra --config inferra.toml setup --yes --skip-connection-test
inferra --config inferra.toml init-db
inferra --config inferra.toml serve --help
```

`setup` writes `inferra.toml`, creates `storage.data_dir`, and runs migrations. `init-db` is idempotent and safe after upgrades. Use `inferra serve` (same as `inferra run`) without `--help` to bind `[server].host`:`[server].port` and open the dashboard at `http://127.0.0.1:7433` by default.

## Windows desktop

Typical developer workstation:

1. Install Python 3.11+ and Git.
2. `python -m pip install -e ".[dev]"` (add `.[windows]` if you need Event Log and service collectors with pywin32).
3. Run the **Local Python** commands above.
4. Optional: enable collectors with `inferra config preset windows-server` or edit `[collectors.*]` in `inferra.toml`.

For optional TLS or LAN binding, set `[server].host` / `[server].port` and keep `require_loopback` consistent with your threat model (see `src/web/http_security.py`).

## Windows Server

For unattended operation, install as a service:

```powershell
python -m pip install -e ".[windows]"
.\deploy\windows\install-service.ps1
```

Run PowerShell **as Administrator** from the repository or deployment root. The script creates `%ProgramData%\Inferra\`, applies ACLs for `SYSTEM` and `Administrators`, runs first-time `setup` when `inferra.toml` is missing, runs **`init-db`**, registers the `Inferra` service with automatic start, and starts it. After install it probes `http://127.0.0.1:<port>/` and prints the **serve log** path (`%ProgramData%\Inferra\logs\serve.log`) if the dashboard is not reachable yet.

Optional **`-AllowFirewall`** opens the inbound TCP port from `[server].port` in `inferra.toml` (default 7433). Optional **`-AddCliToPath`** adds the CLI to the machine PATH (PyInstaller: copies to `%ProgramData%\Inferra\bin`; pip mode: adds Python `Scripts`). Optional **`-KillInferraProcessesBeforeInstall`** aggressively stops the Inferra service and all `inferra.exe` before install (use only when no interactive `inferra serve` must remain — see [Windows exe build](windows_exe_build.md)).

**PyInstaller one-file `inferra.exe`** — use the full pipeline (staged output, promotion with retries, smoke test, isolated venv helper). See **[Windows exe (PyInstaller)](windows_exe_build.md)**.

Quick path:

```powershell
.\deploy\windows\prepare-build-venv.ps1
.\deploy\windows\build-exe.ps1 -Python .\.venv-inferra-build\Scripts\python.exe
.\deploy\windows\install-service.ps1 -InferraExe (Resolve-Path .\dist\inferra.exe) -SkipPipInstall
```

If something still holds `inferra.exe` open during service upgrade, re-run install with **`-KillInferraProcessesBeforeInstall`** (stops all `inferra.exe` — only when no interactive `inferra serve` must stay up).

Service control uses `python -m windows_service` (or `inferra.exe` from the PyInstaller build) with pywin32 verbs (`install`, `remove`, `start`, …). Put standard options **before** the verb, for example `inferra.exe --startup auto install --config … --data-dir …` (pywin32 uses `getopt`, so `install --startup auto` is rejected). Inferra strips `--config` / `--data-dir` before calling pywin32.

Remove the service:

```powershell
.\deploy\windows\uninstall-service.ps1
```

## Linux (systemd)

The unit file `deploy/systemd/inferra.service` uses `DynamicUser=yes`, `StateDirectory=inferra`, and `ProtectSystem=strict` with state under `/var/lib/inferra`. Install the `inferra` binary on `PATH` (for example via `deploy/linux/fpm-package.sh`, which stages `/opt/inferra/venv` and `/usr/bin/inferra`), place config at `/etc/inferra/inferra.toml`, then:

```bash
sudo cp deploy/systemd/inferra.service /lib/systemd/system/inferra.service
sudo systemctl daemon-reload
sudo systemctl enable --now inferra
```

## Docker

From the repository root:

```bash
docker compose up --build
```

Adjust published ports and volume mounts for `inferra.toml` and persistent `data/` per `compose.yaml`.

## Kubernetes

```bash
helm install inferra ./deploy/helm/inferra
```

For in-cluster Kubernetes collection, align `rbac.create`, ServiceAccount bindings, and `[collectors.kubernetes]` in `values.yaml` with your namespace scope. See [Troubleshooting](troubleshooting.md) for RBAC symptoms.

## macOS (LaunchDaemon)

Requires `inferra` on `PATH` (for example `python3 -m pip install .`):

```bash
sudo ./deploy/macos/install.sh
```

Remove:

```bash
sudo ./deploy/macos/uninstall.sh
```

## Release artifacts and signing

See [release_signing.md](release_signing.md) for cosign (container digest), optional signtool (Windows exe), and CycloneDX SBOM outputs from CI.