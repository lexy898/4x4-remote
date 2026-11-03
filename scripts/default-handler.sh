#!/usr/bin/env bash
# Makes 4x4 the default app for .torrent files and magnet links, or gives them back. Run by
# `just default-handler`, by the owner, after installing the package. Agents never run it:
# changing the default apps is the owner's.
#
# Usage: scripts/default-handler.sh [--undo] [--yes]
#     (none)   check that 4x4's desktop file is installed, ask for confirmation, then run
#              `xdg-mime default` for application/x-bittorrent and x-scheme-handler/magnet, which
#              writes the user's mimeapps.list; print what each type now opens with. Exits 1 when
#              the question is not answered with yes, when 4x4 is not installed, or when another
#              setting still outranks the choice (it names the types).
#     --undo   ask for confirmation, then remove exactly the entries `xdg-mime default` made for
#              4x4 from mimeapps.list and print what each type opens with now. It does not need 4x4
#              to be installed, so it works after uninstalling.
#     --yes    answer the question with yes, for the owner's own scripts and the tests.
#
# The question is asked on /dev/tty, so a pipe or a missing terminal can never answer it: without
# a terminal and without --yes the script refuses. It never runs as root (mimeapps.list is the
# user's), and it writes nothing but the user's mimeapps.list: the default is set by xdg-mime
# itself, the undo is a copy of the list without 4x4's lines, moved over it.
#
# What `xdg-mime default` writes (xdg-utils 1.1 and 1.2): in [Default Applications], a line
# `<type>=<desktop file>` for each type, replacing the line the type had; on KDE also, in
# [Added Associations], the desktop file first in the list of a type that has one or a bare line
# for a type that has none. The undo takes those lines (and that first place) out and leaves the
# rest of the file byte for byte, as it was before. A line of [Default Applications] that named
# another app is gone after the default (xdg-mime replaced it); the undo cannot bring it back:
# the types then open with what the system picks, and `xdg-mime default <app.desktop> <type>`
# pins an app again.
#
# Env: HOME, XDG_CONFIG_HOME (where mimeapps.list is), XDG_DATA_HOME and XDG_DATA_DIRS (where
# the desktop file is looked for), as for xdg-mime.
set -euo pipefail

app=io.github.lexy898.FourByFour.desktop
types=(application/x-bittorrent x-scheme-handler/magnet)

usage() {
    echo "usage: scripts/default-handler.sh [--undo] [--yes]" >&2
    exit 2
}

fail() {
    echo "default-handler: $*" >&2
    exit 1
}

undo=no
yes=no
while (($# > 0)); do
    case $1 in
        --undo) undo=yes ;;
        --yes) yes=yes ;;
        *) usage ;;
    esac
    shift
done

if ((EUID == 0)); then
    fail "do not run this as root: the default apps are the user's, not the system's"
fi
if ! command -v xdg-mime >/dev/null 2>&1; then
    echo "default-handler: xdg-mime not found; it comes with the package xdg-utils" >&2
    exit 127
fi

list=${XDG_CONFIG_HOME:-$HOME/.config}/mimeapps.list

# True when the desktop file of 4x4 is in an applications/ dir that desktops search.
installed() {
    local dirs dir
    IFS=: read -ra dirs <<<"${XDG_DATA_HOME:-$HOME/.local/share}:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
    for dir in "${dirs[@]}"; do
        if [[ -n $dir && -f $dir/applications/$app ]]; then
            return 0
        fi
    done
    return 1
}

# What <mime type> opens with, or "none".
opens_with() {
    local answer
    answer=$(xdg-mime query default "$1") || true
    echo "${answer:-none}"
}

# Prints what each of the types opens with, and puts the types that do not open with 4x4 in
# $other.
other=()
report() {
    local mime_type opens
    other=()
    for mime_type in "${types[@]}"; do
        opens=$(opens_with "$mime_type")
        echo "$mime_type: $opens"
        if [[ $opens != "$app" ]]; then
            other+=("$mime_type")
        fi
    done
}

