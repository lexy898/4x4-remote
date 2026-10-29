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

# AGENTS.md rules clippy cannot express: lint exceptions, workspace lints, dependency direction,
# and that the vendored gpui-pre-wgpu patch is applied.
lint-policy:
    scripts/lint-policy.sh

# The gates build x4-app with its `synthetic` feature too, so that the demo is linted, tested and
# documented with the rest.
lint:
    cargo clippy --workspace --all-targets --features x4-app/synthetic --locked -- -D warnings

test:
    cargo nextest run --workspace --features x4-app/synthetic --locked --no-tests=warn

doc-test:
    cargo test --workspace --doc --locked

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --features x4-app/synthetic --locked

deny:
    cargo deny check --hide-inclusion-graph

# Only our crates: vendor/ holds third-party code (vendor/gpui-pre-wgpu/PATCHED.md).
machete:
    cargo machete crates

# Re-record the scrubbed test fixtures from a real daemon (read-only calls). X4_LIVE_URL overrides the endpoint.
record-fixtures:
    X4_LIVE_URL="${X4_LIVE_URL:-http://192.168.1.5:9091/transmission/rpc}" cargo run -p x4-rpc --example record_fixtures --features test-support --locked

# --no-capture runs the tests one at a time and streams their output, so the hash each test
# prints for its torrent stays visible even if the run is interrupted (nextest drops the captured
# output of a cancelled test).
# Live tests on a torrent they create in a test directory: X4_LIVE_URL, X4_LIVE_REMOTE_DIR (daemon's path) and X4_LIVE_LOCAL_DIR (mounted here) override the defaults.
test-live:
    X4_LIVE_URL="${X4_LIVE_URL:-http://192.168.1.5:9091/transmission/rpc}" \
    X4_LIVE_REMOTE_DIR="${X4_LIVE_REMOTE_DIR:-/mnt/hdd/torrents/4x4-test}" \
    X4_LIVE_LOCAL_DIR="${X4_LIVE_LOCAL_DIR:-/mnt/pi-hdd/torrents/4x4-test}" \
    cargo nextest run -p x4-rpc --test live --run-ignored only --locked --no-capture

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

# The main window over a synthetic daemon of 2000 torrents, no network: `--static` (rates stay 0), `--scroll` (scroll 10 s, log frame intervals, quit), `--select N` (select row N), `--extend K` (then Shift+Down K times).
demo-synthetic *args:
    RUST_LOG="${RUST_LOG:-warn,x4_app=info,x4_core=info,x4_synthetic=info}" \
        cargo run -p x4-app --features synthetic --bin x4-synthetic --locked -- {{args}}

# Startup time, PSS, USS, anonymous and resident memory, threads, GL stack and binary size of the release build; also `--synthetic` (the demo), `--cpu <s>`, `-- <args>`.
[positional-arguments]
measure *args:
    scripts/measure.sh "$@"

# Run a command in a headless KWin, off the real screens (KDE only): `just virtual just run`; X4_VIRTUAL_KEEP=1 keeps its temp dir.
[positional-arguments]
virtual +cmd:
    scripts/virtual-session.sh "$@"

update:
    cargo update
    just check
