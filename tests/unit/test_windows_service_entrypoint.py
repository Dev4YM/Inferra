import pytest

import inferra_legacy.windows_service as windows_service


def test_windows_service_entrypoint_uses_importable_service_class(monkeypatch):
    if windows_service.win32serviceutil is None:
        pytest.skip("pywin32 is not installed")

    call = {}

    def fake_handle_command_line(cls, serviceClassString=None):
        call["cls"] = cls
        call["service_class_string"] = serviceClassString
        return 0

    monkeypatch.setattr(windows_service.win32serviceutil, "HandleCommandLine", fake_handle_command_line)

    assert windows_service.main() == 0
    assert call["cls"] is windows_service.InferraWindowsService
    assert call["service_class_string"] == "inferra_legacy.windows_service.InferraWindowsService"