# confirm <question>: asks on the terminal and returns only if the answer is yes.
confirm() {
    local answer=
    if [[ $yes == yes ]]; then
        return 0
    fi
    # Opening /dev/tty fails without a controlling terminal (cron, a service, setsid): that must
    # end in a refusal, never in an answer taken from stdin or assumed.
    if ! { exec 3<>/dev/tty; } 2>/dev/null; then
        fail "no terminal to ask on; run it in a terminal, or answer for yourself with --yes"
    fi
    printf '%s [y/N] ' "$1" >&3
    read -r -u 3 answer || true
    exec 3>&-
    case ${answer,,} in
        y | yes) ;;
        *) fail "not confirmed; nothing was changed" ;;
    esac
}

# is_ours <mime type>: whether the type is one of the types this script sets.
is_ours() {
    local mime_type
    for mime_type in "${types[@]}"; do
        if [[ $mime_type == "$1" ]]; then
            return 0
        fi
    done
    return 1
}

# strip_entries <in> <out>: copies the list without the lines `xdg-mime default` made for 4x4. A
# line is theirs only in the section xdg-mime puts it in and only for one of the types: in
# [Default Applications] `<type>=<app>` (with or without the closing ";"); in [Added Associations]
# `<type>=<app>`, or `<type>=<app>;<others>` which gets back `<type>=<others>`. The last line keeps
# or lacks its newline as it had.
strip_entries() {
    local line section= mime_type value rest newline
    while true; do
        if IFS= read -r line; then
            newline=$'\n'
        elif [[ -n $line ]]; then
            newline=
        else
            break
        fi
        if [[ $line == '['* ]]; then
            section=$line
        elif [[ $line == *=* ]] && is_ours "${line%%=*}"; then
            mime_type=${line%%=*}
            value=${line#*=}
            if [[ $section == '[Default Applications]' && ($value == "$app" || $value == "$app;") ]]; then
                continue
            fi
            if [[ $section == '[Added Associations]' ]]; then
                if [[ $value == "$app" ]]; then
                    continue
                fi
                if [[ $value == "$app;"* ]]; then
                    rest=${value#"$app;"}
                    if [[ -z $rest ]]; then
                        continue
                    fi
                    line=$mime_type=$rest
                fi
            fi
        fi
        printf '%s%s' "$line" "$newline"
    done <"$1" >"$2"
}

if [[ $undo == yes ]]; then
    confirm "Give .torrent files and magnet links back? This removes the entries of 4x4 from $list.
Continue?"
    # xdg-mime resolves a symlinked list to its target, and so does the undo: moving a copy over the
    # link would replace the link.
    target=$(readlink -f -- "$list")
    if [[ ! -f $target ]]; then
        echo "nothing to undo: $list does not exist"
    else
        copy=$(mktemp "$target.XXXXXX")
        trap 'rm -f -- "$copy"' EXIT
        strip_entries "$target" "$copy"
        if cmp -s -- "$target" "$copy"; then
            echo "nothing to undo: $list has no entry of 4x4"
        else
            chmod --reference="$target" -- "$copy"
            mv -f -- "$copy" "$target"
            echo "removed the entries of 4x4 from $list"
        fi
    fi
    report
    exit 0
fi

installed || fail "$app is not installed in any applications/ dir of XDG_DATA_HOME or XDG_DATA_DIRS; install the package first (see the README)"
question="Make 4x4 the default app for .torrent files and magnet links?"
for mime_type in "${types[@]}"; do
    question+=$'\n'"  $mime_type: now $(opens_with "$mime_type")"
done
question+=$'\n'"This changes $list; \`just default-handler --undo\` takes it back."$'\nContinue?'
confirm "$question"

xdg-mime default "$app" "${types[@]}"
report

# A list for the current desktop (kde-mimeapps.list) or a missing executable outranks what was
# just written; saying yes to the question and then not getting the answer must not pass silently.
if ((${#other[@]} > 0)); then
    fail "${other[*]} still opens with another app, although $list names 4x4: a mimeapps.list for your desktop (such as kde-mimeapps.list) or a missing 4x4 executable on PATH outranks it. The undo takes the entries out again."
fi
