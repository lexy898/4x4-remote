#!/usr/bin/env bash
# Runs one command inside a headless KWin and exits with the command's exit code. Run by
# `just virtual <command> [<arg>...]`. KDE Plasma only: it needs kwin_wayland and dbus-run-session.
#
# The command gets a virtual screen that no monitor shows, so it can open windows, measure them
# and take screenshots (`spectacle -b -n -f -o <file>`) without showing anything on the real
# desktop or taking its focus. The session has its own D-Bus session, its own XDG config, data,
# state, cache and runtime dirs and its own HOME, and neither it nor anything it starts sees the
# owner's DISPLAY or XAUTHORITY; cargo and rustup keep the owner's CARGO_HOME and RUSTUP_HOME, so
# builds inside reuse the registry and the toolchains. The command's stdout and stderr are the
# caller's; KWin's own output goes to kwin.log in the session's temp dir, which is removed at the
# end, after every process still running with the session's dirs is stopped.
#
# Env: X4_VIRTUAL_WIDTH and X4_VIRTUAL_HEIGHT, the size of the virtual screen (default 1440x900);
# X4_VIRTUAL_KEEP=1 keeps the session's temp dir (kwin.log, the session's XDG dirs and HOME) and
# prints its path. Exits with 127 when a tool is missing, with 125 when KWin ended before the
# command ran (KWin's log is printed then), and with 2 for a bad command line or a TMPDIR so long
# that KWin's socket path in it would not fit a Unix socket address (107 bytes).
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
# Cargo's and rustup's homes stay the owner's while HOME changes below, as they would be found
# from HOME. Neither has to exist (Arch's cargo needs no rustup).
cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}

root=$(mktemp -d "${TMPDIR:-/tmp}/x4-virtual.XXXXXX")
# Absolute, also under a relative TMPDIR: HOME and the links below must not depend on the current
# directory. Without CDPATH, which could send cd elsewhere and make it print the dir.
root=$(CDPATH='' cd -- "$root" && pwd)
# KWin's socket, in the session's runtime dir.
socket=x4-virtual-$$

# Whether process $1 runs with the session's XDG dirs. The session's path in the environment marks
# the processes of the session, and nothing else has it.
in_session() {
    [[ $1 != "$$" ]] && grep -qzxF -- "XDG_DATA_HOME=$root/data" "/proc/$1/environ" 2>/dev/null
}

# The pids of $@ that still run in the session. Each is checked again, because a pid that has
# ended may already belong to a new process, which must not be signalled.
still_in_session() {
    local pid
    for pid in "$@"; do
        if in_session "$pid"; then echo "$pid"; fi
    done
}

