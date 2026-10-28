#!/usr/bin/env bash
# PreToolUse hook (Bash): refuse commands that would bypass the repository's gates.
#
# Blocked (exit 2, the reason on stderr goes to the agent):
#   - `git commit --no-verify` or a short-flag cluster containing `n` (`-n`, `-an`, ...)
#   - `git push --no-verify`
#   - any change of core.hooksPath except exactly `git config core.hooksPath .githooks`
#     (what `just setup` runs), including `git -c core.hooksPath=...`
#   - writes to or removal of the Stop hook state in target/.agent-hooks
#     (`>`/`>>` redirections, tee, rm, mv, cp, touch and similar)
# Everything else exits 0 silently. Mentions inside quoted arguments, heredoc bodies and
# read-only commands (`grep no-verify`, `cat target/.agent-hooks/last-green`) are allowed:
# the command is split into words and simple commands roughly the way the shell does it,
# and only the command words are judged.
#
# This is a guard against mistakes and shortcuts, not a sandbox: a determined workaround
# (an alias, a script file, an encoded string) is not detected.
set -euo pipefail
shopt -s extglob
export LC_ALL=C

advice='Fix the failing check instead; if it cannot be fixed, stop and tell the owner (AGENTS.md, Definition of Done).'
max_depth=6

block() { # <reason>
    echo "guard-bash: blocked: $1" >&2
    echo "$advice" >&2
    exit 2
}

# --- Lexer --------------------------------------------------------------------------------
# lex <text> fills LEX_TYPES / LEX_VALUES with words (W), operators that end a simple
# command (O) and redirection operators (R), with quotes and escapes removed from words.
# Command substitutions found inside double quotes go to LEX_NESTED for a separate pass.
#
# Bash string operations cost time proportional to the string length, so the lexer works on
# a small window (`buf`) in front of the unread text (`tail`) to stay fast on long commands.

re_blank=$'^[ \t\r\v\f]+'
re_op='^(&&|\|\||;;|\|&|[;&|()`])'
re_redir='^(>>|>\||>&|>|<<<|<<-|<<|<>|<&|<|&>>|&>)'
re_squote="^'([^']*)'"
re_ansi="^[\$]'(([^'\\\\]|\\\\.)*)'"
re_dquote='^"(([^"\\]|\\.)*)"'
re_run='^[^][:space:]'\''"\;&|()<>`$]+'

LEX_TYPES=()
LEX_VALUES=()
LEX_NESTED=()

