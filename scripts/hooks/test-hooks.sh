#!/usr/bin/env bash
# Tests for the agent and git hook scripts. Every case works in a throwaway
# directory under a temp root; the real repository and its target/.agent-hooks
# state are never touched. Prints "ok <case>" per case, exits 1 on the first failure.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
format_hook=$root/scripts/hooks/format-rust-file.sh
stop_hook=$root/scripts/hooks/stop-check.sh
check_tools=$root/scripts/check-tools.sh

# Do not leak a surrounding git hook or agent session into the temp repos.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE CLAUDE_PROJECT_DIR X4_STOP_CHECK_CMD

tmp_root=$(mktemp -d)
trap 'rm -rf "$tmp_root"' EXIT

case_name=
status=0
stdout=
stderr=

fail() {
    echo "FAIL $case_name: $*" >&2
    exit 1
}

new_repo() {
    local dir
    dir=$(mktemp -d "$tmp_root/repo.XXXXXX")
    git -C "$dir" init -q
    echo "$dir"
}

# run_with_input <stdin-text> <command...>: sets $status, $stdout, $stderr.
run_with_input() {
    local input=$1
    shift
    status=0
    "$@" <<<"$input" >"$tmp_root/stdout" 2>"$tmp_root/stderr" || status=$?
    stdout=$(<"$tmp_root/stdout")
    stderr=$(<"$tmp_root/stderr")
}

expect_status() {
    [[ $status -eq $1 ]] || fail "expected exit $1, got $status (stderr: $stderr)"
}

expect_contains() { # <needle> <haystack> <what>
    [[ $2 == *"$1"* ]] || fail "expected $3 to contain '$1', got: $2"
}

run_format() { # <file>
    local json
    json=$(jq -n --arg path "$1" '{tool_input: {file_path: $path}}')
    run_with_input "$json" "$format_hook"
}

# The stop-hook cases share one repo on purpose: they exercise the state kept
# between consecutive stops (last green fingerprint, block counter).
stop_dir=

run_stop() { # <check-command>
    run_with_input '{"hook_event_name":"Stop","stop_hook_active":false}' \
        env CLAUDE_PROJECT_DIR="$stop_dir" X4_STOP_CHECK_CMD="$1" "$stop_hook"
}

test_format_hook_formats_rust_file() {
    local dir
    dir=$(new_repo)
    printf 'fn  main( ){}' >"$dir/a.rs"
    run_format "$dir/a.rs"
    expect_status 0
    [[ $(<"$dir/a.rs") == 'fn main() {}' ]] || fail "a.rs was not formatted: $(<"$dir/a.rs")"
}

test_format_hook_ignores_non_rust() {
    local dir
    dir=$(new_repo)
    printf 'fn  main( ){}' >"$dir/a.md"
    run_format "$dir/a.md"
    expect_status 0
    [[ $(<"$dir/a.md") == 'fn  main( ){}' ]] || fail "a.md was modified"
}

test_format_hook_reports_syntax_error() {
    local dir
    dir=$(new_repo)
    printf 'fn main( {' >"$dir/a.rs"
    run_format "$dir/a.rs"
    expect_status 2
    [[ -n $stderr ]] || fail "expected the rustfmt error on stderr"
}

test_stop_hook_runs_and_records_green() {
    stop_dir=$(new_repo)
    printf 'fn main() {}\n' >"$stop_dir/b.rs"
    run_stop true
    expect_status 0
    [[ -s $stop_dir/target/.agent-hooks/last-green ]] || fail "fingerprint file was not written"
}

test_stop_hook_skips_unchanged() {
    run_stop false
    expect_status 0
}

test_stop_hook_blocks_on_failure() {
    printf 'fn main() { }\n' >"$stop_dir/b.rs"
    run_stop "echo boom; false"
    expect_status 2
    expect_contains boom "$stderr" stderr
}

test_stop_hook_gives_up_after_three() {
    printf 'fn main() { let _x = 1; }\n' >"$stop_dir/b.rs"
    local message='4x4 checks are still red after 3 blocked stops — tell the user before finishing'
    run_stop false
    expect_status 2
    [[ -z $stdout ]] || fail "blocking run 1 must not print to stdout, got: $stdout"
    run_stop false
    expect_status 2
    [[ -z $stdout ]] || fail "blocking run 2 must not print to stdout, got: $stdout"
    run_stop false
    expect_status 0
    expect_contains "giving up" "$stderr" stderr
    # Exit 0 hides stderr from the user, so the warning must also be a systemMessage.
    jq -es --arg m "$message" 'length == 1 and .[0].systemMessage == $m' <<<"$stdout" >/dev/null ||
        fail "expected stdout to be exactly one systemMessage JSON object, got: $stdout"
}

test_stop_hook_ignores_non_rust_change() {
    run_stop true # the previous case left the tree red; go green first
    expect_status 0
    echo 'notes' >"$stop_dir/notes.md"
    run_stop false
    expect_status 0
}

# A PATH holding stub executables for every required tool except those given.
stub_path_without() { # <missing-tool...>
    local bin tool skip missing
    bin=$(mktemp -d "$tmp_root/bin.XXXXXX")
    for tool in cargo rustfmt just cargo-nextest cargo-deny cargo-machete taplo typos jq; do
        skip=0
        for missing in "$@"; do
            [[ $tool == "$missing" ]] && skip=1
        done
        if ((skip == 0)); then
            : >"$bin/$tool"
            chmod +x "$bin/$tool"
        fi
    done
    echo "$bin"
}

test_check_tools_names_missing_tool() {
    local bin
    bin=$(stub_path_without typos)
    run_with_input '' env PATH="$bin" "$BASH" "$check_tools"
    expect_status 1
    expect_contains "missing: typos" "$stdout$stderr" output
    expect_contains "sudo pacman -S" "$stdout$stderr" output
}

test_check_tools_passes_when_all_present() {
    local bin
    bin=$(stub_path_without)
    run_with_input '' env PATH="$bin" "$BASH" "$check_tools"
    expect_status 0
}

cases=(
    format_hook_formats_rust_file
    format_hook_ignores_non_rust
    format_hook_reports_syntax_error
    stop_hook_runs_and_records_green
    stop_hook_skips_unchanged
    stop_hook_blocks_on_failure
    stop_hook_gives_up_after_three
    stop_hook_ignores_non_rust_change
    check_tools_names_missing_tool
    check_tools_passes_when_all_present
)

for case_name in "${cases[@]}"; do
    "test_$case_name"
    echo "ok $case_name"
done
