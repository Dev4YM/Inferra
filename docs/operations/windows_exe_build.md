# Windows executable builds

Inferra now has one preferred Windows build path and one archived compatibility path:

- `deploy/windows/build-rust-exe.ps1` builds the native Rust runtime shell and bundles the UI plus active `src/` runtime assets.
- `deprecated/windows-pyinstaller/build-exe.ps1` is legacy only and exists for compatibility during migration.

## Native Rust build

Run from the repository root in PowerShell:

```powershell
Set-Location D:\MYFiles\Projects\py\Inferra
.\deploy\windows\build-rust-exe.ps1 -CopyUiBundle
```

Outputs:

- `dist\inferra-rust.exe` — preferred native Rust artifact when the stable filename is writable
- `dist\inferra-rust-<timestamp>.exe` — fallback artifact name if `dist\inferra-rust.exe` is locked by an older/manual install
- `dist\inferra.exe` — stable compatibility filename when it can be updated in place
- `dist\runtime-assets\ui_dist\` — bundled web UI assets
- `dist\runtime-assets\src\` — bundled active runtime assets used by the native executable

## Recommended flow

1. Build the native runtime:

```powershell
Set-Location D:\MYFiles\Projects\py\Inferra
.\deploy\windows\build-rust-exe.ps1 -CopyUiBundle
```

2. Install the Windows service from an elevated PowerShell:

```powershell
.\deploy\windows\install-service.ps1 -InferraExe (Resolve-Path .\dist\inferra-rust.exe) -AddCliToPath
```

This stages the runtime under `%ProgramFiles%\Inferra\` and keeps mutable state under `%ProgramData%\Inferra\`.

3. Open `http://127.0.0.1:7433/` (or the port in `%ProgramData%\Inferra\inferra.toml`). If the dashboard does not load, inspect `%ProgramData%\Inferra\logs\serve.log`.

## Legacy PyInstaller path

The remainder of this document describes the archived PyInstaller flow. Use it only when you intentionally need the deprecated Python-first packaging path.

## Why a dedicated pipeline exists

On developer and server machines, `**dist\inferra.exe` or `**dist\inferra-rust.exe` can still be locked** if an older/manual install points directly at the project tree:

- The **Inferra** Windows service loads the project-copy binary instead of the packaged install root.
- The service spawns a **child `inferra.exe serve`** process.
- Antivirus or indexing may briefly hold handles during scans.

