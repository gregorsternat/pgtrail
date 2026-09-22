# Architecture

pgtrail is one Rust 2024 package, with a thin binary entry point and an internal
library. Rust 1.98.1 is pinned; `Cargo.lock` is committed. Runtime dependencies are
used for implemented capabilities: Tokio, SQLx 0.9 with PostgreSQL/SQLite and
Rustls, Ratatui 0.30/Crossterm, Clap, Chrono, Serde, and contextual/typed errors.
Only the application runner is public. There is no SDK, plugin system, or web server.

## Data flow

```text
PostgreSQL 18 -> asynchronous collector -> serializable observations -> app state
                                                       |                 |
                                               manual capture         pure render
                                                       |                 |
                                                 local SQLite            TUI
                                                       |
                                          two captures -> compare -> report/JSON
```

| Responsibility | Module |
| --- | --- |
| Runtime entry | `main.rs` |
| CLI dispatch, asynchronous jobs, refresh timing | `lib.rs` |
| Arguments and local path resolution | `cli.rs` |
| Driver-independent observations and payload version | `model.rs` |
| PostgreSQL queries, capabilities, safe collection errors | `collector.rs` |
| Private SQLite history and schema migration | `store.rs` |
| Pure deterministic snapshot comparison | `compare.rs` |
| Markdown diagnostic reports | `report.rs` |
| Synthetic demo observations | `demo.rs` |
| Pure input-driven state transitions | `app.rs` |
| Terminal input and interrupt translation | `event.rs` |
| Terminal setup/restoration | `terminal.rs` |
| Ratatui rendering without database/filesystem/network I/O | `ui.rs` |

## Runtime and terminal

Tokio runs a current-thread runtime. Collection and history work run in background
jobs, while input, job completion, and a one-second status tick drive the event loop.
Only one live collection is in flight at a time. The refresh interval skips missed
ticks. Pause stops automatic collection; manual refresh remains available. Offline
inspection stops automatic live refresh. Capturing requests a fresh collection;
it never silently persists an old screen after a failed refresh.

The app retains the last successful observation after a connection failure and
identifies it as stale. SQLite failures appear separately from collector failures.
Quitting aborts pending jobs and drops the terminal guard. The guard restores the
terminal after normal exit and errors; Ratatui also installs its restoration panic
hook. Rendering only reads state, including the clock value supplied by the runner.

## PostgreSQL collection

SQLx uses one pooled connection and parameterized queries with runtime row mapping;
compilation never requires a database. Collection rejects a superuser and pre-18
servers. Startup settings enforce read-only transactions, statement timeouts, and
a recognizable application name. Overall timeouts bound connection/collection work.
Raw driver errors are converted into safe categories, without URL, server error
text, credentials, or SQL. No logging subscriber writes into the terminal.

Activity is scoped to the connected database and excludes the collector backend.
`pg_blocking_pids()` provides blocker relationships directly from PostgreSQL;
a missing blocker remains unresolved rather than being guessed from a PID.
Aggregate statement statistics are collected separately, only when available.
Collection records counter-reset metadata and individual `stats_since` values.
Statement collection is bounded; truncation is explicit and prevents misleading
set or counter comparisons. No monitored-server provisioning is performed.

SQL text is opt-in. NULL values remain optional. Restricted visibility, missing
extensions, denied access, and failed optional collection remain distinct from
empty results and numeric zero. Collection timing describes the observation window;
PostgreSQL activity and shared statistics can change during that window.

## History and comparison

SQLite stores versioned JSON observations plus searchable capture metadata. The
history schema uses a versioned, transactional migration and rejects future schema
or snapshot versions. Saves are atomic. SQLite I/O stays outside rendering. New
files and directories are private on Unix; existing user paths are not broadly
re-permissioned. Captures contain no connection configuration.

Source comparison checks endpoint, database identity, and server start time, plus
system identifiers when available. A restart is conservatively incompatible. If
system identifiers cannot be read, matching endpoint/database/OID/start time is an
explicitly weaker identity check, with a warning. Different aliases cannot be
silently equated. Missing session start times cannot establish identity, and reused
PIDs never represent a continuing session.

Statement identity includes database, user, query ID, and top-level status. Numeric
deltas require compatible sources and capture windows, available complete metrics,
consistent global and per-statement reset metadata, no eviction changes, and
nondecreasing counters. A missing, reset, or incompatible metric is not a zero delta.
Mean time over an interval comes from delta execution time divided by delta calls;
subtracting cumulative averages would not describe interval performance.

Reports preserve unavailable states and comparison warnings. Markdown text is
escaped and control characters are removed for safe terminal/report display. JSON
uses the versioned observation model. Exports are explicit, private, and do not
overwrite existing files.

## References

- [SQLx 0.9 documentation](https://docs.rs/sqlx/0.9.0/sqlx/)
- [Ratatui application patterns](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/)
- [PostgreSQL 18 activity statistics](https://www.postgresql.org/docs/18/monitoring-stats.html)
- [PostgreSQL 18 statement statistics](https://www.postgresql.org/docs/18/pgstatstatements.html)
