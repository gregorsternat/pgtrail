# pgtrail

A Rust terminal application for daily PostgreSQL investigation. Follow blocking
transactions, identify expensive workloads, inspect maintenance and replication,
and keep an annotated local record of an incident before and after a change.

**Version 1.1** connects read-only to **PostgreSQL 16, 17, and 18**. It combines ten
interactive views, explained diagnostic findings, interval metrics and trends,
SQLite capture history, incident dossiers, and Markdown/JSON exports. Linux and
macOS are supported. One database connection target is observed at a time.

## Try it

Install your platform's native compiler and [Rust](https://rustup.rs/). The
repository pins its toolchain through rustup:

```sh
git clone https://github.com/gregorsternat/pgtrail.git
cd pgtrail
cargo run --locked -- --demo
```

The demo includes changing checkout sessions, blockers, statement workloads,
maintenance estimates, and replication observations. All demo data, captures, and
reports are explicitly synthetic; PostgreSQL is not needed.

Install the binary with `cargo install --path . --locked`, or prefix the arguments
below with `cargo run --locked --` instead of `pgtrail`.

## Connect to PostgreSQL

Use a **non-superuser monitoring account**. pgtrail rejects superusers, enforces
read-only transactions, and bounds connection and collection time. It never creates
extensions, changes grants, resets statistics, executes application queries, or
cancels sessions on the monitored server.

```sh
# Configure this through your secret manager or shell environment.
export PGTRAIL_DATABASE_URL='postgresql://monitor@db.example.com/app?sslmode=verify-full'
pgtrail
```

Use `sslmode=verify-full` for verified remote TLS and `sslrootcert` when a custom CA
is needed. `--database-url` also accepts a URL; environment configuration avoids
putting credentials in process arguments. With a password-free URL, SQLx can use
PostgreSQL password environment settings such as `PGPASSWORD`. The application
**does not load `.env`**. Connection configuration is never saved in capture history
or included in collection errors.

For cross-session visibility, the database operator should grant `CONNECT` and
`pg_read_all_stats` to the dedicated monitoring account. `pg_stat_statements` must
already be loaded and installed for aggregate query statistics. Missing extensions,
restricted visibility, and inaccessible metrics are reported as unavailable while
other sections continue. Supported server majors are 16–18; other majors are
rejected. PostgreSQL 16 lacks the per-statement `stats_since` baseline, so its
cumulative statement rankings work, but statement interval deltas are unavailable.

With no connection configured, pgtrail opens offline to browse existing captures
and incidents. Headless commands and `--help` do not require a terminal.

### Named connection profiles

Save connection metadata once and resolve the password from an environment variable
only when connecting:

```sh
pgtrail profile add production --host db.example.com --database app --user monitor \
  --password-env APP_MONITOR_PASSWORD
# Populate APP_MONITOR_PASSWORD through your normal secret-management process.
pgtrail --profile production
pgtrail --profile production diagnose
pgtrail profile list
pgtrail profile remove production
```

Profiles default to `sslmode=verify-full`; `--sslrootcert PATH`, `--sslmode`, and
`--port` are available when adding a profile. `--host` accepts a hostname, IP
address, or absolute Unix socket directory. A profile stores metadata and the
password variable's **name**, never its value or a connection URL. If
`--password-env` is omitted, the driver's normal password environment applies.
Missing configured password variables fail clearly. An explicit `--profile` takes
precedence over `PGTRAIL_DATABASE_URL` and cannot be combined with `--database-url`
or `--demo`.

The private JSON file defaults to `$XDG_CONFIG_HOME/pgtrail/profiles.json` when that
base is absolute, otherwise `~/.config/pgtrail/profiles.json`, on both platforms.
Override it with `--profiles-file PATH` or `PGTRAIL_PROFILES`. Updates are atomic;
Unix files must be private (`0600`). Symbolic links and `..` in profile paths are
rejected. Profile names must contain 1–64 ASCII letters, digits, `.`, `_`, or `-`.

## Investigate in the terminal

