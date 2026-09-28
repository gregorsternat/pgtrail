# Working on pgtrail

<!-- owner: maintainers; reviewed: 2026-09-28 -->

pgtrail is a Rust TUI for read-only PostgreSQL investigation and local snapshot
comparison. Keep code, documentation, UI text, commits, and PRs in English.
Inspect the relevant code and existing changes before editing. Keep each change
focused on one coherent outcome.

## Find the source of truth

Start with the [knowledge index](docs/index.md), then read only what the task needs:

- Implemented behavior and commands: [README](README.md). Roadmap items are not shipped.
- Boundaries, data flow, and decisions: [architecture](docs/architecture.md).
- Future scope and acceptance criteria: [roadmap](docs/roadmap.md).
- Rust conventions, checks, and review: [Contributing](CONTRIBUTING.md).
- Complex work and decision logs: [execution plans](docs/plans.md).
- Coverage and known gaps: [quality](docs/quality.md) and [debt](docs/exec-plans/tech-debt-tracker.md).
- Disposable database setup: [fixture](dev/postgres/init.sh) and [privilege check](scripts/check-db.sh).
- Dependency versions: [Cargo.lock](Cargo.lock); use their matching documentation.

## Non-negotiable boundaries

- Keep one package and a thin main; orchestration belongs in the library.
- Input becomes messages; state updates and rendering stay separate.
- Rendering performs no database, filesystem, or network I/O. Database I/O is async.
- PostgreSQL collection and SQLite persistence are separate responsibilities.
- Observe PostgreSQL 16–18 with a non-superuser, read-only account. Never create
  extensions, reset statistics, or cancel sessions on a monitored server.
- Provisioning SQL belongs only in the disposable development fixture.
- Unavailable data differs from zero, NULL, empty results, and a healthy state.
- Current query duration differs from cumulative statement statistics.
- Comparisons must handle counter resets and incompatible sources.
- Do not log credentials, connection strings, or sensitive SQL by default.
- Keep captured data and local settings out of version control. Demo data is synthetic.
- Use the pinned Rust toolchain, rustfmt, and Clippy. `unsafe` is forbidden.
- Return contextual errors for recoverable failures; use narrow visibility and
  concrete types. Add dependencies or abstractions only for a present need.
- Test observable behavior and failure cases; do not mirror the implementation.
- Do not weaken a guardrail to silence a failure. Fix the boundary or document and
  review a concrete exception with its test and debt entry.

## Work and verify

- Use a versioned plan for complex work; update decisions and evidence as it progresses.
- Run `just check` for documentation, format, Clippy, tests, build, and synthetic PTY
  checks without PostgreSQL. Equivalent commands are in Contributing.
- Run `just docs-check` for focused documentation work; `just architecture-check`
  checks module boundaries. Both are enforced in CI.
- For terminal changes, also inspect the demo visually and check quit, Ctrl+C,
  resize, and terminal restoration.
- For collector/fixture changes, run the disposable database checks in Contributing.
- Update the owning guide when behavior or decisions change. Keep the knowledge
  index, quality gaps, and plan links current; record repeated failures as tests.
- Review the final diff and report exactly what passed and what remains unverified.
- Use short scoped Conventional Commits, e.g. `feat(activity): show sessions`.
- Do not add agent transcripts, generic tutorials, or duplicate instruction files.
