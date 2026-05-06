from __future__ import annotations

import pytest

import cli


def test_root_help_lists_top_level_and_readme_commands(capsys: pytest.CaptureFixture[str]) -> None:
    parser = cli._build_parser()

    with pytest.raises(SystemExit) as exc:
        parser.parse_args(["--help"])

    assert exc.value.code == 0
    help_text = capsys.readouterr().out
    assert "usage: inferra" in help_text
    for command in (
        "setup",
        "serve",
        "run",
        "run-collectors",
        "init-db",
        "reason-incident",
        "check-config",
        "ai",
        "collectors",
        "collect-host",
        "collect-processes",
        "collect-services",
        "collect-eventlog",
        "collect-syslog",
        "collect-journald",
        "collect-kubernetes",
        "config",
        "reset-weights",
        "calibration",
        "completion",
    ):
        assert command in help_text
    for command in cli._README_HELP_COMMANDS:
        assert command in help_text


def test_completion_command_returns_shellcode(monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]) -> None:
    class FakeArgcomplete:
        @staticmethod
        def shellcode(executables, shell):
            assert executables == ["inferra"]
            assert shell == "powershell"
            return "Register-ArgumentCompleter inferra"

        @staticmethod
        def autocomplete(parser: object) -> None:
            return None

    monkeypatch.setattr(cli, "argcomplete", FakeArgcomplete())

    result = cli.main(["--json", "completion", "powershell"])
    payload = capsys.readouterr().out

    assert result == 0
    assert "Register-ArgumentCompleter inferra" in payload


@pytest.mark.parametrize(
    ("argv", "expected_commands"),
    [
        (["ai", "--help"], ("status", "models", "pull", "test")),
        (["collectors", "--help"], ("status", "start", "stop")),
        (["config", "--help"], ("show", "get", "set", "preset")),
    ],
)
def test_group_help_outputs_include_expected_subcommands(
    argv: list[str],
    expected_commands: tuple[str, ...],
    capsys: pytest.CaptureFixture[str],
) -> None:
    parser = cli._build_parser()

    with pytest.raises(SystemExit) as exc:
        parser.parse_args(argv)

    assert exc.value.code == 0
    help_text = capsys.readouterr().out
    for command in expected_commands:
        assert command in help_text


def test_project_version_reads_bundled_pyproject_when_frozen(monkeypatch: pytest.MonkeyPatch, tmp_path) -> None:
    bundled = tmp_path / "pyproject.toml"
    bundled.write_text('[project]\nversion = "9.8.7"\n', encoding="utf-8")
    monkeypatch.setattr(cli.sys, "frozen", True, raising=False)
    monkeypatch.setattr(cli.sys, "_MEIPASS", str(tmp_path), raising=False)
    assert cli._project_version() == "9.8.7"
