from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def test_windows_rust_build_copies_flat_runtime_assets() -> None:
    script = read("deploy/windows/build-rust-exe.ps1")
    assert '$rustRoot = Join-Path $repoRoot "src"' in script
    assert "CopyPythonWorker is ignored" in script
    assert 'Copy-Item (Join-Path $uiDist "*") $uiTarget -Recurse -Force' in script
    assert 'Copy-Item (Join-Path $repoRoot "src\\*") $srcTarget -Recurse -Force' not in script
    assert "could not overwrite $PreferredPath because it is locked" in script


def test_windows_service_install_is_rust_only() -> None:
    script = read("deploy/windows/install-service.ps1")
    assert "function Ensure-InferraPythonEnv" not in script
    assert "python-env" not in script
    assert "INFERRA_PYTHON" not in script
    assert '"--startup", "auto"' in script
    assert '[Environment]::GetFolderPath("ProgramFiles")' in script
    assert '"--ui-dist", $installedUiDist' in script
    assert "--python" not in script
    assert "/api/health" in script


def test_docker_runtime_is_native_binary_only() -> None:
    dockerfile = read("Dockerfile")
    entrypoint = read("deploy/docker-entrypoint.sh")
    assert "FROM debian:bookworm-slim" in dockerfile
    assert "COPY src/Cargo.toml src/Cargo.lock ./src/" in dockerfile
    assert "/app/runtime-assets/src" not in dockerfile
    assert "INFERRA_PYTHON" not in dockerfile
    assert "INFERRA_PYTHON" not in entrypoint
    assert '/app/inferra --config "${CONFIG_PATH}" init-db' in entrypoint


def test_linux_and_systemd_entrypoints_are_rust_only() -> None:
    package_script = read("deploy/linux/fpm-package.sh")
    unit = read("deploy/systemd/inferra.service")
    assert 'cargo build --manifest-path "${ROOT}/src/Cargo.toml" -p inferra-cli --release' in package_script
    assert 'runtime-assets/src' not in package_script
    assert "INFERRA_PYTHON" not in package_script
    assert "python3 >=" not in package_script
    assert "python3 (>= " not in package_script
    assert "python3 -c" not in package_script
    assert 'awk -F \'"\' ' in package_script
    assert "INFERRA_PYTHON" not in unit


def test_helm_and_macos_launchd_drop_python_runtime_env() -> None:
    chart = read("deploy/helm/inferra/templates/deployment.yaml")
    plist = read("deploy/macos/com.inferra.agent.plist")
    assert "INFERRA_PYTHON" not in chart
    assert "INFERRA_PYTHON" not in plist
    assert "init-db" in plist
