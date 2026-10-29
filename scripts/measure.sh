#!/usr/bin/env bash
# Startup time and memory of the release build. Run by `just measure`.
#
# Usage: scripts/measure.sh [--example <name>] [--cpu <seconds>] [-- <arg>...]
#     --example <name>  measure the release build of that example of x4-app (for instance
#                       `synthetic`) instead of the app
#     --cpu <seconds>   also measure the CPU the process takes over that many seconds, after the
#                       memory figures, and print it on a second line
#     -- <arg>...       arguments for the app or the example
#
# Builds the release binary, starts it with only the 4x4 lifecycle log enabled, waits for the UI
# to settle, then prints one line:
#     first_frame_ms=<n> pss_mib=<n> uss_mib=<n> rss_anon_mib=<n> rss_mib=<n> threads=<n>
#     gl_stack_mapped=yes|no binary_mib=<n>
# and, with --cpu, a second one:
#     cpu_percent=<n> window_s=<n>
# pss_mib is the proportional set size (Pss in /proc/<pid>/smaps_rollup): every shared page counts
# divided by the number of processes that map it, so it is the app's fair share.
# uss_mib is the unique set size (Private_Clean + Private_Dirty): what exiting the app frees.
# rss_anon_mib is the anonymous resident memory (RssAnon): heap, stacks and other private
# allocations, which only swap can take back.
# rss_mib is the whole resident set (VmRSS), shared libraries included. It is a reference only.
# gl_stack_mapped says whether Mesa's OpenGL stack (libgallium, libEGL_mesa or libGLX_mesa) is
# mapped, found in /proc/<pid>/maps; a Vulkan-only process does not map it.
# cpu_percent is the user and system time of all the process's threads (utime + stime in
# /proc/<pid>/stat) over the window_s seconds that followed, as a percentage of one core.
#
# The reference limits checked in scripts/measure-lib.sh are rss_anon_mib <= 16, pss_mib <= 55,
# rss_mib <= 120 and gl_stack_mapped=no; they are not the targets in README.md (rss_mib <= 120 is
# looser than its 100 MiB target). A breach prints a "WARN: ..." line on stderr and does not
# change the exit code, which is non-zero only if the measurement itself failed: the app did not
# start, did not log its first frame, or its /proc files could not be read (2 for a bad command
# line). The first-frame target (<= 500 ms) is reported in README.md, not checked here.
#
# Env: X4_MEASURE_SETTLE_SECONDS (default: 5), CARGO_TARGET_DIR (default: target). The app is
# started with X4_DEVEL=1 (the development app id) and with XDG_STATE_HOME in a temp dir, so that
# it can never save over the owner's state.toml; its log file goes there too, and the dir is
# removed at the end. It reads config.toml as usual, so without one it polls the default daemon.
set -euo pipefail

settle_seconds=${X4_MEASURE_SETTLE_SECONDS:-5}

usage() {
    echo "usage: scripts/measure.sh [--example <name>] [--cpu <seconds>] [-- <arg>...]" >&2
    exit 2
}

example=
cpu_seconds=
args=()
while (($# > 0)); do
    case $1 in
        --example)
            [[ $# -ge 2 && -n $2 ]] || usage
            example=$2
            shift 2
            ;;
        --cpu)
            [[ $# -ge 2 && $2 =~ ^[1-9][0-9]*$ ]] || usage
            cpu_seconds=$2
            shift 2
            ;;
        --)
            shift
            args=("$@")
            break
            ;;
        *) usage ;;
    esac
done

cd "$(dirname "$0")/.."
source scripts/measure-lib.sh
if [[ -n $example ]]; then
    cargo build -p x4-app --release --locked --example "$example" >&2
    binary=${CARGO_TARGET_DIR:-target}/release/examples/$example
else
    just release >&2
    binary=${CARGO_TARGET_DIR:-target}/release/4x4
fi

log=$(mktemp)
snapshot=$(mktemp -d)
state_home=$(mktemp -d)
app_pid=""
cleanup() {
    if [[ -n $app_pid ]] && kill -0 "$app_pid" 2>/dev/null; then
        kill "$app_pid" 2>/dev/null || true
        wait "$app_pid" 2>/dev/null || true
    fi
    rm -rf "$log" "$snapshot" "$state_home"
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

# X4_DEVEL=1 gives the window the development app id, as in a debug build, so the measurement
# window is not mistaken for the owner's own (the window rule for test windows matches that id).
# The temp XDG_STATE_HOME keeps its state.toml and its log away from the owner's.
X4_DEVEL=1 RUST_LOG=x4_app=info XDG_STATE_HOME="$state_home" "$binary" "${args[@]}" 2>"$log" &
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

if [[ -n $cpu_seconds ]]; then
    started=$EPOCHREALTIME
    cat "/proc/$app_pid/stat" >"$snapshot/stat-before" || fail "could not read /proc/$app_pid/stat"
    sleep "$cpu_seconds"
    cat "/proc/$app_pid/stat" >"$snapshot/stat-after" || fail "could not read /proc/$app_pid/stat"
    window_s=$(elapsed_seconds "$started" "$EPOCHREALTIME")
    cpu_line "$snapshot/stat-before" "$snapshot/stat-after" "$window_s" "$(getconf CLK_TCK)" ||
        fail "could not read the CPU time of pid $app_pid"
fi