| View | What to investigate |
| --- | --- |
| `1` Overview | Findings ordered by severity, evidence, interpretation, next checks, and coverage limits |
| `2` Activity | Session identity, transaction age, active query age, and wait events |
| `3` Blocking | Waiter/blocker relationships, transaction ages, and unresolved blockers |
| `4` Statements | Cumulative workload rankings or valid interval calls, execution time, and reads |
| `5` History | Saved observations, capture labels, offline inspection, and before/after comparison |
| `6` Database | Connection pressure, transaction rates, cache activity, temporary writes, WAL, and recent trends |
| `7` Relations | Table/index sizes and usage, maintenance estimates, transaction ID age, and vacuum progress |
| `8` Replication | Primary/standby state, sender backlog, and replication slot WAL retention |
| `9` I/O | Cluster I/O counters by backend type, object, and context, with timing availability |
| `0` Incidents | Local investigation dossiers, timestamped notes, and attached captures |

| Key | Action |
| --- | --- |
| `1`–`9`, `0`, `Tab`, `Shift+Tab` | Switch views |
| `↑` / `↓`, `j` / `k`, `PgUp` / `PgDn` | Select rows or scroll a report |
| `/` | Edit this view's filter; `Enter` applies, `Esc` cancels. Report-only views and incident chronology have no filter |
| `Enter` | Expand the selected finding, session, blocking relationship, statement or relation into scrollable details |
| `h` | Read provenance, collection coverage, unavailable reasons and interval readiness |
| `e` in Overview | Follow a finding to a reliably identified backend, relation or statement |
| `b` / `w` in Activity or Blocking | Follow blocker / waiter relationships in the same observation |
| `Backspace` | Return to the originating evidence and selection |
| `[` / `]` in details or comparison | Jump to the previous / next section |
| `Home` / `End` | Reach the first / last row of a list, report or help |
| `v` in Statements | Switch cumulative/interval metrics |
| `s` in Statements | Rank by execution time, mean time, or calls |
| `v` in Relations | Switch tables/indexes |
| `s` in Relations | Cycle size, maintenance/validity, and scan rankings |
| `r` | Refresh now; reload lists in History or Incidents |
| `p` | Pause or resume automatic collection |
| `c` | Collect and save a fresh capture, attached to the active incident when selected |
| `Enter` in History | Inspect the selected saved capture offline |
| `a`, `b`, then `d` in History | Mark earlier/later captures and compare them |
| `l` in History | Rename the selected capture |
| `I` in History | Attach the selected capture to the active incident |
| `i` | Create an incident |
| `n` | Add a note to the active incident |
| `Enter` in Incidents | Open the complete chronology and activate an open incident; in the chronology, inspect a capture or the full note |
| `t` in Incidents | Switch between incident list and chronology |
| `e` / `E` in Incidents | Export Markdown / JSON to a destination entered in the prompt; existing files are never overwritten |
| `o` in Incidents | Close or reopen the selected incident |
| `x` in Incidents | Clear the active capture target |
| `Esc` | Close details, return from attached evidence, leave chronology, clear the view filter, or return live |
| `?` | Show scrollable keyboard help; use arrows, page keys or Home/End |
| `q`, `Ctrl+C` | Quit and restore the terminal |

Selections follow observed identities across refreshes: PID **and backend start** for
sessions, the complete statement identity, relation OID, or finding ID. If an item
disappears or its backend identity cannot be verified, a notice explains the change
and the first visible row is selected. Filters are saved separately for each view;
Statements matches explicitly captured SQL in both cumulative and interval modes.

Details scroll through wrapped lines, including the end of long captured SQL.
Contextual evidence navigation freezes the observation so following a blocker or
waiter cannot silently switch to a newer sample. `Backspace` restores the originating
view and selection; ordinary view switching leaves that navigation path. The header
keeps observation age visible at 80×24 and identifies an open capture by ID and label.
Coverage distinguishes synthetic provenance, collection completeness, and interval
readiness; complete collection does not establish database health.

