# Architecture

pgtrail is one Rust 2024 package with a thin binary entry point and an internal
library. Rust 1.98.1 is pinned; `Cargo.lock` is committed. Runtime dependencies serve
implemented capabilities: Tokio, SQLx 0.9 with PostgreSQL/SQLite and Rustls,
Ratatui 0.30/Crossterm, Clap, Chrono, Serde, and contextual/typed errors. Only the
application runner is public. There is no SDK, plugin system, or web server.

## Data flow

```text
connection profile / environment
              |
PostgreSQL 16–18 -> async collector -> versioned observations
                                           |
                        +------------------+------------------+
                        |                  |                  |
                  metrics/findings     live app state    SQLite captures
                        |                  |                  |
                        +------------> pure rendering    incident + notes
                                                              |
                                           offline comparison + reports
```

| Responsibility | Module |
| --- | --- |
| Binary entry | `main.rs` |
| Application dispatch and connection resolution | `lib.rs` |
| Headless workflows and explicit exports | `commands.rs` |
| Asynchronous TUI jobs, freshness, refresh timing | `runtime.rs` |
| Arguments and local path resolution | `cli.rs` |
| Private connection metadata and environment password resolution | `profiles.rs` |
| Driver-independent observations and payload version | `model.rs` |
| PostgreSQL activity, statement capabilities, safe errors | `collector.rs` |
| Database, relation, replication, WAL, I/O, vacuum collection | `collector/health.rs` |
| Private SQLite captures, incidents, notes, migrations | `store.rs` |
| Pure snapshot comparisons and identity checks | `compare.rs` |
| Guarded interval counters, rates, and ratios | `metrics.rs` |
| Evidence-backed findings and coverage limits | `diagnostics.rs` |
| Markdown observation, analysis, and comparison reports | `report.rs` |
| Incident chronology and evidence exports | `incidents.rs` |
| Explicitly synthetic observations | `demo.rs` |
| Input-driven state transitions and bounded trend state | `app.rs` |
| Terminal events and restoration | `event.rs`, `terminal.rs` |
| Rendering without database, filesystem, or network I/O | `ui.rs`, `ui/investigation.rs` |

## Runtime and terminal

Tokio runs a current-thread runtime. Collection and history jobs run asynchronously;
input, job completion, and a one-second status tick drive the event loop. Only one
live collection is in flight at a time. Missed refresh ticks are skipped. Pause
stops automatic collection; manual refresh remains available. Offline inspection
stops automatic refresh. Capture requests a fresh collection rather than saving
an old screen after a failed refresh. Its incident target is fixed at the keypress,
so changing the active incident during collection cannot redirect the capture.

Generation numbers protect asynchronous reads: a late capture, comparison, or
incident result cannot replace a newer user selection. List and incident-detail
refreshes have their own request ordering. Collection and local write failures are
reported separately. A saved capture remains reported as saved even if a subsequent
list refresh fails.

The app retains the last successful live observation after connection failure and
marks it stale. Two live snapshots support interval calculations; up to 120 compact
trend points retain optional metrics and timestamps. Failed collection adds a gap.
Offline inspection never adds historical observations to the live trend; incompatible
sources clear it. Trends disappear on exit. `record` instead saves bounded,
sequential observations in the foreground, preserving committed captures on
interruption. Its interval is a delay after each successful save, not a fixed-rate
scheduler.

Quitting aborts pending jobs and drops the terminal guard. The guard restores the
terminal after normal exit and errors; Ratatui also installs its restoration panic
hook. Rendering only reads state, including the clock supplied by the runner.

## PostgreSQL collection

SQLx uses a one-connection pool and parameterized queries with runtime row mapping;
compilation never requires PostgreSQL. Collection rejects superusers and server
majors outside 16–18. Startup settings enforce read-only transactions, statement
timeouts, and a recognizable application name. Overall timeouts bound connection
and collection. Raw driver errors become safe categories without server-provided
error text, URLs, credentials, or SQL. Connection URL parameters are validated
before reaching the driver. No logging subscriber writes into the terminal.

Activity is scoped to the connected database and excludes the collector backend.
`pg_blocking_pids()` supplies relationships directly; missing blockers stay
unresolved. Aggregate statements are separate from current activity and record
global resets, evictions, and per-entry `stats_since` when supported. The collector
checks the installed extension's actual column availability: an older extension
SQL definition can lack a newer field even on a supported server. PostgreSQL 16
has no per-entry `stats_since`, so only its statement interval metrics are withheld.
Statement rankings are bounded, with explicit truncation.

Six health sections use separate savepoints: database statistics, tables/indexes,
replication, WAL, I/O, and vacuum progress. Failure of an optional section rolls back
its savepoint and preserves the remaining observation. Replication and vacuum
progress require full statistics visibility; restricted access is unavailable,
not a falsely empty result.

| Observation | Scope and interpretation |
| --- | --- |
| Activity and statement ranking | Connected database; currently active duration differs from completed cumulative execution |
| Database counters | Connected database; reset metadata belongs to that statistics entry |
| Connection pressure | Cluster client backends against the approximate non-reserved connection budget |
| Tables/indexes | Connected database; tuple estimates, cumulative scans, maintenance timestamps, and catalog validity |
| Vacuum progress | Connected database; current reported progress, not a history of completed maintenance |
| WAL and I/O | Cluster counters; I/O grouped by backend type, object, and context |
| Replication | Cluster senders/slots and current primary/standby positions |

