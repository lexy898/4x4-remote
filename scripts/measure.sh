#!/usr/bin/env bash
# Startup time and memory of the release build. Run by `just measure`.
#
# Builds the release binary, starts it with only the 4x4 lifecycle log enabled, waits for the UI
# to settle, then prints one line:
#     first_frame_ms=<n> rss_mib=<n> rss_anon_mib=<n> binary_mib=<n>
# rss_mib is the whole resident set (VmRSS, including shared libraries mapped from disk);
# rss_anon_mib is its anonymous part (RssAnon): heap, stacks and other private allocations.
# Exits non-zero only if the app failed to start or did not log its first frame. The targets
# (first frame <= 500 ms, RSS <= 100 MiB) are reported in README.md, not enforced here.
#
# Env: X4_MEASURE_SETTLE_SECONDS (default: 5), CARGO_TARGET_DIR (default: target).
set -euo pipefail

settle_seconds=${X4_MEASURE_SETTLE_SECONDS:-5}

cd "$(dirname "$0")/.."
just release >&2
binary=${CARGO_TARGET_DIR:-target}/release/4x4

log=$(mktemp)
app_pid=""
cleanup() {
    if [[ -n $app_pid ]] && kill -0 "$app_pid" 2>/dev/null; then
        kill "$app_pid" 2>/dev/null || true
        wait "$app_pid" 2>/dev/null || true
    fi
    rm -f "$log"
}
trap cleanup EXIT

fail() {
    echo "measure: $1" >&2
    if [[ -s $log ]]; then
        echo "--- app log:" >&2
        cat "$log" >&2
    fi
    exit 1
}

RUST_LOG=x4_app=info "$binary" 2>"$log" &
app_pid=$!
sleep "$settle_seconds"

if ! kill -0 "$app_pid" 2>/dev/null; then
    fail "the app exited during startup"
fi

status=$(<"/proc/$app_pid/status") || fail "could not read /proc/$app_pid/status"
rss_kib=$(awk '/^VmRSS:/ { print $2 }' <<<"$status")
[[ -n $rss_kib ]] || fail "could not read VmRSS of pid $app_pid"
rss_anon_kib=$(awk '/^RssAnon:/ { print $2 }' <<<"$status")
[[ -n $rss_anon_kib ]] || fail "could not read RssAnon of pid $app_pid"

first_frame_ms=$(sed -n 's/.*first frame rendered.*elapsed_ms=\([0-9][0-9]*\).*/\1/p' "$log" | head -n 1)
[[ -n $first_frame_ms ]] || fail "the app did not log 'first frame rendered' within ${settle_seconds} s"

binary_bytes=$(stat -c %s "$binary")

awk -v ms="$first_frame_ms" -v rss="$rss_kib" -v anon="$rss_anon_kib" -v bin="$binary_bytes" 'BEGIN {
    printf "first_frame_ms=%d rss_mib=%.1f rss_anon_mib=%.1f binary_mib=%.1f\n",
        ms, rss / 1024, anon / 1024, bin / 1048576
}'
