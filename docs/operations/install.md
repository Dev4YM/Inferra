# Install Inferra

This guide covers local Rust builds, Windows service, systemd, Docker, Helm, and macOS LaunchDaemon. **Browse all operator docs** from the documentation sidebar (**Operator guides**) or from the `docs/operations/` directory in the repo. To view them as a site, run `mkdocs serve` from the repo root (see [Documentation home](../index.md)).

## Command-line on PATH (all platforms)

- **Windows (native Rust build):** `deploy/windows/install-service.ps1` auto-detects `dist/inferra-rust.exe`, `dist/inferra.exe`, or `inferra` on `PATH`, stages the packaged runtime under `%ProgramFiles%\Inferra\`, and keeps config/data/logs under `%ProgramData%\Inferra\`. `-AddCliToPath` appends `%ProgramFiles%\Inferra\bin` to the machine PATH.
- **Linux (.deb / .rpm from `deploy/linux/fpm-package.sh`):** installs `/usr/bin/inferra` as a wrapper around the native Rust runtime under `/opt/inferra/`.
- **Docker / Compose / Helm:** use the native Rust runtime as PID 1.
- **macOS (`deploy/macos/install.sh`):** installs the Rust CLI plus runtime assets under `/usr/local/lib/inferra` and symlinks `/usr/local/bin/inferra`.
- **Archived Python reference:** deprecated Python code lives under `deprecated/` for historical reference only and is not part of the active runtime.

See also [Troubleshooting](troubleshooting.md) if the dashboard or CLI commands fail after install.

## Local repository workflow

From a Git checkout:

```powershell
cargo build --manifest-path src/Cargo.toml -p inferra-cli --release
inferra --config inferra.toml setup --yes
inferra --config inferra.toml init-db
inferra --config inferra.toml collectors status
inferra --config inferra.toml ai status
inferra --config inferra.toml
inferra --config inferra.toml serve
```

`setup` writes `inferra.toml` and sets `storage.data_dir` when requested. `init-db` is idempotent and runs entirely inside the Rust runtime. `collectors status` shows configured collector/runtime state, and `ai status` probes the configured provider. Running bare `inferra` now shows a welcome/status screen with the version, runtime snapshot, and next-step commands. Use `inferra serve` only when you intentionally want to start the local HTTP runtime and dashboard on `[server].host`:`[server].port` (default `http://127.0.0.1:7433`).

## Windows desktop

Typical developer workstation — **recommended scripts** (elevated PowerShell from repo root):

```powershell
# Standard install: incremental build + Program Files staging + service + PATH
.\scripts\install-inferra.ps1

# Full install: npm ci, release build, stop running inferra, reinstall everything
.\scripts\install-inferra.ps1 -Full

# Remove service + Program Files install (keeps %ProgramData%\Inferra)
.\scripts\uninstall-inferra.ps1

# Full remove including config, data, and logs
.\scripts\uninstall-inferra.ps1 -Full
```

Both install and uninstall scripts verify **Machine PATH** and whether `inferra` resolves in the current shell. Installed layout:

| Location | Contents |
|----------|----------|
| `%ProgramFiles%\Inferra\bin\inferra.exe` | Rust CLI (core, API, collectors, service host) |
| `%ProgramFiles%\Inferra\runtime-assets\ui_dist\` | Built web dashboard |
| `%ProgramFiles%\Inferra\runtime-assets\defaults.toml` | Packaged defaults reference |
| `%ProgramData%\Inferra\inferra.toml` | Live config |
| `%ProgramData%\Inferra\data\` | SQLite events/incidents |
| `%ProgramData%\Inferra\logs\` | Service logs |

Build only (no install):

```powershell
.\scripts\build-all.ps1          # incremental
.\scripts\build-all.ps1 -Full    # npm ci + cargo release
```

Manual workflow:

For optional LAN binding, set `[server].host` / `[server].port` and keep `[server].require_loopback` consistent with your threat model.

## Windows Server

For unattended operation, install as a service:

```powershell
.\deploy\windows\build-rust-exe.ps1 -CopyUiBundle
.\deploy\windows\install-service.ps1
```

Run PowerShell **as Administrator** from the repository or deployment root. The script stages the runtime binary plus `runtime-assets` under `%ProgramFiles%\Inferra\`, creates `%ProgramData%\Inferra\`, applies ACLs for `SYSTEM` and `Administrators`, runs first-time `setup` when `inferra.toml` is missing, runs **`init-db`**, registers the `Inferra` service with automatic start, and starts it. Service registration uses the native `inferra service install|remove|start|stop|status` command surface only. After install it probes `http://127.0.0.1:<port>/api/health` and prints the **serve log** path (`%ProgramData%\Inferra\logs\serve.log`) if the runtime is not reachable yet.