Relation collection selects 1,001 candidate tables and indexes using catalog page
estimates, computes sizes for that bounded candidate set, then retains at most
1,000 of each. Materialized query stages keep the limit ahead of relation-size
calls. The extra candidate detects truncation. Catalog estimates can be stale, so
this is not a guaranteed largest-N listing. Catalog processing and
`pg_database_size` still have cost; timeout bounds do not make monitoring free.

SQL text is opt-in. NULL fields remain optional; unavailable sections retain a
reason. Missing extensions, denied access, disabled timing, and partial visibility
are distinct from empty results and zero. I/O timing follows `track_io_timing`, or
`track_wal_io_timing` for WAL I/O objects. Time since the last replayed transaction
is not treated as a backlog estimate. Collection start/end timestamps describe a
window: PostgreSQL statistics can lag activity, and subsystems do not provide one
atomic cross-view snapshot.

## Metrics and findings

Interval calculations require chronological, non-overlapping observations from a
compatible source. Source comparison checks endpoint, database/OID, and server
start time, plus system identifiers when available. Restarts are incompatible. If
system identifiers are unavailable, endpoint/database/OID/start time is explicitly
a weaker identity check. Different connection aliases are not silently equated.

Database and WAL baselines are independent. A changed reset timestamp prevents
rates for that section; unavailable sections never imply zeros. PostgreSQL's
explicit SQL NULL `pg_stat_database.stats_reset` represents the initial unreset
epoch. Two collected NULL values on the same server instance are accepted with an
explanation and nondecreasing-counter guards; this differs from absent collection
metadata. WAL reset metadata must be known. Disabled `track_counts` prevents normal
database counter interpretation, while disabled I/O timing withholds timing rates.
Individual counter regressions invalidate their own derived metrics.

Statement identity contains database, user, query ID, and top-level status. Deltas
require complete compatible observations, matching global/per-statement reset
metadata, unchanged eviction counts, and nondecreasing counters. A missing entry,
reset, truncated ranking, or incompatible baseline cannot become a zero delta.
Rates divide counter deltas by elapsed time between completed observations. Interval
mean execution time is delta execution time divided by delta calls; zero calls
produce no mean. Subtracting cumulative means would not measure interval latency.
Ratios also require a nonzero denominator.

The pure diagnostic engine emits a stable finding ID, severity, title, observed
evidence, interpretation, suggested checks, and related PIDs. It covers blocking
chains/cycles, old queries/transactions, connection pressure, estimated maintenance
pressure, transaction ID age, invalid indexes, replication retention/backlog, and
valid interval deadlocks, rollbacks, temporary writes, and workload changes. Thresholds
and caveats appear in the finding itself. The engine does not claim causal diagnosis,
predict remaining capacity, execute remedial SQL, or infer health from an empty
finding list. Coverage accompanies every analysis, including incomplete baselines.

## Local persistence and profiles

SQLite stores versioned JSON observations plus searchable capture metadata,
incident memberships, and timestamped notes. Store schema 2 migrates schema 1
transactionally and retains v1 captures. Snapshot payload v2 adds health sections;
loading v1 defaults these to unavailable with a not-collected reason. Future store
or snapshot versions are rejected. A v1.0 binary cannot open the migrated store.

Saves and incident attachment are one transaction. Invalid, closed, or incompatible
incident targets roll back the new capture rather than leaving an orphan. Incident
identity permits server restarts for one endpoint/database/OID, but rejects known
system-identifier mismatches. Comparisons inside that incident still enforce their
stricter interval identity. Closed incidents must be reopened before notes or
membership changes. Capture annotations and timestamped incident notes are distinct.
Deleting a capture removes its membership links, not its incident or incident notes.

Profiles store structured connection metadata and optional environment variable
names, never password values or URLs. Passwords are resolved only for connection
creation. The versioned JSON rejects unknown fields, has bounded size/entry counts,
and is replaced atomically through a private temporary file. Profile paths reject
symbolic links and parent traversal; existing profile files must be private. See
the README for path precedence and commands.

SQLite and profile I/O stay outside rendering. New files/directories are private
on Unix; existing user directories are not broadly re-permissioned. Captures omit
connection configuration but can contain sensitive metadata, and opted-in SQL can
contain secrets. Local deletion does not guarantee secure filesystem erasure.

## Reports

Observation, analysis, comparison, and incident reports preserve unavailable states,
source identity, timestamps, and coverage. Incident reports order captures and
operator notes chronologically, retain annotations, include evidence, and compare
the first and last observations with the normal guards. Notes are operator input,
not a trusted diagnosis.

Markdown content is escaped and terminal control characters are removed for safe
display. JSON follows versioned observation structures; `show` adds capture metadata
without moving existing snapshot fields. Explicit exports use private files and
refuse overwrites. No automatic remote upload is performed.

## References

- [SQLx 0.9 documentation](https://docs.rs/sqlx/0.9.0/sqlx/)
- [Ratatui application patterns](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/)
- [PostgreSQL 18 activity statistics](https://www.postgresql.org/docs/18/monitoring-stats.html)
- [PostgreSQL 18 statement statistics](https://www.postgresql.org/docs/18/pgstatstatements.html)
- [PostgreSQL 16 statement statistics](https://www.postgresql.org/docs/16/pgstatstatements.html)
