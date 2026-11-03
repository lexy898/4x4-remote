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
#     --undo   ask for confirmation, then remove the entries that name 4x4 for the two types from
#              mimeapps.list and print what each type opens with now. It does not need 4x4 to be
#              installed, so it works after uninstalling.
#     --yes    answer the question with yes, for the owner's own scripts and the tests.
#
# The question is asked on /dev/tty, so a pipe or a missing terminal can never answer it: without
# a terminal and without --yes the script refuses. It never runs as root (mimeapps.list is the
# user's). The only file it changes is the user's mimeapps.list: the default is set by xdg-mime
# itself (on KDE xdg-mime then also runs kbuildsycoca6, which rebuilds KDE's cache of installed
# applications in ~/.cache); the undo is a copy of the list without 4x4's entries, moved over it.
#
# What `xdg-mime default` writes (xdg-utils 1.1 and 1.2): under [Default Applications], a line
# `<type>=<desktop file>` for each type, replacing the line the type had: two lines. On KDE 6
# (xdg-utils 1.2) it also writes under [Added Associations], the desktop file first in the list of
# a type that has one, or a bare line for a type that has none: two more, four lines in all. A
# section the list lacks is added at the end of the file (by the generic writer after a blank line).
#
# What the undo removes: in those two sections, the entries that name 4x4 for the two types,
# whoever made them (one the user made by hand goes too), and 4x4 at the front of an
# [Added Associations] list. A section header it leaves empty as the last section of the file is
# taken out too, with the blank line before it: a header the command added is always the last
# section. The rest of the file stays byte for byte, line endings and file mode included. An empty
# header that stood in the middle of the file stays. A line under [Default Applications] that named
# another app for these types is gone after the default (xdg-mime replaced it) and the undo cannot
# bring it back: the types then open with what the system picks, and
# `xdg-mime default <app.desktop> <type>` pins an app again.
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

# is_blank <line>: whether the line has nothing but white space (a CR of a CRLF list included).
is_blank() {
    [[ -z ${1//[[:space:]]/} ]]
}

# strip_entries <in> <out>: copies the list without the entries `xdg-mime default` made for 4x4.
# An entry is theirs only in the section xdg-mime puts it in and only for one of the two types:
# under [Default Applications] `<type>=<app>` (with or without the closing ";"); under [Added
# Associations] `<type>=<app>`, or `<type>=<app>;<others>`, which gets back `<type>=<others>`. A
# header is recognised by its beginning, as xdg-mime does (a CR or spaces may follow it), and
# trailing white space after a value is ignored. A section that was left empty as the last one of
# the file goes, with a blank line before it. The last line keeps or lacks its newline as it had.
strip_entries() {
    local line section='' mime_type value trimmed rest newline header=-1 i drop count
    local -a out=() ends=() removed=()
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
            out+=("$line")
            ends+=("$newline")
            header=$((${#out[@]} - 1))
            removed[header]=0
            continue
        elif [[ $line == *=* ]] && is_ours "${line%%=*}"; then
            mime_type=${line%%=*}
            value=${line#*=}
            trimmed=${value%"${value##*[![:space:]]}"}
            if [[ $section == '[Default Applications]'* && ($trimmed == "$app" || $trimmed == "$app;") ]]; then
                removed[header]=$((removed[header] + 1))
                continue
            fi
            if [[ $section == '[Added Associations]'* ]]; then
                if [[ $trimmed == "$app" ]]; then
                    removed[header]=$((removed[header] + 1))
                    continue
                fi
                if [[ $value == "$app;"* ]]; then
                    rest=${value#"$app;"}
                    if is_blank "$rest"; then
                        removed[header]=$((removed[header] + 1))
                        continue
                    fi
                    line=$mime_type=$rest
                fi
            fi
        fi
        out+=("$line")
        ends+=("$newline")
    done <"$1"
    # The last section, if the removal left nothing in it, was added by the command (or stood
    # there empty before: the one case this cannot tell apart); then the one before it may be too.
    while ((${#out[@]} > 0)); do
        header=-1
        for ((i = ${#out[@]} - 1; i >= 0; i--)); do
            if [[ ${out[i]} == '['* ]]; then
                header=$i
                break
            fi
        done
        if ((header < 0 || ${removed[header]:-0} == 0)); then
            break
        fi
        for ((i = header + 1; i < ${#out[@]}; i++)); do
            if ! is_blank "${out[i]}"; then
                break 2
            fi
        done
        drop=$header
        count=1
        if ((header > 0)) && is_blank "${out[header - 1]}"; then
            drop=$((header - 1))
            count=2
        fi
        out=("${out[@]:0:drop}" "${out[@]:drop+count}")
        ends=("${ends[@]:0:drop}" "${ends[@]:drop+count}")
        unset 'removed[header]'
    done
    for ((i = 0; i < ${#out[@]}; i++)); do
        printf '%s%s' "${out[i]}" "${ends[i]}"
    done >"$2"
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
