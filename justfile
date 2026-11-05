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
# Live tests (x4-rpc's and x4-core's) on torrents they create in a test directory: X4_LIVE_URL, X4_LIVE_REMOTE_DIR (daemon's path) and X4_LIVE_LOCAL_DIR (mounted here) override the defaults. x4-app's checks the server as the Connection dialog's Test does, reading only. x4-core's geoip_live reads the installed GeoIP country files, from X4_GEOIP_DIR if set.
test-live:
    X4_LIVE_URL="${X4_LIVE_URL:-http://192.168.1.5:9091/transmission/rpc}" \
    X4_LIVE_REMOTE_DIR="${X4_LIVE_REMOTE_DIR:-/mnt/hdd/torrents/4x4-test}" \
    X4_LIVE_LOCAL_DIR="${X4_LIVE_LOCAL_DIR:-/mnt/pi-hdd/torrents/4x4-test}" \
    cargo nextest run -p x4-rpc -p x4-core -p x4-app --test live --test geoip_live --run-ignored only --locked --no-capture

# Enable the git hooks and verify the required tools are installed.
setup:
    git config core.hooksPath .githooks
    scripts/check-tools.sh

# Tests for the agent and git hook scripts.
test-hooks:
    bash scripts/hooks/test-hooks.sh

# The arguments go to the app as given, so that a quoted magnet link stays one argument.
[positional-arguments]
run *args:
    cargo run -p x4-app --locked -- "$@"

release:
    cargo build -p x4-app --release --locked

# Build the Arch package of the committed master branch (never the working tree or another branch) under ~/.cache/4x4-pkg and print its path; install it with `sudo pacman -U <path>`.
package:
    #!/usr/bin/env bash
    set -euo pipefail
    # Everything built comes from the local master branch, its PKGBUILD as well as its sources;
    # without them, nothing is written. A directory that git cannot read as a repository (a copy
    # without .git, or one git refuses) must not look like a master without the PKGBUILD.
    if ! git rev-parse --git-dir >/dev/null; then
        echo "just package builds the local master branch of the checkout it is in, which is not a" \
            "git repository (git's message is above)" >&2
        exit 1
    fi
    if ! git cat-file -e refs/heads/master:packaging/arch/PKGBUILD 2>/dev/null; then
        echo "just package builds the local master branch, which has no packaging/arch/PKGBUILD:" \
            "merge the branch that adds it into master, then run git switch master && git pull," \
            "then just package" >&2
        exit 1
    fi
    # The local master may lag behind the origin/master that was last fetched. That is only a
    # warning (an older master may be wanted), and it compares with what is there: nothing is
    # fetched, so it is as old as the last fetch, and without that ref it says nothing.
    if git rev-parse -q --verify refs/remotes/origin/master >/dev/null; then
        behind=$(git rev-list --count refs/heads/master..refs/remotes/origin/master)
        if ((behind > 0)); then
            noun=commits
            if ((behind == 1)); then noun=commit; fi
            echo "warning: the local master is $behind $noun behind origin/master (as last fetched);" \
                "to build the newer ones, run git switch master && git pull, then just package" >&2
        fi
    fi
    commit=$(git log -1 --format='%h %s' refs/heads/master)
    echo "building master: $commit"
    repo="{{justfile_directory()}}"
    cache="${XDG_CACHE_HOME:-$HOME/.cache}/4x4-pkg"
    mkdir -p "$cache/build" "$cache/sources" "$cache/packages"
    # makepkg rewrites pkgver= in the PKGBUILD it runs, so it runs a copy, and master's, as the
    # sources it builds are.
    git show refs/heads/master:packaging/arch/PKGBUILD >"$cache/PKGBUILD"
    # makepkg refuses a source mirror made from another checkout (this one moved, or a clone), and
    # the working copy made from that mirror cannot follow this checkout's commits: both go, and the
    # build starts over.
    mirror=$cache/sources/4x4
    origin=$(git config --file "$mirror/config" --get remote.origin.url 2>/dev/null || true)
    if [[ -d $mirror && $origin != "file://$repo" ]]; then
        echo "the source mirror in $cache was made from another checkout; starting over"
        rm -rf "$mirror" "$cache/build/4x4-git/src"
    fi
    cd "$cache"
    export X4_REPO="$repo" BUILDDIR="$cache/build" SRCDEST="$cache/sources" PKGDEST="$cache/packages"
    makepkg --force
    makepkg --packagelist

# Make 4x4 the default app for .torrent files and magnet links, after asking on the terminal; `--undo` gives them back, `--yes` answers the question. The owner runs it; agents never do.
[positional-arguments]
default-handler *args:
    scripts/default-handler.sh "$@"

# The main window over a synthetic daemon of 2000 torrents, no network: `--static` (rates stay 0), `--scroll` (scroll 10 s, log frame intervals, quit), `--select N` (select row N), `--extend K` (then Shift+Down K times), `--right-click` (then right-click row N), `--press KEY` (press a key, repeatable, one every 3 s), `--connection` (open the Connection dialog), `--showcase` (24 torrents with made-up names, for screenshots), `--theme system|light|dark`.
demo-synthetic *args:
    RUST_LOG="${RUST_LOG:-warn,x4_app=info,x4_core=info,x4_synthetic=info}" \
        cargo run -p x4-app --features synthetic --bin x4-synthetic --locked -- {{args}}

# Startup time, PSS, USS, anonymous and resident memory, threads, GL stack and binary size of the release build; also `--synthetic` (the demo), `--cpu <s>`, `--binary <path>` (measure that file, build nothing), `--offline` (a config of a closed port, so no daemon is reached; not with `--synthetic`), `-- <args>`.
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
