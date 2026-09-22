# pgtrail

A Rust terminal application for investigating PostgreSQL activity and comparing
local diagnostic snapshots. Find blocking transactions, separate currently slow
queries from expensive aggregate workloads, capture an incident, and inspect what
changed after a fix.

**Version 1.0:** live read-only PostgreSQL 18 diagnostics, an interactive TUI,
SQLite history, offline comparisons, and Markdown/JSON reports. Linux and macOS are
the supported platforms. One database connection target at a time.

## Try it

The repository pins Rust through rustup. Install your platform's native compiler
and [Rust](https://rustup.rs/), then:

```sh
git clone https://github.com/gregorsternat/pgtrail.git
cd pgtrail
cargo run --locked -- --demo
```

The demo contains synthetic checkout sessions, a blocking transaction, and statement
statistics. Its changing incident state lets you try captures and comparisons
without PostgreSQL. Demo captures and reports are explicitly labeled synthetic.

Install the local binary with `cargo install --path . --locked`, or use
`cargo run --locked --` before the arguments in the examples below.

## Connect to PostgreSQL

Use a PostgreSQL URL with a **non-superuser monitoring account**. pgtrail rejects
superusers, enforces read-only transactions, sets bounded timeouts, and never
creates extensions, changes grants, resets statistics, or cancels sessions.

```sh
# Set this through your preferred secret manager or shell environment.
export PGTRAIL_DATABASE_URL='postgresql://monitor@db.example.com/app?sslmode=verify-full'
pgtrail
```

SQLx supports PostgreSQL URL settings, including `sslmode` and `sslrootcert`.
Use `sslmode=verify-full` for verified remote TLS. The URL can also be passed with
`--database-url`, but environment configuration avoids putting it in process
arguments. `.env` is **not** loaded by the application. SQLx can use PostgreSQL
password environment settings, such as `PGPASSWORD`, with a URL that omits the
password. Connection configuration, including its URL and credentials, is never written to
history or included in collection errors.

For visibility across sessions, the database operator should grant
`pg_read_all_stats` and `CONNECT` on the database to a dedicated account.
`pg_stat_statements` must already be loaded and installed to collect aggregate
query metrics. Missing capabilities appear as unavailable or partial data; activity
monitoring continues when statement statistics are unavailable. PostgreSQL 18 is
the supported server baseline; earlier versions are rejected.

With no connection configured, pgtrail opens in offline mode so you can browse
existing history. `--help`, exports, and all history commands work without a TTY.

## Investigate in the terminal

| Key | Action |
| --- | --- |
| `1`–`5`, `Tab`, `Shift+Tab` | Overview, Activity, Blocking, Statements, History |
| `↑` / `↓`, `j` / `k`, `PgUp` / `PgDn` | Select rows or scroll a report |
| `/` | Filter the current view; `Enter` applies, `Esc` cancels |
| `s` | Cycle the statement ranking: total time, mean time, calls |
| `r` | Refresh now; reload the list in History |
| `p` | Pause or resume automatic collection |
| `c` | Collect and save a fresh manual capture |
| `Enter` in History | Inspect the selected saved capture offline |
| `a`, `b` in History | Mark the earlier and later captures |
| `d` in History | Compare the two marked captures |
| `Esc` | Close a dialog or return to live monitoring |
| `?` | Show keyboard help |
| `q`, `Ctrl+C` | Quit and restore the terminal |

Automatic collection runs every five seconds; change it with `--refresh SECONDS`.
`--timeout SECONDS` bounds collection and connection attempts. Input remains
responsive during collection. Paused, offline, refreshing, failed, and stale
observations are explicitly identified.

Activity shows session identity, transaction age, active query age, and wait events.
Blocking uses PostgreSQL's `pg_blocking_pids()` relationships, including unresolved
blockers that disappeared or are outside the observed database. Statement rankings
show cumulative execution time, calls, and mean execution time in milliseconds.
They describe accumulated workloads, not the duration of a running query.

**SQL text is excluded by default.** Start with `--include-query-text` to include it
in the UI, subsequent snapshots, and exports. Opted-in SQL text can itself contain
passwords, connection URLs, or other sensitive literals. Captures may still contain sensitive
operational metadata (database names, users, application names, and client addresses).

## Capture, compare, and export

```sh
pgtrail check                         # Collect once, print a report, save nothing
pgtrail capture --label 'Before fix'
# Apply your change through your normal operational process.
pgtrail capture --label 'After fix'
pgtrail snapshots
pgtrail show 1
pgtrail compare 1 2
pgtrail compare 1 2 --output incident.md
pgtrail show 1 --format json --output before.json
pgtrail snapshots --json
pgtrail delete 1                       # Explicitly delete a local capture
```

`check`, `show`, and `compare` accept `--format markdown|json`. Connection and
storage errors exit nonzero. A successful collection with unavailable optional
metrics still succeeds and reports its partial coverage in the output. Export files are
created with private permissions and never overwrite existing files. Capture IDs
are printed after saving; use your actual IDs in place of `1` and `2`.

Snapshots record collection start/end times, source identity, availability, and a
versioned payload. A failed save reports the error without stopping live monitoring.
History defaults to:

- macOS: `~/Library/Application Support/pgtrail/history.sqlite3`
- Linux: `$XDG_DATA_HOME/pgtrail/history.sqlite3` or `~/.local/share/pgtrail/history.sqlite3`

Override this with `--store PATH` or `PGTRAIL_STORE`. New history files use `0600`
permissions on Unix; newly created data directories are private. Keep them out of
version control. Deleting a capture removes it from the logical history; it is not
a secure erasure guarantee for filesystem backups or storage media.

Comparisons match sessions by PID **and backend start time**, and statements by
user, database, query ID, and top-level status. They report additions, removals,
changes, and blocking relationships. Statement deltas are withheld for incompatible
sources, missing metrics, counter resets, evictions, truncated rankings, and invalid
capture ordering. Interval mean execution time is derived from interval totals and
calls. If PostgreSQL's system identifier is unavailable to the monitoring role,
source matching uses the endpoint, database OID, and server start time with an
explicit warning. Using different connection aliases conservatively makes sources
incompatible; snapshots are bounded observations, not a transactionally frozen
view of every PostgreSQL subsystem.

## Local PostgreSQL fixture

Install Docker with Compose and start the engine. The fixture is isolated on
`127.0.0.1:55432`, with PostgreSQL 18 and `pg_stat_statements`:

```sh
docker compose up -d --wait
sh scripts/check-db.sh
export PGTRAIL_DATABASE_URL='postgresql://pgtrail_monitor:pgtrail-local-monitor@127.0.0.1:55432/pgtrail_dev?sslmode=disable'
cargo run --locked
```

These are disposable example credentials. `.env.example` lists overrides for
Compose; copy it to `.env` if needed. `pgtrail_admin` provisions the fixture;
`pgtrail_monitor` has statistics access, read-only transactions by default, and no
application-table privileges. The smoke check verifies actual write denials.

`docker compose down` stops the fixture and preserves its data. Initialization only
runs on an empty volume. Changing passwords in `.env` does not update existing
roles. `docker compose down --volumes` deliberately discards this development data.

## Development

| Task | Command |
| --- | --- |
| Run the demo | `cargo run --locked -- --demo` |
| Format | `cargo fmt --all` |
| Lint | `cargo clippy --locked --all-targets -- -D warnings` |
| Unit and CLI tests, no PostgreSQL | `cargo test --locked` |
| Build | `cargo build --locked` |
| All Rust checks | `just check` |
| Fixture privileges | `sh scripts/check-db.sh` |
| Live collector tests, disposable fixture only | `PGTRAIL_LIVE_TEST=1 cargo test --locked --lib -- --ignored --test-threads=1` |

[Architecture](docs/architecture.md) explains data flow and comparison boundaries.
[Roadmap](docs/roadmap.md) tracks delivered scope and intentional exclusions.
[Contributing](CONTRIBUTING.md) covers validation and repository conventions.

Licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
