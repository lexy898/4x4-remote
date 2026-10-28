#!/usr/bin/env bash
# Startup time and memory of the release build. Run by `just measure`.
#
# Builds the release binary, starts it with only the 4x4 lifecycle log enabled, waits for the UI
# to settle, then prints one line:
#     first_frame_ms=<n> pss_mib=<n> uss_mib=<n> rss_anon_mib=<n> rss_mib=<n> threads=<n>
#     gl_stack_mapped=yes|no binary_mib=<n>
# pss_mib is the proportional set size (Pss in /proc/<pid>/smaps_rollup): every shared page counts
# divided by the number of processes that map it, so it is the app's fair share.
# uss_mib is the unique set size (Private_Clean + Private_Dirty): what exiting the app frees.
# rss_anon_mib is the anonymous resident memory (RssAnon): heap, stacks and other private
# allocations, which only swap can take back.
# rss_mib is the whole resident set (VmRSS), shared libraries included. It is a reference only.
# gl_stack_mapped says whether Mesa's OpenGL stack (libgallium, libEGL_mesa or libGLX_mesa) is
# mapped, found in /proc/<pid>/maps; a Vulkan-only process does not map it.
#
# The reference limits checked in scripts/measure-lib.sh are rss_anon_mib <= 16, pss_mib <= 55,
# rss_mib <= 120 and gl_stack_mapped=no; they are not the targets in README.md (rss_mib <= 120 is
# looser than its 100 MiB target). A breach prints a "WARN: ..." line on stderr and does not
# change the exit code, which is non-zero only if the measurement itself failed: the app did not
# start, did not log its first frame, or its /proc files could not be read. The first-frame
# target (<= 500 ms) is reported in README.md, not checked here.
#
# Env: X4_MEASURE_SETTLE_SECONDS (default: 5), CARGO_TARGET_DIR (default: target).
set -euo pipefail

settle_seconds=${X4_MEASURE_SETTLE_SECONDS:-5}

cd "$(dirname "$0")/.."
source scripts/measure-lib.sh
just release >&2
binary=${CARGO_TARGET_DIR:-target}/release/4x4

log=$(mktemp)
snapshot=$(mktemp -d)
app_pid=""
cleanup() {
    if [[ -n $app_pid ]] && kill -0 "$app_pid" 2>/dev/null; then
        kill "$app_pid" 2>/dev/null || true
        wait "$app_pid" 2>/dev/null || true
    fi
    rm -rf "$log" "$snapshot"
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

# Copy the /proc files once, so every figure comes from the same moment and the parsing in
# scripts/measure-lib.sh works on plain files.
for name in smaps_rollup status maps; do
    cat "/proc/$app_pid/$name" >"$snapshot/$name" || fail "could not read /proc/$app_pid/$name"
done

first_frame_ms=$(sed -n 's/.*first frame rendered.*elapsed_ms=\([0-9][0-9]*\).*/\1/p' "$log" | head -n 1)
[[ -n $first_frame_ms ]] || fail "the app did not log 'first frame rendered' within ${settle_seconds} s"

binary_bytes=$(stat -c %s "$binary")

report_line "$first_frame_ms" "$snapshot/smaps_rollup" "$snapshot/status" "$snapshot/maps" "$binary_bytes" ||
    fail "could not read the memory figures of pid $app_pid"
report_warnings "$snapshot/smaps_rollup" "$snapshot/status" "$snapshot/maps" ||
    fail "could not read the memory figures of pid $app_pid"
