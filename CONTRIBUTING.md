# Contributing

<!-- owner: maintainers; reviewed: 2026-09-28 -->

## Setup

Follow the README quick start. Rustup reads `rust-toolchain.toml`; add
`$HOME/.cargo/bin` to your shell's PATH if your Rust installation did not do so.
On macOS, install the Xcode command-line tools if the linker is missing. On Linux,
install your distribution's native build tools. `just` and Docker are optional for
Rust-only work; PostgreSQL fixture checks require a running Docker engine.

Use a focused branch (the maintainer's default prefix is `gregorsternat/`). Choose
one coherent feature or fix, read the relevant architecture section, and keep unrelated
changes separate. Do not add a dependency or abstraction without a present use.

## Rust practices

Use standard rustfmt, narrow visibility, enums for meaningful states, and concrete
types before introducing generic interfaces. Document non-obvious invariants and
public behavior. Return contextual errors for invalid input or I/O problems rather
than using `unwrap` or `expect` in production paths. Add typed errors when recovery
depends on an error category. Project code forbids `unsafe`. Use `anyhow` at application boundaries and typed
errors where recovery depends on a category. Avoid broad lint suppressions; explain
narrow exceptions inline. Create modules only for implemented responsibilities and
keep public APIs private until a concrete caller requires them.

The minimum supported Rust version is the pinned compiler version, including its
patch release. When updating it, update both Cargo metadata and the toolchain file,
resolve dependencies deliberately, and run the checks. Commit `Cargo.lock`; normal
builds use `--locked`. Do not edit the lockfile by hand.

## Validation

Run `just check`. It runs the following commands, also available without `just`:

```sh
python3 -B scripts/test-check-docs.py
python3 -B scripts/check-docs.py
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python3 scripts/check-terminal.py
```

The same checks run in CI; Rust and PTY checks run on Linux and macOS. Python 3
uses only its standard library. No database is required. `just docs-check` runs
only the documentation checks; `just architecture-check` runs the source boundary
checks and their negative regression fixtures. Test behavior and useful failure
modes; no coverage percentage is required.

For terminal changes, also verify in an actual interactive terminal:

1. Run `cargo run --locked -- --demo`; inspect all ten views and keyboard help.
2. Resize the terminal, including a narrow window; the UI must remain responsive.
3. Quit with `q`; the prompt and cursor must return normally.
4. Run again and quit with `Ctrl+C`; check that typed shell input still echoes.
5. Check `cargo run --locked </dev/null` fails clearly without terminal escapes.

The dependency-free Python 3 PTY check exercises these boundaries with the built
debug binary:

```sh
cargo build --locked
python3 scripts/check-terminal.py
```

It navigates ten views and finding details, creates an incident with notes and
attached captures, compares and reloads offline evidence, filters text containing
`q`, resizes to tiny dimensions, and checks terminal restoration. It also interrupts
a stalled PostgreSQL connection and a recorder waiting for a SQLite write lock.
These automated checks complement visual inspection in your terminal.

For database fixture changes, run `just db-up` and `just db-check`. The check
connects over TCP as `pgtrail_monitor`, checks statistics access and role properties,
and asserts SQLSTATE `42501` for table writes and schema creation even within a
read-write transaction. It must never accept an arbitrary production DSN.

The fixture initializes only on an empty volume. Recreate the disposable development
volume when changing initialization SQL; `just db-down` intentionally preserves it.
Run collector integration tests separately against this fixture:

```sh
PGTRAIL_LIVE_TEST=1 cargo test --locked --lib -- --ignored --test-threads=1
```

These tests are gated against the disposable Compose database. They exercise
monitoring, lock contention/unblocking, supported health sections, and optional
collection failures. Ordinary `cargo test --locked` runs without PostgreSQL. The
live tests can alter fixture permissions to test recovery; never point them at an
operational database.

CI is configured with a separate PostgreSQL integration job for majors 16, 17, and
18. To test another major locally, use its own Compose project and port. Export the
same fixture settings for Compose, the privilege check, and the live tests:

```sh
export COMPOSE_PROJECT_NAME=pgtrail-check-16
export PGTRAIL_POSTGRES_VERSION=16
export PGTRAIL_POSTGRES_PORT=55416
docker compose up -d --wait
sh scripts/check-db.sh
PGTRAIL_LIVE_TEST=1 cargo test --locked --lib -- --ignored --test-threads=1
docker compose down
```

Repeat with `17` and another port/project when needed. Keep separate volumes per
major; changing an image version is not a PostgreSQL data-directory upgrade. Clean
up only the disposable project you created. `docker compose down` keeps its data;
adding `--volumes` deliberately removes that project's development database.

For incident/history changes, verify capture-and-attachment atomicity, closed and
incompatible target rejection, annotations, chronology, and offline exports. For
model changes, retain v1 capture readability and distinguish uncollected fields
from observed NULLs and zero. Metric tests should cover resets, absent or reused
identities, truncation, timing settings, and zero denominators. Tests must not
depend on production credentials or a running server unless explicitly gated.

## Isolated local feedback

Use a temporary directory and explicit paths for ad hoc demo/CLI runs so worktrees
do not share personal history or profiles:

```sh
pgtrail_scratch=$(mktemp -d)
cargo run --locked -- --demo --store "$pgtrail_scratch/history.sqlite3" \
  --profiles-file "$pgtrail_scratch/profiles.json"
```

The PTY and CLI tests already use temporary storage, and the PTY stalled-connection
check binds a local ephemeral port. For live work use a unique Compose project and
port as shown above. Reports in JSON or Markdown, UI coverage/freshness, and failing
test output provide the first debugging evidence. Add targeted timing or tracing
only when needed, without exposing connection configuration or operational SQL.

## Documentation and review

Keep each fact in one place: README for usage/status, architecture for boundaries
and decisions, roadmap for future acceptance criteria, and AGENTS.md for concise
agent instructions. Update relevant documentation with the behavior it describes.
The [knowledge index](docs/index.md) maps owners and review triggers; the
[quality map](docs/quality.md) records coverage gaps. Use a versioned
[execution plan](docs/plans.md) for complex work and preserve the completed plan's
validation evidence. Record unresolved compromises in the
[debt tracker](docs/exec-plans/tech-debt-tracker.md). Documentation checks enforce
local links, heading fragments, discoverability, metadata, and a 90-day review
window. Review facts against code before renewing a date.
Do not commit transcripts, sensitive captures, or copied dependency documentation.

Use English scoped Conventional Commits, for example
`feat(activity): show active sessions` or `fix(tui): restore the terminal on errors`.
PR descriptions explain the concrete problem, resulting behavior, and checks run.
Report unavailable checks honestly; a local success is not a verified CI run.

CI runs on pull requests, pushes to `main`, and manual dispatch. A weekly scheduled
run checks repository documentation for drift; inspect failures and submit focused
fixes through the normal review process. Dependency update PRs are generated
weekly for Cargo and GitHub Actions. Actions are pinned by commit. CI
does not publish to crates.io or create release binaries automatically.
