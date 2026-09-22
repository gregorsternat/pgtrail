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

check:
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    cargo build --locked

db-up:
    docker compose up -d --wait

db-down:
    docker compose down

db-check:
    sh scripts/check-db.sh