Automatic collection runs every five seconds; change it with `--refresh SECONDS`.
`--timeout SECONDS` bounds each collection/connection attempt. Paused, offline,
refreshing, failed, and stale observations are identified. Input remains responsive
during collection. Trends retain up to 120 observations in memory, show missing
samples as gaps, and reset when the source changes. They are not durable time series;
save captures or use `record` when the evidence must survive an exit.

**SQL text is excluded by default.** `--include-query-text` includes it in the UI,
subsequent captures, and exports. SQL can contain passwords or sensitive literals.
Even without SQL text, captures contain operational metadata such as database,
user, table, application, and client names. Keep the local files and exports private.

## A daily investigation workflow

Start with an explained two-sample report, then preserve the relevant observations:

```sh
pgtrail --profile production diagnose --sample-seconds 3
pgtrail incident create 'Checkout latency'
# Replace 1 below with the incident ID printed by create.
pgtrail --profile production --incident 1 capture --label 'Before change'
pgtrail incident note 1 'Investigating the oldest blocking transaction'
pgtrail --profile production --incident 1 record --count 12 --interval 5
# Apply any operational change through your normal process.
pgtrail --profile production --incident 1 capture --label 'After change'
pgtrail incident note 1 'Application latency recovered after the change'
pgtrail incident show 1 --output checkout-incident.md
pgtrail incident close 1
```

`diagnose` waits between two observations, computes valid interval metrics, and
saves nothing. `--fail-on warning` exits nonzero for warning or critical findings;
`--fail-on critical` only does so for critical findings. The report is emitted before
that exit. A successful diagnosis with no findings is **not a health guarantee**;
check its coverage and unavailable metrics.

`record` is a bounded foreground session: 12 observations and five seconds between
completed saves by default, configurable with `--count` (1–3600) and `--interval`
(1–3600 seconds). Collection time adds to the cadence. `Ctrl+C` stops the session
and preserves committed captures. A collection or save failure stops recording with
an error. It does not run as a background service.

An incident contains timestamped operator notes and linked captures. Its Markdown
export includes a chronology, annotations, observed findings, captured evidence,
and a guarded first-to-last comparison. New captures and their incident links are
saved atomically. Attachments must refer to the same source database; restarts can
belong to the same incident, while interval comparisons across them remain
unavailable. Closed incidents reject new notes or attachment changes until reopened.

```sh
pgtrail incident list --json
pgtrail incident attach 1 7           # Incident ID, then capture ID
pgtrail incident detach 1 7           # Keeps the capture in History
pgtrail incident reopen 1
pgtrail annotate 7 --label 'Before pool adjustment' --note 'Observed queue growth'
pgtrail incident show 1 --format json --output checkout-incident.json
```

In the TUI, `0` then `Enter` opens every incident note and attached capture in time
order. Navigate with arrows or page keys; `Enter` reads a complete note or opens a
capture offline, and `Backspace` returns to the selected chronology entry. `e` and
`E` export the incident as Markdown or JSON without leaving the app. Enter a file
path (relative to the working directory or absolute); the status line reports the
destination or failure. Exports retain the CLI's private-file and no-overwrite rules.

Capture annotation replaces the capture's note or label. Incident notes are separate,
timestamped additions. Use actual IDs from the command output.

## Capture, compare, and export

```sh
pgtrail check                        # One report; no capture saved
pgtrail capture --label 'Before change'
pgtrail capture --label 'After change'
pgtrail snapshots
pgtrail show 1
pgtrail compare 1 2
pgtrail compare 1 2 --output comparison.md
pgtrail show 1 --format json --output before.json
pgtrail delete 1                     # Deletes a local capture and its incident links
```

`check`, `show`, `compare`, `diagnose`, and `incident show` accept
`--format markdown|json` and `--output PATH`. `snapshots --json` lists captures.
The TUI comparison starts with capture IDs and labels, elapsed interval, source
compatibility, observed session/blocking changes and valid metric changes. `[` / `]`
navigate sections, with the complete evidence and caveats below the summary.
Changes resolved between observations do not establish that a root cause was fixed.

