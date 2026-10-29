#!/usr/bin/env bash
# Runs one command inside a headless KWin and exits with the command's exit code. Run by
# `just virtual <command> [<arg>...]`. KDE Plasma only: it needs kwin_wayland and dbus-run-session.
#
# The command gets a virtual screen that no monitor shows, so it can open windows, measure them
# and take screenshots (`spectacle -b -n -f -o <file>`) without showing anything on the real
# desktop or taking its focus. The command's stdout and stderr are the caller's; KWin's own
# output goes to kwin.log in the session's temp dir, which is removed at the end.
#
# Env: X4_VIRTUAL_WIDTH and X4_VIRTUAL_HEIGHT, the size of the virtual screen (default 1440x900);
# X4_VIRTUAL_KEEP=1 keeps the session's temp dir (kwin.log, the session's XDG dirs) and prints
# its path. Exits with 127 when a tool is missing, and with 125 when KWin ended before the command
# ran (KWin's log is printed then).
set -euo pipefail

for tool in kwin_wayland dbus-run-session; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "virtual-session: $tool not found; the virtual session needs KDE Plasma's KWin" \
            "(kwin_wayland) and D-Bus (dbus-run-session)" >&2
        exit 127
    fi
done
if (($# == 0)); then
    echo "usage: scripts/virtual-session.sh <command> [<arg>...]" >&2
    exit 2
fi

width=${X4_VIRTUAL_WIDTH:-1440}
height=${X4_VIRTUAL_HEIGHT:-900}
owner_config=${XDG_CONFIG_HOME:-$HOME/.config}

root=$(mktemp -d "${TMPDIR:-/tmp}/x4-virtual.XXXXXX")

# Stops every process that still runs with the session's XDG dirs. The services the session's
# D-Bus starts (xdg-desktop-portal-kde, ksecretd) outlive their bus and their compositor and keep
# writing to the session's dirs; the session's path in the environment marks them, and nothing
# else has it.
stop_leftovers() {
    local environ pid pids=() alive=() _
    for environ in /proc/[0-9]*/environ; do
        pid=${environ#/proc/}
        pid=${pid%/environ}
        if [[ $pid != "$$" ]] && grep -qzxF -- "XDG_DATA_HOME=$root/data" "$environ" 2>/dev/null; then
            pids+=("$pid")
        fi
    done
    ((${#pids[@]} > 0)) || return 0
    kill "${pids[@]}" 2>/dev/null || true
    for _ in {1..20}; do
        alive=()
        for pid in "${pids[@]}"; do
            if kill -0 "$pid" 2>/dev/null; then alive+=("$pid"); fi
        done
        ((${#alive[@]} > 0)) || return 0
        sleep 0.1
    done
    kill -KILL "${alive[@]}" 2>/dev/null || true
}

cleanup() {
    stop_leftovers
    if [[ ${X4_VIRTUAL_KEEP:-} == 1 ]]; then
        echo "virtual-session: kept $root" >&2
    else
        rm -rf "$root"
    fi
}
trap cleanup EXIT

# The session has XDG dirs of its own, and below a D-Bus session of its own, because the nested
# KWin and spectacle save their settings (kwinoutputconfig.json, kwinrc, kglobalshortcutsrc,
# spectaclerc) through them. A run with the owner's dirs and bus once rewrote the owner's files.
mkdir -p "$root/config" "$root/data" "$root/state" "$root/cache"
# Copies of the owner's font and KDE settings, so the window looks as it does on the desktop.
# Anything the session writes lands in the copies; the owner's files are only read.
if [[ -d $owner_config/fontconfig ]]; then
    cp -r "$owner_config/fontconfig" "$root/config/"
fi
if [[ -f $owner_config/kdeglobals ]]; then
    cp "$owner_config/kdeglobals" "$root/config/"
fi

# The command goes inside the session script, quoted word by word. KWin starts every positional
# argument as an app of its own and quits as soon as the --exit-with-session script ends, so
# `--exit-with-session <command> <arg>` would quit at once. The script takes back the caller's
# stdout and stderr from fds 3 and 4, which KWin passes on, and unsets DISPLAY and XAUTHORITY, so
# that nothing in it can fall back to the X server of the owner's desktop.
session=$root/session.sh
{
    printf '#!/usr/bin/env bash\n'
    printf 'exec 1>&3 2>&4 3>&- 4>&-\n'
    printf 'unset DISPLAY XAUTHORITY\n'
    printf 'status=0\n'
    printf '%q ' "$@"
    printf '|| status=$?\n'
    printf 'echo "$status" >%q\n' "$root/status"
} >"$session"
chmod +x "$session"

kwin_status=0
XDG_CONFIG_HOME=$root/config XDG_DATA_HOME=$root/data XDG_STATE_HOME=$root/state \
    XDG_CACHE_HOME=$root/cache \
    dbus-run-session -- kwin_wayland --virtual --no-lockscreen --socket "x4-virtual-$$" \
    --width "$width" --height "$height" --exit-with-session "$session" \
    3>&1 4>&2 >"$root/kwin.log" 2>&1 || kwin_status=$?

if [[ ! -s $root/status ]]; then
    echo "virtual-session: the command did not run; KWin exited with $kwin_status. KWin's log:" >&2
    cat "$root/kwin.log" >&2
    exit 125
fi
exit "$(<"$root/status")"
