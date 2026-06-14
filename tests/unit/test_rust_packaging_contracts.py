from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def test_release_version_manifests_match_canonical_version() -> None:
    import subprocess
    import sys

    result = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "version.py"), "verify"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout


def test_canonical_version_file_exists() -> None:
    version = (ROOT / "VERSION").read_text(encoding="utf-8").strip()
    assert version
    parts = version.split(".")
    assert len(parts) == 3
    assert all(part.isdigit() for part in parts)


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
    compose = read("compose.yaml")
    assert "FROM debian:bookworm-slim" in dockerfile
    assert "COPY src/Cargo.toml src/Cargo.lock ./src/" in dockerfile
    assert "/app/runtime-assets/src" not in dockerfile
    assert "INFERRA_PYTHON" not in dockerfile
    assert "INFERRA_PYTHON" not in entrypoint
    assert '/app/inferra --config "${CONFIG_PATH}" init-db' in entrypoint
    assert "HEALTHCHECK" in dockerfile
    assert "/healthz" in dockerfile
    assert "127.0.0.1:7433:7433" in compose
    assert "/healthz" in compose


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


def test_helm_chart_wires_auth_and_probes() -> None:
    values = read("deploy/helm/inferra/values.yaml")
    configmap = read("deploy/helm/inferra/templates/configmap.yaml")
    deployment = read("deploy/helm/inferra/templates/deployment.yaml")
    secret = read("deploy/helm/inferra/templates/secret.yaml")
    assert 'authTokenEnv: "INFERRA_API_TOKEN"' in values
    assert "require_loopback = {{ .Values.server.requireLoopback }}" in configmap
    assert 'auth_token_env = "{{ .Values.server.authTokenEnv }}"' in configmap
    assert 'fail "server.authTokenEnv is required when server.requireLoopback=false"' in configmap
    assert "secretKeyRef" in deployment
    assert "/healthz" in deployment
    assert "/readyz" in deployment
    assert "readOnlyRootFilesystem: true" in values
    assert "seccompProfile" in values
    assert "{{ .Values.auth.token | quote }}" in secret
