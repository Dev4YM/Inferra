# ADR 0003: Storage Protocols and Split Stores

## Status

Accepted

## Context

Inferra originally had a storage monolith in `src/storage/sqlite.py` that combined event persistence, incident
persistence, and compatibility helpers. Newer storage concerns had already moved into focused modules for events,
incidents, baselines, calibration, service graph caching, metric buffers, migrations, and scoring weights.

Keeping both shapes made import ownership ambiguous and made future storage swaps harder to reason about.

## Decision

`src/storage/__init__.py` is the canonical storage surface. Callers import protocol contracts and concrete stores from
`storage`, not from implementation submodules.

The public protocols are:

- `EventStore`
- `IncidentStore`
- `BaselineStore`
- `ServiceGraphStore`
- `WeightStore`
- `CalibrationStore`

The concrete local-first implementations are:

- `SqliteEventStore`
- `SqliteIncidentStore`
- `JsonBaselineStore`
- `JsonServiceGraphStore`
- `JsonWeightStore`
- `JsonCalibrationStore`

`initialize_storage(data_dir: Path)` builds the full local storage stack from one configured data directory. The legacy
`src/storage/sqlite.py` monolith is deleted.

## Consequences

SQLite remains the required event and incident database, with migrations in `src/storage/migrations.py`. JSON-backed
stores remain local files under the configured data directory for baselines, graph cache, weights, and calibration.

Future PostgreSQL or alternative file-backed stores can implement the same protocols without changing analysis, runtime,
or API imports. Compatibility behavior required by the current runtime lives in the split stores instead of resurrecting
the monolith.
