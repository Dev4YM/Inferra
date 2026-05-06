from __future__ import annotations

import asyncio
import ctypes
import glob
import os
import platform
import re
from dataclasses import dataclass, field
from pathlib import Path

from collectors.base import PollingCollector
from core.time import utc_now
from events.models import RawEvent

if platform.system().lower() == "windows":  # pragma: no cover
    import msvcrt


@dataclass
class FileState:
    offset: int = 0
    identity: tuple[int, int] | tuple[str, int, float] | None = None
    pending_lines: list[str] = field(default_factory=list)


class FileCollector(PollingCollector):
    source_type = "file"

    def __init__(
        self,
        path: str | Path | None = None,
        *,
        glob_pattern: str | None = None,
        service_id: str | None = None,
        service_id_from_filename: bool = False,
        multiline_pattern: str | None = None,
        poll_interval_seconds: float = 1.0,
        start_at_end: bool = False,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(poll_interval_seconds=poll_interval_seconds, emit_timeout_seconds=emit_timeout_seconds)
        if path is None and not glob_pattern:
            raise ValueError("FileCollector requires either a path or a glob pattern")
        self.path = Path(path) if path is not None else None
        self.glob_pattern = glob_pattern
        self.service_id = service_id
        self.service_id_from_filename = service_id_from_filename
        self.multiline_pattern = re.compile(multiline_pattern) if multiline_pattern else None
        self.start_at_end = start_at_end
        self._states: dict[Path, FileState] = {}

    @property
    def collector_id(self) -> str:
        target = self.glob_pattern or (str(self.path) if self.path is not None else "unknown")
        return f"file://{target}"

    async def run(self, queue: asyncio.Queue[RawEvent]) -> None:
        await self._mark_running()
        if self.start_at_end:
            for path in self._resolve_paths():
                if path.exists():
                    state = self._states.setdefault(path, FileState())
                    state.offset = path.stat().st_size
                    state.identity = self._file_identity(path)
        try:
            while not self._should_stop():
                try:
                    await self.collect_once(queue)
                except Exception as exc:  # pragma: no cover
                    self._record_error(exc)
                try:
                    await asyncio.wait_for(self._stop_event.wait(), timeout=self.poll_interval_seconds)
                except TimeoutError:
                    continue
        finally:
            await self._mark_stopped()

    async def collect_existing(self, sink: asyncio.Queue[RawEvent]) -> int:
        return await self.collect_once(sink, read_to_eof=True)

    async def collect_once(self, sink: asyncio.Queue[RawEvent], read_to_eof: bool = False) -> int:
        emitted = 0
        for path in self._resolve_paths():
            emitted += await self._poll_path(path, sink, read_to_eof=read_to_eof)
        return emitted

    async def _poll_path(self, path: Path, sink: asyncio.Queue[RawEvent], read_to_eof: bool) -> int:
        if not path.exists():
            return 0
        state = self._states.setdefault(path, FileState())
        try:
            identity = self._file_identity(path)
            size = path.stat().st_size
            if state.identity is not None and state.identity != identity:
                state.offset = 0
                state.pending_lines.clear()
            elif size < state.offset:
                state.offset = 0
                state.pending_lines.clear()
            emitted = 0
            with _open_text_file(path) as handle:
                handle.seek(state.offset)
                while True:
                    line = handle.readline()
                    if not line:
                        break
                    state.offset = handle.tell()
                    emitted += await self._handle_line(path, line.rstrip("\r\n"), state, sink)
                    if not read_to_eof:
                        await asyncio.sleep(0)
            state.identity = identity
            if read_to_eof and state.pending_lines:
                emitted += await self._emit_buffer(path, state, sink)
            return emitted
        except OSError as exc:
            self._record_error(exc)
            return 0

    async def _handle_line(self, path: Path, line: str, state: FileState, sink: asyncio.Queue[RawEvent]) -> int:
        if self.multiline_pattern is None:
            return 1 if await self.emit(sink, self._raw_event(path, line, state.offset)) else 0
        matches_start = bool(self.multiline_pattern.search(line))
        if matches_start and state.pending_lines:
            emitted = 1 if await self.emit(sink, self._raw_event(path, "\n".join(state.pending_lines), state.offset)) else 0
            state.pending_lines = [line]
            return emitted
        state.pending_lines.append(line)
        return 0

    async def _emit_buffer(self, path: Path, state: FileState, sink: asyncio.Queue[RawEvent]) -> int:
        payload = "\n".join(state.pending_lines)
        state.pending_lines.clear()
        return 1 if await self.emit(sink, self._raw_event(path, payload, state.offset)) else 0

    def _resolve_paths(self) -> list[Path]:
        if self.path is not None:
            return [self.path]
        assert self.glob_pattern is not None
        return sorted(Path(item) for item in glob.glob(self.glob_pattern, recursive=True))

    def _raw_event(self, path: Path, payload: str, offset: int) -> RawEvent:
        service_id = self.service_id
        if self.service_id_from_filename:
            service_id = path.stem
        return RawEvent(
            source_type=self.source_type,
            source_id=f"file://{path}",
            raw_payload=payload,
            collected_at=utc_now(),
            metadata={"path": str(path), "raw_offset": offset, "service_id": service_id},
        )

    def _file_identity(self, path: Path) -> tuple[int, int] | tuple[str, int, float]:
        stat = path.stat()
        if platform.system().lower() == "windows":
            return (str(path.resolve()), stat.st_size, stat.st_mtime)
        return (stat.st_dev, stat.st_ino)


def _open_text_file(path: Path):
    if platform.system().lower() != "windows":
        return path.open("r", encoding="utf-8", errors="replace")
    return _WindowsSharedReader(path)


class _WindowsSharedReader:
    FILE_SHARE_READ = 0x00000001
    FILE_SHARE_WRITE = 0x00000002
    FILE_SHARE_DELETE = 0x00000004
    OPEN_EXISTING = 3
    GENERIC_READ = 0x80000000
    INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value

    def __init__(self, path: Path) -> None:
        self._path = path
        self._handle = None
        self._file = None

    def __enter__(self):
        handle = ctypes.windll.kernel32.CreateFileW(
            str(self._path),
            self.GENERIC_READ,
            self.FILE_SHARE_READ | self.FILE_SHARE_WRITE | self.FILE_SHARE_DELETE,
            None,
            self.OPEN_EXISTING,
            0,
            None,
        )
        if handle == self.INVALID_HANDLE_VALUE:
            raise OSError(f"unable to open {self._path}")
        fd = msvcrt.open_osfhandle(handle, os.O_RDONLY)  # type: ignore[name-defined]
        self._handle = handle
        self._file = os.fdopen(fd, "r", encoding="utf-8", errors="replace")
        return self._file

    def __exit__(self, exc_type, exc, tb) -> None:
        if self._file is not None:
            self._file.close()
