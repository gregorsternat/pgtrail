# Roadmap

Version 1.0 implements the five original slices for one PostgreSQL 18 connection
target, local operation, and read-only diagnostics.

## Delivered in v1

- **Active sessions:** asynchronous SQLx collection; session identity, state,
  active query age, transaction age, and waits; bounded refreshes and explicit
  failures, restricted visibility, NULL fields, and stale observations.
- **Blocking transactions:** server-provided waiter/blocker relationships;
  transaction age and unresolved/disappearing blockers; no cancellation action.
- **Expensive queries:** sortable aggregate statement rankings by total execution
  time, mean execution time, and calls, with explicit units and capability failures.
  Current query duration remains separate from cumulative workload metrics.
- **Manual local snapshots:** SQLite persistence across restarts, source identity,
  collection timing and availability, transactional schema versioning, private
  storage, and visible save failures that do not stop collection.
- **Offline comparisons:** session/relationship additions and removals, changed
  observations, and valid statement deltas, with PID reuse, source compatibility,
  counter resets, missing values, eviction, truncation, and capture ordering handled.

The verification suite combines database-free behavior/CLI tests with explicitly
invoked tests against the disposable PostgreSQL 18 fixture. CI runs both suites.
See CONTRIBUTING for commands and terminal lifecycle verification.

## Additional v1 workflows

- An explicitly synthetic demo for exploring the complete UI without PostgreSQL.
- Overview, Activity, Blocking, Statements, and History views with filtering,
  selection/details, keyboard help, pause/resume, and offline inspection.
- Headless capture/check/history/show/compare/delete commands.
- Explicit Markdown and JSON exports for incident reports and further analysis.
- SQL text excluded by default, with an explicit opt-in for investigations that
  need it; connection configuration is never stored. Opted-in SQL may itself
  contain sensitive literals.

## Deliberate exclusions

Continuous recording, remote storage, a web server, multi-target monitoring,
automatic remediation, query execution, and automatic extension provisioning remain
outside v1. Broader PostgreSQL compatibility needs a versioned collector and live
integration matrix before it can be claimed. Prebuilt release binaries and registry
publication can be added when a distribution workflow is chosen.