Optional **`-AllowFirewall`** opens the inbound TCP port from `[server].port` in `inferra.toml` (default 7433). Optional **`-AddCliToPath`** adds `%ProgramFiles%\Inferra\bin` to the machine PATH. Optional **`-KillInferraProcessesBeforeInstall`** aggressively stops the Inferra service and all `inferra.exe` before install (use only when no interactive `inferra serve` must remain — see [Windows exe build](windows_exe_build.md)). Once the service is installed, use `inferra` or `inferra status` for a quick local status snapshot, and reserve `inferra serve` for non-service foreground runs.

**PyInstaller one-file `inferra.exe`** is now legacy and archived under `deprecated/windows-pyinstaller/`. Use the Rust-native build when possible. The legacy pipeline is still documented in **[Windows exe (PyInstaller)](windows_exe_build.md)**.

Quick path:

```powershell
.\scripts\install-inferra.ps1 -Full -AllowFirewall
```

Or lower-level:

```powershell
.\scripts\build-all.ps1 -Full
.\deploy\windows\install-service.ps1 -InferraExe (Resolve-Path .\dist\inferra-rust.exe) -AddCliToPath
```

If something still holds `inferra.exe` open during service upgrade, re-run install with **`-KillInferraProcessesBeforeInstall`** (stops all `inferra.exe` — only when no interactive `inferra serve` must stay up).

Service control on the Rust-native path uses the built-in service command surface, for example:

```powershell
inferra --config "C:\ProgramData\Inferra\inferra.toml" service status
inferra --config "C:\ProgramData\Inferra\inferra.toml" status
inferra --config "C:\ProgramData\Inferra\inferra.toml" service install --startup auto
inferra service restart
```

The archived Python / pywin32 path remains available only as reference under `deprecated/`; it is not part of service install or runtime execution.

Remove the service:

```powershell
.\scripts\uninstall-inferra.ps1
.\scripts\uninstall-inferra.ps1 -Full
```

## Linux (systemd)

The unit file `deploy/systemd/inferra.service` uses `DynamicUser=yes`, `StateDirectory=inferra`, and `ProtectSystem=strict` with state under `/var/lib/inferra`. Install the packaged Rust runtime on `PATH` (for example via `deploy/linux/fpm-package.sh`, which stages `/opt/inferra/inferra`, `/opt/inferra/runtime-assets`, and `/usr/bin/inferra`), place config at `/etc/inferra/inferra.toml`, then:

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

The image now follows the same packaged layout as the other Rust-first targets: `/app/inferra` plus `/app/runtime-assets`. Adjust published ports and volume mounts for `inferra.toml` and persistent `data/` per `compose.yaml`. For anything beyond host-loopback development, follow the production notes in [Docker Deployment](docker.md).

## Kubernetes

```bash
helm install inferra ./deploy/helm/inferra
```

The chart runs the native `inferra init-db` init container followed by the native `inferra serve` container. For in-cluster Kubernetes collection, align `rbac.create`, ServiceAccount bindings, and `[collectors.kubernetes]` in `values.yaml` with your namespace scope. Production installs must mount the API bearer token Secret described in [Kubernetes Deployment](kubernetes.md). See [Troubleshooting](troubleshooting.md) for RBAC symptoms.

## macOS (LaunchDaemon)

Build the Rust CLI first, then install the LaunchDaemon bundle. The workspace root is `src/Cargo.toml` (Rust crates live under `src/crates/`), and the installer copies the native binary plus frontend/runtime assets under `/usr/local/lib/inferra`:

```bash
sudo ./deploy/macos/install.sh --full
```

Remove:

```bash
sudo ./deploy/macos/uninstall.sh
sudo ./deploy/macos/uninstall.sh --full
```

## Release artifacts and signing

See [release_signing.md](release_signing.md) for cosign (container digest), optional signtool (Windows exe), and CycloneDX SBOM outputs from CI.