Connection and storage errors exit nonzero. A collected observation with unavailable
optional metrics succeeds and reports partial coverage. Export files use private
permissions and refuse to overwrite an existing file. `show --format json` retains
the snapshot fields and adds `capture: { id, label, note }`.

History defaults to `$XDG_DATA_HOME/pgtrail/history.sqlite3` when that base is
absolute; otherwise it uses:

- macOS: `~/Library/Application Support/pgtrail/history.sqlite3`
- Linux: `~/.local/share/pgtrail/history.sqlite3`

Override it with `--store PATH` or `PGTRAIL_STORE`. New history files use `0600`
permissions on Unix and newly created directories are private. Keep them out of
version control. Deletion removes logical history, not copies in backups or a
storage medium's recoverable data.

Version 1.1 automatically migrates the SQLite history to schema 2. Existing v1
captures remain readable; their new health sections say **not collected**, rather
than showing zeros. Keep a backup before upgrading if rollback matters: a v1.0
binary cannot open the migrated history.

## Interpret observations correctly

Findings explain the observation, the heuristic threshold, and suggested next
checks. They prioritize investigation; they do not establish root cause or apply
changes. A blocked session may depend on a blocker outside this database or one
that disappeared. Active query age and transaction age are separate from accumulated
statement workload. High execution time can reflect volume rather than slow calls.

Database counters, table/index statistics, and activity describe the connected
database. Connection budget, WAL, I/O, replication senders, and slots include cluster
scope. Relation tuple counts are estimates, not exact bloat measurements, and scan
counts are cumulative. The relation collector selects up to 1,000 tables and 1,000
indexes using catalog size estimates before reading their sizes. Truncation is
explicit; this bounded sample is not a guaranteed largest-N inventory. Collection
still performs catalog and size work, including `pg_database_size`; choose a refresh
interval appropriate to the target.

I/O timing is unavailable when its tracking setting is disabled. Time since the
last replayed transaction is not replication backlog: an idle standby can have an
old replay timestamp without lag. Use the available byte positions and workload
context. Slot retention and invalid-index findings require investigation, not
automatic slot removal or index deletion.

Comparisons match sessions by PID **and backend start time**, and statements by
user, database, query ID, and top-level status. Rates use elapsed observation time
and nondecreasing counter deltas. Missing baselines, incompatible sources, restarts,
resets, statement evictions, truncated rankings, and invalid capture ordering prevent
unsupported deltas. An unavailable rate is never substituted with zero. PostgreSQL
statistics can lag activity, and captures are bounded observation windows rather
than atomic views of every subsystem. See [architecture](docs/architecture.md) for
identity and reset rules.

## Local PostgreSQL fixture

Install Docker with Compose and start its engine. The default fixture runs
PostgreSQL 18 with `pg_stat_statements` on `127.0.0.1:55432`:

```sh
docker compose up -d --wait
sh scripts/check-db.sh
export PGTRAIL_DATABASE_URL='postgresql://pgtrail_monitor:pgtrail-local-monitor@127.0.0.1:55432/pgtrail_dev?sslmode=disable'
cargo run --locked
```

These are disposable example credentials. `.env.example` lists Compose overrides.
`pgtrail_admin` provisions the fixture; `pgtrail_monitor` has statistics access,
read-only transactions by default, and no application-table privileges. The smoke
check verifies actual write denials.

`docker compose down` preserves data. Initialization runs only on an empty volume;
changing passwords in `.env` does not update existing roles.
`docker compose down --volumes` deliberately discards development data. Use separate
Compose project names and ports when testing another server major; the compatibility
commands are in [Contributing](CONTRIBUTING.md).

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

[Architecture](docs/architecture.md) covers data flow and invariants.
[Roadmap](docs/roadmap.md) tracks delivered scope and future acceptance criteria.
[Contributing](CONTRIBUTING.md) covers validation and repository conventions.

Licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
