# Upgrade and migrations

## Schema versions

Every SQLite database embeds a `schema_version` row. On startup and when you run `inferra init-db`, Inferra applies ordered migrations from `src/storage/migrations.py` until each file reaches the current version.

- **events.db** path: `storage.data_dir` + `storage.events_db` (default `./data/events.db`).
- **incidents.db** path: `storage.data_dir` + `storage.incidents_db` (default `./data/incidents.db`).

Migrations are forward-only in production builds: downgrade is refused once newer DDL has been applied.

## CLI maintenance

After upgrading the Python package, run:

```powershell
inferra --config inferra.toml init-db
```

This performs migrate + integrity check (`PRAGMA integrity_check`) on both databases.

Additional storage commands:

```powershell
inferra --config inferra.toml storage verify
inferra --config inferra.toml storage vacuum
inferra --config inferra.toml storage backup path\to\backup
```

Use `storage verify` when investigating suspected corruption; use `storage backup` before risky changes or OS upgrades.

## Backup and restore

1. Stop the Inferra process (service or foreground `serve` / `run`) so files are not mid-transaction.
2. Copy `events.db`, `incidents.db`, and any auxiliary JSON under `storage.data_dir` you rely on (`noise_registry.json`, `baselines/`, `calibration.json`, scoring weight files, and similar).
3. Restore by placing files back with matching paths and ownership, then run `inferra init-db` on the restored files to ensure schema compatibility with the installed code version.

WAL mode creates `-wal` / `-shm` companions briefly while the process runs; backup while stopped avoids copying transient WAL state inconsistently.

## Configuration upgrades

New releases may extend `inferra.toml`. Prefer `inferra config show` after upgrade to compare with your saved copy; merge new keys from `src/config/defaults.toml` rather than deleting custom tuning.
