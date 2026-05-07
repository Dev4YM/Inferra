# Upgrade and migrations

## Schema versions

Every SQLite database embeds a `schema_version` row. On startup and when you run `inferra init-db`, the native Rust runtime in `src/crates/inferra-storage/` ensures the expected schema exists for both databases.

- **events.db** path: `storage.data_dir` + `storage.events_db` (default `./data/events.db`).
- **incidents.db** path: `storage.data_dir` + `storage.incidents_db` (default `./data/incidents.db`).

Migrations are forward-only in production builds: downgrade is refused once newer DDL has been applied.

## CLI maintenance

After upgrading the Rust runtime or replacing packaged artifacts, run:

```powershell
inferra --config inferra.toml init-db
```

This validates and bootstraps the active SQLite files in place. On Windows services, follow it with:

```powershell
inferra --config inferra.toml service repair
```

There is no separate native `storage verify|vacuum|backup` command surface yet. Use filesystem-level backups while the runtime is stopped.

## Backup and restore

1. Stop the Inferra process (service or foreground `serve`) so files are not mid-transaction.
2. Copy `events.db`, `incidents.db`, and any auxiliary JSON under `storage.data_dir` you rely on (`noise_registry.json`, `baselines/`, `calibration.json`, scoring weight files, and similar).
3. Restore by placing files back with matching paths and ownership, then run `inferra init-db` on the restored files to ensure schema compatibility with the installed code version.

WAL mode creates `-wal` / `-shm` companions briefly while the process runs; backup while stopped avoids copying transient WAL state inconsistently.

## Configuration upgrades

New releases may extend `inferra.toml`. Prefer `inferra config show` after upgrade to compare with your saved copy, then merge new keys from `src/config/defaults.toml` rather than deleting custom tuning.
