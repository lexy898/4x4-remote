set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Full gate. The only definition of "done"; CI runs exactly this.
check: fmt-check toml-check spell lint test doc-test doc deny machete

# Fast gate run by the agent Stop hook.
check-fast: fmt-check lint test

# Run by .githooks/pre-commit.
pre-commit: fmt-check toml-check spell lint

fmt:
    cargo fmt --all
    taplo fmt

fmt-check:
    cargo fmt --all --check

toml-check:
    taplo fmt --check

spell:
    typos

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

test:
    cargo nextest run --workspace --locked --no-tests=warn

doc-test:
    cargo test --workspace --doc --locked

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

deny:
    cargo deny check

machete:
    cargo machete

run *args:
    cargo run -p x4-app --locked -- {{args}}

release:
    cargo build -p x4-app --release --locked

update:
    cargo update
    just check
