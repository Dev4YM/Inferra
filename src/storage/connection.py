from __future__ import annotations

import sqlite3
import threading
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

from core.logging import get_logger

_log = get_logger(__name__)


def connect_sqlite(
    path: Path,
    *,
    wal_mode: bool = True,
    busy_timeout_ms: int = 5000,
    mmap_size_bytes: int = 0,
    read_only: bool = False,
) -> sqlite3.Connection:
    path.parent.mkdir(parents=True, exist_ok=True)
    if read_only:
        uri = path.resolve().as_uri() + "?mode=ro"
        conn = sqlite3.connect(uri, uri=True, check_same_thread=False)
    else:
        conn = sqlite3.connect(path, check_same_thread=False)
    conn.row_factory = sqlite3.Row
    if wal_mode:
        conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute(f"PRAGMA busy_timeout={busy_timeout_ms}")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute("PRAGMA temp_store=MEMORY")
    conn.execute("PRAGMA auto_vacuum=INCREMENTAL")
    if mmap_size_bytes > 0:
        conn.execute(f"PRAGMA mmap_size={mmap_size_bytes}")
    return conn


class SqliteConnectionPool:
    def __init__(
        self,
        path: Path,
        *,
        wal_mode: bool = True,
        busy_timeout_ms: int = 5000,
        mmap_size_bytes: int = 0,
    ) -> None:
        self._path = path
        self._wal_mode = wal_mode
        self._busy_timeout_ms = busy_timeout_ms
        self._mmap_size_bytes = mmap_size_bytes
        self._guard = threading.Lock()
        self._closed = False
        self._writer = connect_sqlite(
            path,
            wal_mode=wal_mode,
            busy_timeout_ms=busy_timeout_ms,
            mmap_size_bytes=mmap_size_bytes,
        )
        self._readers: dict[int, sqlite3.Connection] = {}

    def writer(self) -> sqlite3.Connection:
        return self._writer

    def reader(self) -> sqlite3.Connection:
        ident = threading.get_ident()
        with self._guard:
            conn = self._readers.get(ident)
            if conn is None:
                conn = connect_sqlite(
                    self._path,
                    wal_mode=self._wal_mode,
                    busy_timeout_ms=self._busy_timeout_ms,
                    mmap_size_bytes=self._mmap_size_bytes,
                )
                self._readers[ident] = conn
            return conn

    def close(self) -> None:
        with self._guard:
            if self._closed:
                return
            self._closed = True
            for conn in self._readers.values():
                conn.close()
            self._readers.clear()
            self._writer.close()


@contextmanager
def transaction(conn: sqlite3.Connection) -> Iterator[None]:
    conn.execute("BEGIN IMMEDIATE")
    try:
        yield
    except Exception:
        conn.rollback()
        raise
    else:
        conn.commit()
