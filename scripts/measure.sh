#!/usr/bin/env bash
# Startup time and memory of the release build. Run by `just measure`.
#
# Usage: scripts/measure.sh [--synthetic] [--cpu <seconds>] [--binary <path>] [--offline]
#                           [-- <arg>...]
#     --synthetic       measure the release build of the synthetic demo (the `x4-synthetic` binary
#                       of the `synthetic` feature, over 2000 made-up torrents) instead of the app
#     --cpu <seconds>   also measure the CPU the process takes over that many seconds, after the
#                       memory figures, and print it on a second line
#     --binary <path>   measure that file, such as the installed /usr/bin/4x4, instead of building
#                       the app; a relative path is taken from the caller's directory. Not with
#                       --synthetic, which builds the demo
#     --offline         give the program a config dir of its own, whose 4x4/config.toml names a
#                       closed port on this machine (http://127.0.0.1:1/transmission/rpc), so that
#                       the app reaches no daemon, whatever the caller's config says. Not with
#                       --synthetic: the demo reads no config and reaches no network anyway
#     -- <arg>...       arguments for the app or the demo
#
# Builds the release binary (unless --binary names one), starts it with only the 4x4 lifecycle
# log enabled, waits for the UI to settle, then prints one line:
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
# binary_mib is the size of the measured binary file.
# cpu_percent is the user and system time of all the process's threads (utime + stime in
# /proc/<pid>/stat) over the window_s seconds that followed, as a percentage of one core.
#
# The reference limits checked in scripts/measure-lib.sh are rss_anon_mib <= 16, pss_mib <= 55,
# rss_mib <= 120 and gl_stack_mapped=no; they are not the targets in docs/performance.md
# (rss_mib <= 120 is looser than its 100 MiB target). A breach prints a "WARN: ..." line on stderr
# and does not change the exit code, which is non-zero only if the measurement itself failed: the
# app did not start, did not log its first frame, or its /proc files could not be read (2 for a bad
# command line, or a --binary that is not an executable file). The first-frame target (<= 500 ms)
# is reported in docs/performance.md, not checked here.
#
# Env: X4_MEASURE_SETTLE_SECONDS (default: 5), CARGO_TARGET_DIR (default: target). The program is
# started with X4_DEVEL=1 (the development app id) and with XDG_STATE_HOME in a temp dir, so that
# it can never save over the owner's state.toml; its log file goes there too, and the dir is
# removed at the end. Without --offline it reads config.toml as usual, from the caller's
# XDG_CONFIG_HOME or ~/.config; without one no server is set, so it polls nothing and shows the
# Connection dialog, also inside `just virtual`, whose config dir is empty. With --offline the
# config dir is a temp dir too, removed at the end.
set -euo pipefail

settle_seconds=${X4_MEASURE_SETTLE_SECONDS:-5}

# A closed port on this machine: the app keeps trying to connect and never reaches a daemon.
offline_url=http://127.0.0.1:1/transmission/rpc

# usage [<why>]: says why the command line is wrong, if given, and how to write it; exits 2.
usage() {
    if (($# > 0)); then
        echo "measure: $1" >&2
    fi
    echo "usage: scripts/measure.sh [--synthetic] [--cpu <seconds>] [--binary <path>] [--offline] [-- <arg>...]" >&2
    exit 2
}

synthetic=no
cpu_seconds=
binary=
offline=no
args=()
while (($# > 0)); do
    case $1 in
        --synthetic)
            synthetic=yes
            shift
            ;;
        --cpu)
            [[ $# -ge 2 && $2 =~ ^[1-9][0-9]*$ ]] || usage
            cpu_seconds=$2
            shift 2
            ;;
        --binary)
            [[ $# -ge 2 && -n $2 ]] || usage
            binary=$2
            shift 2
            ;;
        --offline)
            offline=yes
            shift
            ;;
        --)
            shift
            args=("$@")
            break
            ;;
        *) usage ;;
    esac
done

if [[ $synthetic == yes && -n $binary ]]; then
    usage "--binary and --synthetic exclude each other: --synthetic builds the demo, --binary measures the given file"
fi
if [[ $synthetic == yes && $offline == yes ]]; then
    usage "--offline has no meaning with --synthetic: the demo reads no config and reaches no network"
fi

# The app ignores a relative XDG_STATE_HOME or XDG_CONFIG_HOME and would save into the owner's
# ~/.local/state, or read the owner's ~/.config, so the temp dirs below must be absolute. A
# relative TMPDIR, like a relative --binary, is taken from the caller's directory, before the cd.
if [[ -n ${TMPDIR:-} && $TMPDIR != /* ]]; then
    export TMPDIR=$PWD/$TMPDIR
fi
if [[ -n $binary && $binary != /* ]]; then
    binary=$PWD/$binary
fi
if [[ -n $binary && ! (-f $binary && -x $binary) ]]; then
    usage "--binary $binary is not an executable file"
fi
cd "$(dirname "$0")/.."
source scripts/measure-lib.sh
if [[ -n $binary ]]; then
    echo "measure: measuring $binary, built elsewhere" >&2
elif [[ $synthetic == yes ]]; then
    cargo build -p x4-app --release --locked --features synthetic --bin x4-synthetic >&2
    binary=${CARGO_TARGET_DIR:-target}/release/x4-synthetic
else
    just release >&2
    binary=${CARGO_TARGET_DIR:-target}/release/4x4
fi

log=$(mktemp)
snapshot=$(mktemp -d)
state_home=$(mktemp -d)
config_home=
app_pid=""
cleanup() {
    if [[ -n $app_pid ]] && kill -0 "$app_pid" 2>/dev/null; then
        kill "$app_pid" 2>/dev/null || true
        wait "$app_pid" 2>/dev/null || true
    fi
    rm -rf "$log" "$snapshot" "$state_home" ${config_home:+"$config_home"}
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
if [[ $offline == yes ]]; then
    config_home=$(mktemp -d)
    mkdir "$config_home/4x4"
    printf '[connection]\nurl = "%s"\n' "$offline_url" >"$config_home/4x4/config.toml"
fi
# The subshell execs the program, so $! is the program's pid.
(
    export X4_DEVEL=1 RUST_LOG=x4_app=info XDG_STATE_HOME="$state_home"
    if [[ -n $config_home ]]; then
        export XDG_CONFIG_HOME="$config_home"
    fi
    exec "$binary" "${args[@]}"
) 2>"$log" &
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

# -L: a symlink (such as one to the installed binary) reports the size of its target, not its own.
binary_bytes=$(stat -L -c %s "$binary")

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
