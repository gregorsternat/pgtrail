# Architecture

## Implemented foundation

pgtrail is one Cargo package with a binary entry point and a small internal library.
It uses Rust 2024, a pinned stable toolchain, and a committed lockfile. The declared
minimum Rust version matches the pinned version; older compilers are not promised.
The initial targets are Linux and macOS.

| Responsibility | Location | Boundary |
| --- | --- | --- |
| Runtime entry | `src/main.rs` | Starts the Tokio current-thread runtime |
| Orchestration | `src/lib.rs` | Parses arguments and drives draw/event/update |
| Arguments | `src/cli.rs` | Clap help and version, no connection options yet |
| State | `src/app.rs` | Pure state transitions from messages |
| Input | `src/event.rs` | Crossterm stream and interrupt signal to messages |
| Terminal lifetime | `src/terminal.rs` | Interactive check, setup, restoration |
| Rendering | `src/ui.rs` | Ratatui rendering without I/O or state mutation |

The event loop draws once, waits asynchronously for a meaningful event, updates
state, and redraws. It does not poll a database or redraw on a timer. `futures-util`
provides the stream adapter for Crossterm. Events currently request redraw or quit;
resize causes Ratatui to recalculate the layout. A terminal guard restores normal
mode on success or errors, and Ratatui installs a restoration panic hook.

`anyhow` adds context at the application boundary. Only the application runner is
public. There is no reusable SDK contract, plugin system, or multi-crate workspace.

## Planned data flow, not implemented

The intended flow is PostgreSQL observations -> application state -> TUI. A manual
capture will persist a diagnostic snapshot to a local SQLite file. Comparing two
saved captures will work without a live PostgreSQL connection.

- Use SQLx with Tokio for PostgreSQL reads and SQLite persistence. Add the drivers
  when implementing those capabilities, with minimal features and TLS for remote
  PostgreSQL connections. Do not require a live database merely to compile.
- Keep queries and row mapping in the collector, outside the TUI and domain model.
  Read-only queries need bounded timeouts and a small connection footprint.
- Introduce typed domain errors with `thiserror` where callers need to distinguish
  failures. Add `tracing` with an explicit output destination that does not corrupt
  the terminal; never log DSNs, credentials, or sensitive SQL by default.
- Keep domain observations independent of Ratatui and SQLx types. Add concrete
  modules as features arrive, rather than speculative traits or empty layers.
- SQLite is a local history store, not part of the monitored database. Design its
  versioned schema and migrations in the snapshot slice, not in the bootstrap.

The first supported server target is PostgreSQL 18. Broader compatibility requires
version-specific query checks and an expanded integration matrix before claiming it.

## Diagnostic semantics

Activity comes from `pg_stat_activity`; blockers should use PostgreSQL's
`pg_blocking_pids()` rather than guessing dependencies from matching lock rows.
`pg_stat_statements` provides aggregate execution statistics when available, not
the elapsed duration of a currently running query. Enabling the extension is an
operator task; pgtrail must never configure a monitored server automatically.

Distinguish missing permissions, unavailable extensions, NULL values, no activity,
and measured zero. A failed refresh must not silently present stale data as current.
Each future capture needs collection timing and source/capability information so
comparisons can identify incomplete data, different targets, and statistics resets.
Do not derive execution deltas across a counter reset. Captures contain potentially
sensitive operational data and stay local unless the user explicitly exports them.

The local Compose fixture uses PostgreSQL 18, `pg_stat_statements`, and a dedicated
monitoring role with `pg_read_all_stats`. The role has no application-table grants;
read-only mode and a statement timeout add safeguards but do not replace privileges.
The administrator exists only for fixture provisioning and maintenance.

## References

- [Cargo package layout](https://doc.rust-lang.org/cargo/guide/project-layout.html)
- [Ratatui application patterns](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/)
- [PostgreSQL activity statistics](https://www.postgresql.org/docs/18/monitoring-stats.html)
- [PostgreSQL statement statistics](https://www.postgresql.org/docs/18/pgstatstatements.html)
