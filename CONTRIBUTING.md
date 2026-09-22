# Contributing

## Setup

Follow the README quick start. Rustup reads `rust-toolchain.toml`; add
`$HOME/.cargo/bin` to your shell's PATH if your Rust installation did not do so.
On macOS, install the Xcode command-line tools if the linker is missing. On Linux,
install your distribution's native build tools. `just` and Docker are optional for
Rust-only work; PostgreSQL fixture checks require a running Docker engine.

Use a focused branch (the maintainer's default prefix is `gregorsternat/`). Choose
one roadmap slice or fix, read the relevant architecture section, and keep unrelated
changes separate. Do not add a dependency or abstraction without a present use.

## Rust practices

Use standard rustfmt, narrow visibility, enums for meaningful states, and concrete
types before introducing generic interfaces. Document non-obvious invariants and
public behavior. Return contextual errors for invalid input or I/O problems rather
than using `unwrap` or `expect` in production paths. Add typed errors when recovery
depends on an error category. Project code forbids `unsafe`.

The minimum supported Rust version is the pinned compiler version, including its
patch release. When updating it, update both Cargo metadata and the toolchain file,
resolve dependencies deliberately, and run the checks. Commit `Cargo.lock`; normal
builds use `--locked`. Do not edit the lockfile by hand.

## Validation

Run `just check` or the four equivalent Cargo commands in the README. The same
commands run on Linux and macOS in CI. No database is required for these checks.
Test behavior and useful failure modes; no coverage percentage is required.

For terminal changes, also verify in an actual interactive terminal:

1. Run `cargo run --locked` and check the current capability message.
2. Resize the terminal, including a narrow window; the UI must remain responsive.
3. Quit with `q`; the prompt and cursor must return normally.
4. Run again and quit with `Ctrl+C`; check that typed shell input still echoes.
5. Check `cargo run --locked </dev/null` fails clearly without terminal escapes.

For database fixture changes, run `just db-up` and `just db-check`. The check
connects over TCP as `pgtrail_monitor`, checks statistics access and role properties,
and asserts SQLSTATE `42501` for table writes and schema creation even within a
read-write transaction. It must never accept an arbitrary production DSN.

The fixture initializes only on an empty volume. Recreate the disposable development
volume when changing initialization SQL; `just db-down` intentionally preserves it.
Future collector integration tests belong in a separate explicitly invoked suite
so ordinary `cargo test --locked` continues to run without PostgreSQL.

## Documentation and review

Keep each fact in one place: README for usage/status, architecture for boundaries
and decisions, roadmap for future acceptance criteria, and AGENTS.md for concise
agent instructions. Update relevant documentation with the behavior it describes.
Do not commit transcripts, sensitive captures, or copied dependency documentation.

Use English scoped Conventional Commits, for example
`feat(activity): show active sessions` or `fix(tui): restore the terminal on errors`.
PR descriptions explain the concrete problem, resulting behavior, and checks run.
Report unavailable checks honestly; a local success is not a verified CI run.

CI runs on pull requests and pushes to `main`. Dependency update PRs are generated
weekly for Cargo and GitHub Actions. Actions are pinned by commit. This foundation
does not publish to crates.io or create release binaries automatically.
