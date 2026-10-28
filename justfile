set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Full gate. The only definition of "done"; CI runs exactly this.
check: fmt-check toml-check spell lint-policy lint test test-hooks doc-test doc deny machete

# Fast gate run by the agent Stop hook.
check-fast: fmt-check lint test

# Run by .githooks/pre-commit.
pre-commit: fmt-check toml-check spell lint-policy lint

fmt:
    cargo fmt --all
    taplo fmt

fmt-check:
    cargo fmt --all --check

toml-check:
    taplo fmt --check

spell:
    typos

# AGENTS.md rules clippy cannot express: lint exceptions, workspace lints, dependency direction.
lint-policy:
    scripts/lint-policy.sh

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

test:
    cargo nextest run --workspace --locked --no-tests=warn

doc-test:
    cargo test --workspace --doc --locked

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

deny:
    cargo deny check --hide-inclusion-graph

machete:
    cargo machete

# Enable the git hooks and verify the required tools are installed.
setup:
    git config core.hooksPath .githooks
    scripts/check-tools.sh

# Tests for the agent and git hook scripts.
test-hooks:
    bash scripts/hooks/test-hooks.sh

run *args:
    cargo run -p x4-app --locked -- {{args}}

release:
    cargo build -p x4-app --release --locked

# Startup time to the first frame, resident memory and binary size of the release build.
measure:
    scripts/measure.sh

update:
    cargo update
    just check