The current `deploy/windows/install-service.ps1` flow avoids that by copying the runtime into `%ProgramFiles%\Inferra\` before service registration, so normal rebuilds of the repository no longer target the running service executable.

PyInstaller’s default behaviour is to **delete and recreate** the output EXE in `**dist\`**. If the file is mapped for execution, Windows returns `**PermissionError: WinError 5 Access denied`**.

The Inferra pipeline avoids that failure mode by design:

1. **Stage**: PyInstaller writes only under `**dist\_inferra_exe_stage\`** (via `--distpath`), using a dedicated `**build\inferra_exe_work\`** work directory (`--workpath`).
2. **Verify**: Optional `**inferra.exe --version`** smoke test on the staged binary.
3. **Promote**: Copy staged → `**dist\inferra-<version>.exe`** then `**dist\inferra.exe`** with **retries** and (by default) a **second lock-release** pass before overwriting the stable name.

Supporting implementation:

- `**deploy/windows/InferraWindows.psm1`** — legacy reusable cmdlets (`Invoke-InferraWindowsExeBuild`, `Stop-InferraWindowsExecutionLocks`, …).
- `**deprecated/windows-pyinstaller/build-exe.ps1`** — archived human-facing entry script.

## Isolated build virtualenv (details)

If you skipped **Start here** above: a clean `**.venv-inferra-build`** keeps unrelated packages (and bad `**PYTHONPATH`**) out of PyInstaller Analysis. `**deprecated/windows-pyinstaller/prepare-build-venv.ps1`** only invokes `**python.exe -m pip**` for installs/upgrades.

## Standard workstation build (same interpreter every time)

From an elevated **or** normal PowerShell session (elevation is **not** required for PyInstaller itself):

```powershell
.\deprecated\windows-pyinstaller\build-exe.ps1
```

Parameters (all optional):


| Parameter                | Purpose                                                                                  |
| ------------------------ | ---------------------------------------------------------------------------------------- |
| `-Python`                | Interpreter that has `PyInstaller` and the project installed (default `python`).         |
| `-SkipReleaseLocks`      | Do **not** stop the Inferra service / `inferra.exe`. Use on **clean CI** agents only.    |
| `-CleanPyInstallerWork`  | Delete `build\inferra_exe_work\` before building (helps after hook or dependency churn). |
| `-NoSmokeTest`           | Skip `inferra.exe --version` on the staged binary (not recommended locally).             |
| `-LockReleaseTimeoutSec` | Max seconds to wait for the Windows service to reach **Stopped** (default `120`).        |
| `-PublishCopyAttempts`   | Retries when copying into `dist\` (default `48`).                                        |


### Outputs

After a successful **legacy PyInstaller** run you should have a staged or promoted `inferra.exe` from the archived Python-first path. It does **not** produce `dist\inferra-rust.exe`; that file comes from `deploy/windows/build-rust-exe.ps1`.

Exit codes:

- `**0**` — success.
- `**1**` — hard failure (PyInstaller, smoke test, missing artifacts, …).
- `**2**` — promotion failed (staged exe is still valid; see message).

## CI / automation

Use the native Rust build in CI and release automation:

```text
pwsh -NoProfile -File ./deploy/windows/build-rust-exe.ps1 -CopyUiBundle
```

Release builds follow this pattern in `.github/workflows/release.yml`.

## Service install interactions

`deploy/windows/install-service.ps1` supports `**-KillInferraProcessesBeforeInstall**` (opt-in). When set, it imports `**InferraWindows.psm1**` and runs `**Stop-InferraWindowsExecutionLocks**` before the rest of the script.

Use this **only** when you intentionally want every `inferra.exe` stopped (for example you know no developer is running `inferra serve` on that machine). Default behaviour remains safe for shared workstations.

## Troubleshooting


| Symptom                                                                                        | Likely cause                                | Mitigation                                                                                                                                                                                                |
| ---------------------------------------------------------------------------------------------- | ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `**ERROR: To modify pip, please run … python.exe -m pip`** during `**prepare-build-venv.ps1`** | `**pip.exe` cannot self-upgrade** (pip 24+) | Already fixed in repo: script uses `**python -m pip`**. Update your checkout or run: `**.\venv\Scripts\python.exe -m pip install --upgrade pip wheel`**. Or `**.\deprecated\windows-pyinstaller\prepare-build-venv.ps1 -SkipPipUpgrade**`. |
| Promotion exits `**2**` or copy errors mention **sharing violation** / **being used**          | Lock on `**dist\inferra.exe`**              | Run `**Stop-Service Inferra`**, confirm `**Get-Process inferra`** is empty, retry. Import `**InferraWindows.psm1**` and run `**Stop-InferraWindowsExecutionLocks**`.                                      |
| PyInstaller Analysis pulls **torch**, huge `**pkg`**, multi-minute builds                      | Polluted global Python or `**PYTHONPATH`**  | Use `**prepare-build-venv.ps1`**; ensure no foreign paths are injected into the build shell.                                                                                                              |
| `**inferra.exe --version` smoke failure**                                                      | Frozen metadata / broken build              | Inspect PyInstaller warnings; run staged exe manually from `**dist\_inferra_exe_stage\`**.                                                                                                                |
| Service runs but HTTP never listens                                                            | Child `**serve`** crashed                   | See `**%ProgramData%\Inferra\logs\serve.log`** (written by the Windows service wrapper).                                                                                                                  |


## Manual commands (advanced)

If you must invoke PyInstaller yourself, use the archived flow under `**deprecated/windows-pyinstaller/**`:

```powershell
python -m PyInstaller --noconfirm `
  --distpath dist\_inferra_exe_stage `
  --workpath build\inferra_exe_work `
  deploy\windows\inferra.spec
```

Then copy the staged binary into `**dist\**` only after locks are released.

For Authenticode signing of `**dist\inferra.exe**`, see [Release signing](release_signing.md).