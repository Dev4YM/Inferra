from __future__ import annotations

import multiprocessing as mp
import sqlite3
import sys
import time
from pathlib import Path

import pytest

from storage.migrations import integrity_check, migrate


def _open_transaction_then_sleep(db_path: Path, barrier: mp.synchronize.Barrier) -> None:
    conn = sqlite3.connect(db_path)
    conn.execute("BEGIN IMMEDIATE")
    conn.execute(
        """
        INSERT INTO collector_state(collector_id, state_key, state_value, updated_at)
        VALUES ('chaos', 'cursor', 'open', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        """,
    )
    barrier.wait()
    time.sleep(60)


@pytest.mark.chaos
@pytest.mark.skipif(sys.platform == "win32", reason="POSIX SIGKILL mid-transaction coverage")
def test_sigkill_during_open_transaction_keeps_prior_commits(tmp_path: Path) -> None:
    db_path = tmp_path / "events.db"
    migrate(db_path)
    conn = sqlite3.connect(db_path)
    try:
        conn.execute(
            """
            INSERT INTO collector_state(collector_id, state_key, state_value, updated_at)
            VALUES ('seed', 'k', 'committed', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            """,
        )
        conn.commit()
    finally:
        conn.close()

    barrier = mp.Barrier(2)
    proc = mp.Process(target=_open_transaction_then_sleep, args=(db_path, barrier), daemon=True)
    proc.start()
    barrier.wait(timeout=10)
    time.sleep(0.3)
    if proc.is_alive():
        proc.kill()
        proc.join(timeout=10)

    assert integrity_check(db_path) is True
    conn = sqlite3.connect(db_path)
    try:
        row = conn.execute(
            "SELECT COUNT(*) FROM collector_state WHERE collector_id = 'seed'",
        ).fetchone()
        assert int(row[0]) == 1
        row2 = conn.execute(
            "SELECT COUNT(*) FROM collector_state WHERE collector_id = 'chaos'",
        ).fetchone()
        assert int(row2[0]) == 0
    finally:
        conn.close()
