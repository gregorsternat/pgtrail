# pgtrail

A Rust terminal application for investigating PostgreSQL activity and comparing
diagnostic snapshots.

**Status: executable foundation.** The TUI starts and exits cleanly, but does not
connect to a database, collect metrics, or save snapshots yet. The planned features
are active sessions, blocking transactions, expensive queries, and manual snapshots
stored locally in SQLite.

## Quick start

Development targets Linux and macOS. Install [Rust with rustup](https://rustup.rs/),
including your platform's native compiler/linker. The repository pins its toolchain
and includes Clippy and rustfmt; rustup installs them on first use.

If Rust was just installed, open a new shell or run `. "$HOME/.cargo/env"` in
bash/zsh (`source "$HOME/.cargo/env.fish"` in fish).

```sh
git clone https://github.com/gregorsternat/pgtrail.git
cd pgtrail
cargo run --locked -- --help
cargo run --locked
```

Run the TUI in an interactive terminal. Press `q` or `Ctrl+C` to quit. `--help` and
`--version` also work in scripts. No database or configuration is needed to start.

## Development commands

[just](https://just.systems/) is an optional command runner; Cargo and Docker work
directly too. Install it using your package manager, or `cargo install just --locked`.

| Task | Shortcut | Direct command |
| --- | --- | --- |
| Run | `just run` | `cargo run --locked` |
| Format | `just fmt` | `cargo fmt --all` |
| Lint | `just lint` | `cargo clippy --locked --all-targets -- -D warnings` |
| Test | `just test` | `cargo test --locked` |
| Build | — | `cargo build --locked` |
| All Rust checks | `just check` | Format check, lint, test, build; see below |
| Start local database | `just db-up` | `docker compose up -d --wait` |
| Check database | `just db-check` | `sh scripts/check-db.sh` |
| Stop database | `just db-down` | `docker compose down` |

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
```

## Local PostgreSQL

Install Docker with Compose and start its engine (for example, OrbStack on macOS).
PostgreSQL is optional for the current TUI, but ready for the first collector.

```sh
# Optional: copy once if you want to override the local defaults.
cp .env.example .env
docker compose up -d --wait
sh scripts/check-db.sh
```

The development database is `pgtrail_dev` on `127.0.0.1:55432`, running PostgreSQL 18
with `pg_stat_statements` enabled. Its disposable example credentials are in
`.env.example`; do not reuse them outside this local environment.

- `pgtrail_monitor` is the future application account: statistics access, no
  superuser or application-table privileges, and read-only transactions by default.
- `pgtrail_admin` owns the development database and performs initialization. Never
  use it as the application's diagnostic account.
- `.env` currently configures Compose only. The Rust binary does not consume
  connection settings yet. No connection CLI flag is implemented.

The smoke check uses the monitoring account over TCP and verifies real privilege
denials, not just the read-only default. It only targets this Compose project.

`docker compose down` preserves local data. Initialization runs only on an empty
volume: changing a password in `.env` does not change an existing database role.
To apply initialization changes, deliberately discard this **development database**
with `docker compose down --volumes`, then start it again. A port conflict can be
resolved with `PGTRAIL_POSTGRES_PORT` in `.env`.

## Project guides

- [Architecture](docs/architecture.md): implemented boundaries and planned data flow.
- [Roadmap](docs/roadmap.md): small feature slices with acceptance criteria.
- [Contributing](CONTRIBUTING.md): development workflow and validation.
- [Agent instructions](AGENTS.md): concise entry point for AI coding agents.

## License

Licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