# Stops every process that still runs with the session's XDG dirs. The services the session's
# D-Bus starts (xdg-desktop-portal-kde, ksecretd) outlive their bus and their compositor and keep
# writing to the session's dirs.
stop_leftovers() {
    local environ pid pids=() alive=() _
    for environ in /proc/[0-9]*/environ; do
        pid=${environ#/proc/}
        pid=${pid%/environ}
        if in_session "$pid"; then pids+=("$pid"); fi
    done
    ((${#pids[@]} > 0)) || return 0
    mapfile -t alive < <(still_in_session "${pids[@]}")
    kill "${alive[@]}" 2>/dev/null || true
    for _ in {1..20}; do
        mapfile -t alive < <(still_in_session "${alive[@]}")
        ((${#alive[@]} > 0)) || return 0
        sleep 0.1
    done
    mapfile -t alive < <(still_in_session "${alive[@]}")
    ((${#alive[@]} == 0)) || kill -KILL "${alive[@]}" 2>/dev/null || true
}

# Unmounts what the session's services mounted under its dirs (the document portal and gvfs
# mount FUSE file systems in the runtime dir), so that the dirs can be removed.
unmount_leftovers() {
    local mount
    while read -r mount; do
        fusermount3 -uz "$mount" 2>/dev/null || fusermount -uz "$mount" 2>/dev/null || true
    done < <(awk -v root="$root/" 'index($5, root) == 1 { print $5 }' /proc/self/mountinfo)
}

cleanup() {
    stop_leftovers
    unmount_leftovers
    if [[ ${X4_VIRTUAL_KEEP:-} == 1 ]]; then
        echo "virtual-session: kept $root" >&2
    else
        rm -rf "$root"
    fi
}
trap cleanup EXIT

# A Unix socket's path holds at most 107 bytes, so under a long TMPDIR KWin could not create its
# socket. Say so here, before anything starts.
socket_path=$root/runtime/$socket
socket_bytes=$(printf '%s' "$socket_path" | wc -c)
if ((socket_bytes > 107)); then
    echo "virtual-session: KWin's socket path $socket_path would be $socket_bytes bytes," \
        "more than the 107 a Unix socket allows; set TMPDIR to a shorter directory" >&2
    exit 2
fi

# The session has XDG dirs of its own, and below a D-Bus session of its own, because the nested
# KWin and spectacle save their settings (kwinoutputconfig.json, kwinrc, kglobalshortcutsrc,
# spectaclerc) through them. A run with the owner's dirs and bus once rewrote the owner's files.
# Its runtime dir holds KWin's socket and whatever the session's services put there (the
# document portal's and gvfs's mounts), away from the owner's.
mkdir -p "$root/config" "$root/data" "$root/state" "$root/cache"
mkdir -m 0700 "$root/runtime"
# A HOME of its own too, because some programs write by fixed paths under HOME, not through the XDG
# dirs: KDE's update scripts, which the session's first Plasma start runs, once rewrote the owner's
# ~/.local/share/plasma-systemmonitor that way. Its usual dirs lead to the session's XDG dirs, so
# both ways of finding a file find the same one. Only HOME changes: a program that looks the home
# up in the passwd database, or follows an absolute path it inherits, still finds the owner's.
mkdir -p "$root/home/.local"
ln -s "$root/config" "$root/home/.config"
ln -s "$root/data" "$root/home/.local/share"
ln -s "$root/state" "$root/home/.local/state"
ln -s "$root/cache" "$root/home/.cache"
# Copies of the owner's font and KDE settings, so the window looks as it does on the desktop.
# Anything the session writes lands in the copies, never in the owner's files, even where those
# are symlinks: the copies follow the links (-L).
if [[ -d $owner_config/fontconfig ]]; then
    cp -rL "$owner_config/fontconfig" "$root/config/"
fi
if [[ -f $owner_config/kdeglobals ]]; then
    cp -L "$owner_config/kdeglobals" "$root/config/"
fi

# The command goes inside the session script, quoted word by word. KWin starts every positional
# argument as an app of its own and quits as soon as the --exit-with-session script ends, so
# `--exit-with-session <command> <arg>` would quit at once. The script takes back the caller's
# stdout and stderr from fds 3 and 4, which KWin passes on.
session=$root/session.sh
{
    printf '#!/usr/bin/env bash\n'
    printf 'exec 1>&3 2>&4 3>&- 4>&-\n'
    printf 'status=0\n'
    printf '%q ' "$@"
    printf '|| status=$?\n'
    printf 'echo "$status" >%q\n' "$root/status"
} >"$session"
chmod +x "$session"

# Without DISPLAY and XAUTHORITY, and with WAYLAND_DISPLAY naming KWin's socket from the start,
# the bus and every service it starts (the portal, ksecretd) know only the nested display: none of
# them can fall back to the X server of the owner's desktop, or open a window there.
kwin_status=0
env -u DISPLAY -u XAUTHORITY WAYLAND_DISPLAY="$socket" XDG_RUNTIME_DIR="$root/runtime" \
    XDG_CONFIG_HOME="$root/config" XDG_DATA_HOME="$root/data" XDG_STATE_HOME="$root/state" \
    XDG_CACHE_HOME="$root/cache" HOME="$root/home" CARGO_HOME="$cargo_home" \
    RUSTUP_HOME="$rustup_home" \
    dbus-run-session -- kwin_wayland --virtual --no-lockscreen --socket "$socket" \
    --width "$width" --height "$height" --exit-with-session "$session" \
    3>&1 4>&2 >"$root/kwin.log" 2>&1 || kwin_status=$?

if [[ ! -s $root/status ]]; then
    echo "virtual-session: the command did not run; KWin exited with $kwin_status. KWin's log:" >&2
    cat "$root/kwin.log" >&2
    exit 125
fi
exit "$(<"$root/status")"
