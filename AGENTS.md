# Working on pgtrail

## Start here

- pgtrail is a Rust TUI for PostgreSQL investigation and local snapshot comparison.
- Read the README for implemented behavior; do not describe roadmap items as shipped.
- The current binary is an interactive skeleton. It does not connect to PostgreSQL.
- Keep code, documentation, user-facing text, commits, and PR descriptions in English.
- Inspect the relevant code and existing changes before editing.
- Keep each change focused on one coherent outcome.

## Read only what the task needs

- Setup and commands: `README.md`.
- Boundaries, data flow, and technology decisions: `docs/architecture.md`.
- Feature scope and acceptance criteria: `docs/roadmap.md`.
- Contribution workflow and manual checks: `CONTRIBUTING.md`.
- PostgreSQL fixture and permissions: `dev/postgres/init.sh` and `scripts/check-db.sh`.
- Use the documentation for the dependency versions in `Cargo.lock`.

## Commands

- Run: `cargo run --locked` (requires an interactive terminal).
- CLI help: `cargo run --locked -- --help`.
- Format: `cargo fmt --all`.
- Check: `just check`, or the following equivalent commands:
  - `cargo fmt --all -- --check`
  - `cargo clippy --locked --all-targets -- -D warnings`
  - `cargo test --locked`
  - `cargo build --locked`
- Start the local database: `docker compose up -d --wait`.
- Validate database setup: `sh scripts/check-db.sh`.
- Stop it without deleting data: `docker compose down`.
- Rust checks must work without PostgreSQL running.

## Implementation boundaries

- Keep one package until a concrete need justifies extracting a crate.
- Keep `main` thin; orchestration belongs in the library.
- Translate terminal input into messages; update state separately from rendering.
- Rendering must not perform database, filesystem, or network I/O.
- Future PostgreSQL collection and SQLite persistence are separate responsibilities.
- Keep database I/O asynchronous and outside the rendering path.
- Create modules when functionality needs them, not as empty placeholders.
- Do not introduce a public API or trait solely for hypothetical reuse.

## Rust conventions

- Use the pinned toolchain, edition 2024, standard rustfmt, and Clippy.
- `unsafe` is forbidden in project code.
- Return errors with context; do not panic on recoverable input or I/O failures.
- Prefer concrete types, enums, private fields, and narrow visibility.
- Use `anyhow` at the application boundary; introduce typed domain errors as needed.
- Do not add broad lint suppressions; justify narrow exceptions inline.
- Add dependencies only for code that uses them, and version the lockfile changes.
- Test observable behavior and failure cases rather than mirroring implementation.

## PostgreSQL and data invariants

- Observe the target database in read-only mode using a non-superuser account.
- Never create extensions, reset statistics, or cancel sessions on a monitored server.
- Provisioning SQL belongs only in the disposable local development fixture.
- Unavailable data is distinct from zero, NULL, an empty result, or a healthy state.
- Current query duration and cumulative statement statistics are different metrics.
- Snapshot comparisons must account for counter resets and incompatible sources.
- Do not log credentials, connection strings, or sensitive SQL text by default.
- Keep captured data and local settings out of version control.

## Finish a change

- Run the relevant checks and report exactly what passed and what was not verified.
- For terminal lifecycle changes, check quit, Ctrl+C, resize, and terminal restoration.
- For database setup changes, run the Compose smoke check.
- Update the single relevant guide when behavior, commands, or a decision changes.
- Use short scoped Conventional Commits, for example `feat(activity): show sessions`.
- Do not add agent transcripts, generic Rust tutorials, or duplicate instruction files.