lex() { # <text>
    local buf='' tail=$1 word='' have_word=0 want_delim=0 strip_tabs=0 c text prefix entry delim
    local -a heredocs=()
    LEX_TYPES=()
    LEX_VALUES=()
    LEX_NESTED=()

    while :; do
        if ((${#buf} < 256)) && [[ -n $tail ]]; then
            buf+=${tail:0:1024}
            tail=${tail:1024}
        fi
        [[ -n $buf ]] || break
        c=${buf:0:1}
        case $c in
            ' ' | $'\t' | $'\r' | $'\v' | $'\f')
                lex_finish_word
                [[ $buf =~ $re_blank ]]
                buf=${buf:${#BASH_REMATCH[0]}}
                ;;
            $'\n')
                lex_finish_word
                LEX_TYPES+=(O)
                LEX_VALUES+=($'\n')
                buf=${buf:1}
                if ((${#heredocs[@]} > 0)); then
                    # Heredoc bodies start after the line that declared them; they are data.
                    text=$'\n'$buf$tail
                    for entry in "${heredocs[@]}"; do
                        strip_tabs=${entry%%:*}
                        delim=${entry#*:}
                        if ((strip_tabs)); then
                            prefix=${text%%$'\n'*($'\t')"$delim"$'\n'*}
                        else
                            prefix=${text%%$'\n'"$delim"$'\n'*}
                        fi
                        if [[ $prefix == "$text" ]]; then
                            text='' # unterminated, or terminated by the end of the text
                        else
                            text=${text:${#prefix}+1}
                            text=$'\n'${text#*$'\n'}
                        fi
                    done
                    buf=''
                    tail=${text:1}
                    heredocs=()
                fi
                ;;
            '#' | '$' | "'" | '"' | '\')
                lex_quoted_or_special
                ;;
            '<' | '>' | '&')
                if [[ $c == '&' && $buf != '&>'* ]]; then
                    lex_operator
                    continue
                fi
                if ((have_word)); then
                    if [[ $word =~ ^[0-9]+$ ]]; then
                        # `2>file`: the digits name a file descriptor, not an argument.
                        word=''
                        have_word=0
                    else
                        lex_finish_word
                    fi
                fi
                [[ $buf =~ $re_redir ]]
                LEX_TYPES+=(R)
                LEX_VALUES+=("${BASH_REMATCH[0]}")
                case ${BASH_REMATCH[0]} in
                    '<<-') want_delim=1 strip_tabs=1 ;;
                    '<<') want_delim=1 strip_tabs=0 ;;
                esac
                buf=${buf:${#BASH_REMATCH[0]}}
                ;;
            ';' | '|' | '(' | ')' | '`')
                lex_operator
                ;;
            *)
                if [[ $buf =~ $re_run ]]; then
                    word+=${BASH_REMATCH[0]}
                    buf=${buf:${#BASH_REMATCH[0]}}
                else
                    word+=$c # any other byte the shell treats as a word character
                    buf=${buf:1}
                fi
                have_word=1
                ;;
        esac
    done
    lex_finish_word
}

# The lex_* helpers run inside lex and share its local variables.

lex_finish_word() {
    if ((have_word)); then
        LEX_TYPES+=(W)
        LEX_VALUES+=("$word")
        if ((want_delim)); then
            heredocs+=("$strip_tabs:$word")
            want_delim=0
        fi
    fi
    word=''
    have_word=0
}

lex_operator() {
    lex_finish_word
    [[ $buf =~ $re_op ]]
    LEX_TYPES+=(O)
    LEX_VALUES+=("${BASH_REMATCH[0]}")
    buf=${buf:${#BASH_REMATCH[0]}}
}

# Matches <regex> at the start of the window, looking into the unread text when a quoted
# piece is longer than the window. On success BASH_REMATCH holds the match and the window
# starts with it.
lex_match() { # <regex>
    [[ $buf =~ $1 ]] && return 0
    [[ -n $tail ]] || return 1
    buf+=$tail
    tail=''
    [[ $buf =~ $1 ]] || return 1
    tail=${buf:${#BASH_REMATCH[0]}}
    buf=${buf:0:${#BASH_REMATCH[0]}}
}

lex_quoted_or_special() {
    local consumed=1
    case $c in
        '#')
            if ((have_word)); then
                word+=$c
                buf=${buf:1}
            else
                # A comment runs to the end of the line.
                text=$buf$tail
                prefix=${text%%$'\n'*}
                buf=''
                tail=${text:${#prefix}}
            fi
            return
            ;;
        '$')
            if [[ $buf == '$('* ]]; then
                lex_finish_word
                LEX_TYPES+=(O)
                LEX_VALUES+=('(')
                buf=${buf:2}
                return
            fi
            if [[ $buf == "\$'"* ]] && lex_match "$re_ansi"; then
                word+=${BASH_REMATCH[1]}
                consumed=${#BASH_REMATCH[0]}
            else
                word+=$c
            fi
            ;;
        "'")
            if lex_match "$re_squote"; then
                word+=${BASH_REMATCH[1]}
                consumed=${#BASH_REMATCH[0]}
            else
                word+=$c # unterminated: keep it literally
            fi
            ;;
        '"')
            if lex_match "$re_dquote"; then
                consumed=${#BASH_REMATCH[0]}
                text=${BASH_REMATCH[1]}
                lex_collect_substitutions "$text"
                text=${text//\\\"/\"}
                text=${text//\\\\/\\}
                word+=$text
            else
                word+=$c
            fi
            ;;
        '\')
            if ((${#buf} >= 2)); then
                [[ ${buf:1:1} == $'\n' ]] || word+=${buf:1:1}
                consumed=2
            else
                word+=$c
            fi
            ;;
    esac
    buf=${buf:consumed}
    have_word=1
}

# `$(...)` and backticks inside double quotes still run commands; queue their text.
lex_collect_substitutions() { # <double-quoted text>
    local text=$1 before
    while :; do
        before=${text%%'$('*}
        ((${#before} < ${#text})) || break
        text=${text:${#before}+2}
        LEX_NESTED+=("$text")
    done
    text=$1
    while :; do
        before=${text%%'`'*}
        ((${#before} < ${#text})) || break
        text=${text:${#before}+1}
        LEX_NESTED+=("${text%%'`'*}")
        before=${text%%'`'*}
        text=${text:${#before}}
        [[ -n $text ]] && text=${text:1}
    done
}

# --- Judging simple commands --------------------------------------------------------------

is_assignment() { # <word>
    [[ $1 =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]
}

check_assignment() { # <NAME=value>
    if [[ $1 == GIT_CONFIG* && ${1,,} == *hookspath* ]]; then
        block "setting core.hooksPath through GIT_CONFIG_* disables the git hooks"
    fi
}

touches_hook_state() { # <word...>
    local word
    for word in "$@"; do
        [[ $word == *.agent-hooks || $word == *.agent-hooks/* ]] && return 0
    done
    return 1
}

check_git_commit() { # <args after `commit`...>
    local arg name cluster char option skip_next=0
    for arg in "$@"; do
        if ((skip_next)); then
            skip_next=0
            continue
        fi
        case $arg in
            --) return ;;
            --*)
                # git accepts unambiguous abbreviations of long options.
                if [[ $arg == --no-ver* && --no-verify == "$arg"* ]]; then
                    block "git commit --no-verify skips the pre-commit gate"
                fi
                name=${arg#--}
                if [[ $arg != *=* && ${#name} -ge 2 ]]; then
                    for option in author date message file reuse-message reedit-message fixup squash \
                        template cleanup trailer pathspec-from-file; do
                        if [[ $option == "$name"* ]]; then
                            skip_next=1
                            break
                        fi
                    done
                fi
                ;;
            -?*)
                cluster=${arg#-}
                while [[ -n $cluster ]]; do
                    char=${cluster:0:1}
                    cluster=${cluster:1}
                    case $char in
                        n) block "git commit -n (--no-verify) skips the pre-commit gate" ;;
                        [mFCct])
                            [[ -n $cluster ]] || skip_next=1
                            break
                            ;;
                        [Su]) break ;; # optional value attached to the flag
                    esac
                done
                ;;
        esac
    done
}

check_git_push() { # <args after `push`...>
    local arg
    for arg in "$@"; do
        [[ $arg == -- ]] && return
        if [[ $arg == --no-ver* && --no-verify == "$arg"* ]]; then
            block "git push --no-verify skips the push hooks"
        fi
    done
}

check_git_config() { # <args after `config`...>
    local arg lower mentions=0 writes=0 reads=0 positional=0 skip_next=0 previous=''
    if (($# == 2)) && [[ ${1,,} == core.hookspath && $2 == .githooks ]]; then
        return # exactly what `just setup` runs
    fi
    for arg in "$@"; do
        lower=${arg,,}
        if [[ $lower == core.hookspath || $lower == core.hookspath=* ]]; then
            mentions=1
        fi
        if [[ $previous == ?(--)re@(move|name)-section && $lower == core ]]; then
            block "removing or renaming the [core] section of the git config drops core.hooksPath"
        fi
        previous=$lower
        if ((skip_next)); then
            skip_next=0
            continue
        fi
        case $arg in
            --unset | --unset-all | --replace-all | --add | --edit | -e | set | unset | edit) writes=1 ;;
            --get | --get-all | --get-regexp | --get-urlmatch | --list | -l | get | list) reads=1 ;;
            --file | -f | --blob | --type | --default | --comment | --value) skip_next=1 ;;
            -*) ;;
            *) positional=$((positional + 1)) ;;
        esac
    done
    if ((mentions && (writes || (!reads && positional >= 2)))); then
        block "changing core.hooksPath disables the git hooks; only \`git config core.hooksPath .githooks\` (just setup) is allowed"
    fi
}

check_git() { # <args after `git`...>
    local -a args=("$@")
    local i=0 n=$# arg value
    while ((i < n)); do
        arg=${args[i]}
        case $arg in
            -c | --config-env)
                value=${args[i + 1]:-}
                [[ $arg == --config-env ]] && value=${value%=*}
                if [[ ${value,,} == core.hookspath || ${value,,} == core.hookspath=* ]]; then
                    block "git $arg core.hooksPath=... disables the git hooks"
                fi
                i=$((i + 2))
                ;;
            --config-env=*)
                value=${arg#--config-env=}
                if [[ ${value,,} == core.hookspath=* ]]; then
                    block "git --config-env core.hooksPath=... disables the git hooks"
                fi
                i=$((i + 1))
                ;;
            -C | --git-dir | --work-tree | --namespace | --exec-path | --super-prefix | --attr-source | --list-cmds)
                i=$((i + 2))
                ;;
            -*) i=$((i + 1)) ;;
            *) break ;;
        esac
    done
    ((i < n)) || return 0
    case ${args[i]} in
        commit) check_git_commit "${args[@]:i+1}" ;;
        push) check_git_push "${args[@]:i+1}" ;;
        config) check_git_config "${args[@]:i+1}" ;;
    esac
}

# Drop wrappers that run their arguments as a command (`env X=1 git ...`, `timeout 5 git ...`).
# Sets ARGV to the remaining words.
strip_wrappers() { # <word...>
    ARGV=("$@")
    local base
    while ((${#ARGV[@]} > 0)); do
        if is_assignment "${ARGV[0]}"; then
            check_assignment "${ARGV[0]}"
            ARGV=("${ARGV[@]:1}")
            continue
        fi
        base=${ARGV[0]##*/}
        case $base in
            '!' | '{' | '}' | if | then | elif | else | do | while | until | fi | done | time | command | builtin | exec | nohup)
                ARGV=("${ARGV[@]:1}")
                ;;
            env | nice | timeout | stdbuf | xargs | sudo | doas)
                ARGV=("${ARGV[@]:1}")
                while ((${#ARGV[@]} > 0)); do
                    case ${ARGV[0]} in
                        -u | -C | -S | -n | -s | -k | -i | -o | -e | -I | -L | -P | -d | -a | -E | -g)
                            [[ $base == env && ${ARGV[0]} == -i ]] && { ARGV=("${ARGV[@]:1}"); continue; }
                            ARGV=("${ARGV[@]:2}")
                            ;;
                        -*) ARGV=("${ARGV[@]:1}") ;;
                        *)
                            if is_assignment "${ARGV[0]}"; then
                                check_assignment "${ARGV[0]}"
                                ARGV=("${ARGV[@]:1}")
                            else
                                break
                            fi
                            ;;
                    esac
                done
                if [[ $base == timeout && ${#ARGV[@]} -gt 0 ]]; then
                    ARGV=("${ARGV[@]:1}") # the duration
                fi
                ;;
            *) return ;;
        esac
    done
}

judge_simple() { # <depth> <word...> (redirections are judged by the caller)
    local depth=$1
    shift
    strip_wrappers "$@"
    ((${#ARGV[@]} > 0)) || return 0
    local -a argv=("${ARGV[@]}")
    local base=${argv[0]##*/} arg i
    case $base in
        git) check_git "${argv[@]:1}" ;;
        rm | rmdir | unlink | mv | cp | touch | tee | truncate | ln | install | shred)
            if touches_hook_state "${argv[@]:1}"; then
                block "$base on target/.agent-hooks would fake or reset the Stop hook state"
            fi
            ;;
        sed | perl)
            for arg in "${argv[@]:1}"; do
                if [[ $arg == -i* || $arg == --in-place* || $arg =~ ^-[A-Za-z]*i ]]; then
                    if touches_hook_state "${argv[@]:1}"; then
                        block "$base -i on target/.agent-hooks would fake the Stop hook state"
                    fi
                fi
            done
            ;;
        export)
            for arg in "${argv[@]:1}"; do
                check_assignment "$arg"
            done
            ;;
        bash | sh | zsh | dash | ksh)
            for ((i = 1; i < ${#argv[@]}; i++)); do
                if [[ ${argv[i]} =~ ^-[A-Za-z]*c[A-Za-z]*$ ]]; then
                    judge_text "$((depth + 1))" "${argv[i + 1]:-}"
                    break
                fi
                [[ ${argv[i]} == -* ]] || break
            done
            ;;
        eval) judge_text "$((depth + 1))" "${argv[*]:1}" ;;
    esac
}

judge_text() { # <depth> <shell text>
    local depth=$1
    ((depth <= max_depth)) || return 0
    lex "$2"
    local -a types=("${LEX_TYPES[@]}") values=("${LEX_VALUES[@]}") nested=("${LEX_NESTED[@]}")
    local -a words=()
    local i n=${#types[@]} redirect=''
    for ((i = 0; i <= n; i++)); do
        if ((i == n)) || [[ ${types[i]} == O ]]; then
            if ((${#words[@]} > 0)); then
                judge_simple "$depth" "${words[@]}"
            fi
            words=()
            redirect=''
        elif [[ ${types[i]} == R ]]; then
            redirect=${values[i]}
        elif [[ -n $redirect ]]; then
            if [[ $redirect == *'>'* ]] && touches_hook_state "${values[i]}"; then
                block "redirecting output to target/.agent-hooks would fake the Stop hook state"
            fi
            redirect=''
        else
            words+=("${values[i]}")
        fi
    done
    local text
    for text in "${nested[@]}"; do
        judge_text "$((depth + 1))" "$text"
    done
}

command=$(jq -r '.tool_input.command // empty')
[[ -n $command ]] || exit 0
judge_text 0 "$command"
exit 0
