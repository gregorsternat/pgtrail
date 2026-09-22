# Roadmap

Version 1.1 extends the original five v1 slices into a daily investigation workflow
for one PostgreSQL 16–18 target, with read-only collection and local evidence.
The README documents shipped behavior and commands; this file records scope and
acceptance boundaries.

## Delivered in v1.0

- Active sessions with identity, transaction/query ages, waits, restricted-visibility
  warnings, explicit failures, and stale-observation handling.
- Server-provided blocking relationships and unresolved blockers, without session
  cancellation.
- Cumulative statement rankings, distinct from running-query duration.
- Private SQLite captures that survive restarts, with source identity and timing.
- Offline session, relationship, and guarded statement comparisons; explicit
  Markdown/JSON exports and a synthetic demo.

## Delivered in v1.1

| Capability | Acceptance boundary |
| --- | --- |
| PostgreSQL 16–18 collection | Supported majors are checked; optional metrics fail independently; old extension definitions are detected; unsupported majors are rejected |
| Six health sections | Database, table/index, replication, WAL, I/O, and vacuum data retain scope, NULL, unavailable, and truncation distinctions |
| Explained findings | Evidence, severity, thresholds, interpretation, next checks, and coverage; no automatic remediation or root-cause guarantee |
| Interval workload | Database/WAL/statement rates require compatible sources, ordering, continuity, and valid denominators; PostgreSQL 16 statement deltas remain unavailable without per-entry reset metadata |
| Ten terminal views | Findings, activity, blockers, statements, history, database, relations, replication, I/O, and incidents; details, filtering, ranking, help, pause, and explicit offline context |
| Recent trends | Up to 120 in-memory observations; missing data remains a gap and incompatible sources are never joined |
| Incident dossiers | Create/select, timestamped notes, capture membership, close/reopen, annotations, chronology, evidence export, and first-to-last comparison |
| Atomic incident captures | Save and attachment commit together; missing, closed, or incompatible incident targets cannot leave orphan captures |
| Bounded foreground recording | Configurable count/cadence, visible saves, interruption preserving commits, and failures reported without claiming a complete recording |
| Headless diagnosis | Two-sample explained reports with optional severity-based nonzero exit status |
| Named profiles | Private atomic metadata storage, verified TLS default, password environment references, and no persisted password or connection URL |
| Existing history | Automatic store migration; v1 payloads remain readable with new sections explicitly not collected |
| Validation infrastructure | Database-free behavior/CLI tests, an actual PTY lifecycle check, and a PostgreSQL 16/17/18 integration matrix configured in CI |

Compatibility tests have been exercised locally against PostgreSQL 16.14, 17.11,
and 18.4. Local results do not establish a successful remote CI run. Use the
validation commands in CONTRIBUTING to verify the current checkout.

## Future work and acceptance criteria

These capabilities are not shipped in v1.1:

- **Durable unattended recording:** an explicit retention policy, bounded disk use,
  restart recovery, and a visible record of missed or failed observations before
  adding a background service.
- **Multi-target investigation:** independent source identity, freshness, errors,
  and permissions for each target before adding a combined view.
- **Configurable diagnostic policy:** versioned thresholds and documented overrides
  whose exports record the policy used; retain observed evidence and uncertainty.
- **Broader or newer PostgreSQL support:** capability-aware collection plus a live
  integration matrix for each newly claimed major.
- **Distribution:** choose and verify a release-binary/signing and registry workflow
  before claiming prebuilt packages or public releases.

A web server, remote evidence storage, query execution, automatic extension
provisioning, and automatic remediation are outside the current product scope.
