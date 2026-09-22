# Roadmap

Only the executable foundation is implemented. Deliver the following slices in
order, each with its own behavior tests and a README status update. Scope is one
PostgreSQL connection at a time, local operation, and read-only diagnostics.

## 1. Active sessions

Connect using SQLx and show session identity, state, query age, and wait information.
Define connection configuration in this slice; do not expose a fake option earlier.

Acceptance: a restricted diagnostic role can list visible sessions; refresh does not
freeze keyboard input; connection failure, insufficient privileges, NULL fields,
and stale observations are explicit. Credentials do not appear in errors or logs.
Unit tests run without PostgreSQL, with separate live integration tests against 18.

## 2. Blocking transactions

Show waiting sessions, blocker relationships, and transaction age using server data.

Acceptance: a controlled two-session lock scenario identifies the real blocker and
waiter; commit/rollback clears the block after refresh; disappearing sessions are
handled. There is no cancel/terminate-session action.

## 3. Expensive queries

Show a sortable view of aggregate `pg_stat_statements` metrics with explicit units,
including total execution time, calls, and mean execution time. Keep currently long
queries identifiable as activity, separate from this aggregate ranking.

Acceptance: known workloads appear with the expected metrics; missing extension or
permissions produces an unavailable state; statistics are never reset by pgtrail.

## 4. Manual local snapshots

Save an explicitly requested capture to SQLite and list captures after restart.
Add source identity, collection timing, metric availability, a versioned schema,
and migration tests in this slice. Do not persist credentials.

Acceptance: captures survive restart; incomplete collection is identified; unwritable
storage fails visibly without losing live monitoring; fixture-based persistence
tests use isolated temporary files.

## 5. Compare two captures

Select two saved captures and compare sessions, blocking relationships, and statement
metrics offline, with additions, removals, and valid metric changes.

Acceptance: deterministic fixtures cover changes and identical captures; comparison
does not confuse reused session identifiers, incompatible targets, missing metrics,
or counters reset between captures with ordinary numeric deltas.

## Outside the first version

Continuous recording, remote storage, a web server, automatic remediation, and
multi-target monitoring are not included. Export formats and release automation
can follow demonstrated demand.
