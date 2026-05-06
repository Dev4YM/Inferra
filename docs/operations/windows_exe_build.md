# Windows one-file executable (PyInstaller)

**Use this page as your single checklist.** Commands use `**python -m pip`** / `**python.exe -m pip`** on purpose: calling `**Scripts\pip.exe**` to upgrade pip fails on pip 24+ with *“To modify pip, please run … python.exe -m pip …”*.

## Start here (Windows, copy/paste)

Run from the **repository root** in PowerShell. Paths shown use `D:\MYFiles\Projects\py\Inferra` — change to yours or run `cd` first.

### 1) One-time: isolated build virtualenv

```powershell
Set-Location D:\MYFiles\Projects\py\Inferra
.\deploy\windows\prepare-build-venv.ps1
```

If pip upgrade is blocked by policy, try:

```powershell
.\deploy\windows\prepare-build-venv.ps1 -SkipPipUpgrade
```

### 2) Build `inferra.exe` (staged output + promote + smoke test)

```powershell
.\deploy\windows\build-exe.ps1 -Python .\.venv-inferra-build\Scripts\python.exe
```

If the **Inferra** service is running (or another `inferra.exe` is open), this script stops them before overwriting `**dist\inferra.exe`**.

When you see `**Primary artifact: ...\dist\inferra.exe`**, the build is done — continue to **step 3**. To sanity-check: `**.\dist\inferra.exe --version`** should match `**[project].version`** in `pyproject.toml`. If you only need the portable binary (no service), you can stop after this step.

### 3) Install Windows service (Administrator PowerShell)

```powershell
Set-Location D:\MYFiles\Projects\py\Inferra
.\deploy\windows\install-service.ps1 -InferraExe (Resolve-Path .\dist\inferra.exe) -SkipPipInstall -AddCliToPath
```

### 4) Open the dashboard

After install completes, browse `**http://127.0.0.1:7433/**` (or the port in `%ProgramData%\Inferra\inferra.toml`). If it does not load, read `**%ProgramData%\Inferra\logs\serve.log**`.

---

This document is the **canonical** procedure for producing `inferra.exe`. It replaces ad-hoc `python -m PyInstaller …` examples when you care about reliability on real workstations.

## Why a dedicated pipeline exists

On developer and server machines, `**dist\inferra.exe` is often locked**:

- The **Inferra** Windows service loads the binary.
- The service spawns a **child `inferra.exe serve`** process.
- Antivirus or indexing may briefly hold handles during scans.

PyInstaller’s default behaviour is to **delete and recreate** the output EXE in `**dist\`**. If the file is mapped for execution, Windows returns `**PermissionError: WinError 5 Access denied`**.

The Inferra pipeline avoids that failure mode by design:

1. **Stage**: PyInstaller writes only under `**dist\_inferra_exe_stage\`** (via `--distpath`), using a dedicated `**build\inferra_exe_work\`** work directory (`--workpath`).
2. **Verify**: Optional `**inferra.exe --version`** smoke test on the staged binary.
3. **Promote**: Copy staged → `**dist\inferra-<version>.exe`** then `**dist\inferra.exe`** with **retries** and (by default) a **second lock-release** pass before overwriting the stable name.

Supporting implementation:

- `**deploy/windows/InferraWindows.psm1`** — reusable cmdlets (`Invoke-InferraWindowsExeBuild`, `Stop-InferraWindowsExecutionLocks`, …).
- `**deploy/windows/build-exe.ps1`** — thin entry script for humans and CI.

## Isolated build virtualenv (details)

If you skipped **Start here** above: a clean `**.venv-inferra-build`** keeps unrelated packages (and bad `**PYTHONPATH`**) out of PyInstaller Analysis. `**prepare-build-venv.ps1**` only invokes `**python.exe -m pip**` for installs/upgrades.

## Standard workstation build (same interpreter every time)

From an elevated **or** normal PowerShell session (elevation is **not** required for PyInstaller itself):

```powershell
.\deploy\windows\build-exe.ps1
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

After success you should have:

- `**dist\inferra.exe`** — stable name consumed by `**install-service.ps1`** and release signing.
- `**dist\inferra-<pyproject-version>.exe**` — immutable, versioned artifact (matches `[project].version` in `pyproject.toml`).
- `**dist\_inferra_exe_stage\inferra.exe**` — last staged build (still overwritten on the next build).

Exit codes:

- `**0**` — success.
- `**1**` — hard failure (PyInstaller, smoke test, missing artifacts, …).
- `**2**` — promotion failed (staged exe is still valid; see message).

## CI / automation

Use `**-SkipReleaseLocks**` because agents do not run the Inferra service:

```text
pwsh -NoProfile -File ./deploy/windows/build-exe.ps1 -SkipReleaseLocks -Python python -CleanPyInstallerWork
```

Release builds follow this pattern in `.github/workflows/release.yml`.

## Service install interactions

`deploy/windows/install-service.ps1` supports `**-KillInferraProcessesBeforeInstall**` (opt-in). When set, it imports `**InferraWindows.psm1**` and runs `**Stop-InferraWindowsExecutionLocks**` before the rest of the script.

Use this **only** when you intentionally want every `inferra.exe` stopped (for example you know no developer is running `inferra serve` on that machine). Default behaviour remains safe for shared workstations.

## Troubleshooting


| Symptom                                                                                        | Likely cause                                | Mitigation                                                                                                                                                                                                |
| ---------------------------------------------------------------------------------------------- | ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `**ERROR: To modify pip, please run … python.exe -m pip`** during `**prepare-build-venv.ps1`** | `**pip.exe` cannot self-upgrade** (pip 24+) | Already fixed in repo: script uses `**python -m pip`**. Update your checkout or run: `**.\venv\Scripts\python.exe -m pip install --upgrade pip wheel`**. Or `**prepare-build-venv.ps1 -SkipPipUpgrade**`. |
| Promotion exits `**2**` or copy errors mention **sharing violation** / **being used**          | Lock on `**dist\inferra.exe`**              | Run `**Stop-Service Inferra`**, confirm `**Get-Process inferra**` is empty, retry. Import `**InferraWindows.psm1**` and run `**Stop-InferraWindowsExecutionLocks**`.                                      |
| PyInstaller Analysis pulls **torch**, huge `**pkg`**, multi-minute builds                      | Polluted global Python or `**PYTHONPATH`**  | Use `**prepare-build-venv.ps1**`; ensure no foreign paths are injected into the build shell.                                                                                                              |
| `**inferra.exe --version` smoke failure**                                                      | Frozen metadata / broken build              | Inspect PyInstaller warnings; run staged exe manually from `**dist\_inferra_exe_stage\`**.                                                                                                                |
| Service runs but HTTP never listens                                                            | Child `**serve`** crashed                   | See `**%ProgramData%\Inferra\logs\serve.log**` (written by the Windows service wrapper).                                                                                                                  |


## Manual commands (advanced)

If you must invoke PyInstaller yourself, mirror what `**Invoke-InferraWindowsExeBuild**` does:

```powershell
python -m PyInstaller --noconfirm `
  --distpath dist\_inferra_exe_stage `
  --workpath build\inferra_exe_work `
  deploy\windows\inferra.spec
```

Then copy the staged binary into `**dist\**` only after locks are released.

For Authenticode signing of `**dist\inferra.exe**`, see [Release signing](release_signing.md).