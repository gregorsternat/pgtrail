default:
    @just --list

run:
    cargo run --locked

fmt:
    cargo fmt --all

lint:
    cargo clippy --locked --all-targets -- -D warnings

test:
    cargo test --locked

docs-check:
    python3 -B scripts/test-check-docs.py
    python3 -B scripts/check-docs.py

architecture-check:
    cargo test --locked --test architecture

terminal-check:
    cargo build --locked
    python3 scripts/check-terminal.py

check: docs-check
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    cargo build --locked
    python3 scripts/check-terminal.py

db-up:
    docker compose up -d --wait

db-down:
    docker compose down

db-check:
    sh scripts/check-db.sh
