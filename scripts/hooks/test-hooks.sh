#!/usr/bin/env bash
# Tests for the agent and git hook scripts and the repository's helper scripts.
# Every case works in a throwaway directory under a temp root; the real repository
# and its target/.agent-hooks state are never touched. Prints "ok <case>" per case,
# exits 1 on the first failure.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
format_hook=$root/scripts/hooks/format-rust-file.sh
stop_hook=$root/scripts/hooks/stop-check.sh
guard_hook=$root/scripts/hooks/guard-bash.sh
check_tools=$root/scripts/check-tools.sh
lint_policy=$root/scripts/lint-policy.sh

# Do not leak a surrounding git hook or agent session into the temp repos.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE CLAUDE_PROJECT_DIR X4_STOP_CHECK_CMD X4_TOOLCHAIN_ID_CMD

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

run_format() { # <file> <project-dir>
    local json
    json=$(jq -n --arg path "$1" '{tool_input: {file_path: $path}}')
    run_with_input "$json" env CLAUDE_PROJECT_DIR="$2" "$format_hook"
}

test_format_hook_formats_rust_file() {
    local dir
    dir=$(new_repo)
    printf 'fn  main( ){}' >"$dir/a.rs"
    run_format "$dir/a.rs" "$dir"
    expect_status 0
    [[ $(<"$dir/a.rs") == 'fn main() {}' ]] || fail "a.rs was not formatted: $(<"$dir/a.rs")"
}

test_format_hook_ignores_non_rust() {
    local dir
    dir=$(new_repo)
    printf 'fn  main( ){}' >"$dir/a.md"
    run_format "$dir/a.md" "$dir"
    expect_status 0
    [[ $(<"$dir/a.md") == 'fn  main( ){}' ]] || fail "a.md was modified"
}

test_format_hook_ignores_files_outside_project() {
    local project outside
    project=$(new_repo)
    outside=$(new_repo)
    printf 'fn  main( ){}' >"$outside/a.rs"
    run_format "$outside/a.rs" "$project"
    expect_status 0
    [[ $(<"$outside/a.rs") == 'fn  main( ){}' ]] || fail "a file outside the project was formatted"
}

test_format_hook_reports_syntax_error() {
    local dir
    dir=$(new_repo)
    printf 'fn main( {' >"$dir/a.rs"
    run_format "$dir/a.rs" "$dir"
    expect_status 2
    expect_contains "rustfmt failed on $dir/a.rs" "$stderr" stderr
    # A missing module file does not hide a syntax error.
    printf 'mod missing;\nfn main( {' >"$dir/b.rs"
    run_format "$dir/b.rs" "$dir"
    expect_status 2
    expect_contains "unclosed delimiter" "$stderr" stderr
}

test_format_hook_tolerates_unresolved_module() {
    local dir
    dir=$(new_repo)
    printf 'mod missing;\nfn  main( ){}\n' >"$dir/a.rs"
    run_format "$dir/a.rs" "$dir"
    expect_status 0
    [[ -z $stderr ]] || fail "expected no complaint, got: $stderr"
    [[ $(<"$dir/a.rs") == $'mod missing;\nfn  main( ){}' ]] || fail "a.rs must be left as it is"
}

# The stop-hook cases share one repo on purpose: they exercise the state kept
# between consecutive stops (last green fingerprint, block counter).
stop_dir=
toolchain_id=toolchain-1
check_ran=$tmp_root/check-ran

run_stop() { # <check-command>
    run_with_input '{"hook_event_name":"Stop","stop_hook_active":false}' \
        env CLAUDE_PROJECT_DIR="$stop_dir" X4_STOP_CHECK_CMD="$1" \
        X4_TOOLCHAIN_ID_CMD="echo $toolchain_id" "$stop_hook"
}

# A green check that leaves a marker, so a case can tell whether it ran or was skipped.
run_stop_green() {
    rm -f "$check_ran"
    run_stop "touch $(printf %q "$check_ran")"
    expect_status 0
}

expect_check_ran() {
    [[ -e $check_ran ]] || fail "expected the check to run"
}

expect_check_skipped() {
    [[ ! -e $check_ran ]] || fail "expected the check to be skipped"
}

test_stop_hook_runs_and_records_green() {
    stop_dir=$(new_repo)
    # Like the real repository: the hook state under target/ is ignored.
    printf '/target/\n' >"$stop_dir/.gitignore"
    printf 'fn main() {}\n' >"$stop_dir/b.rs"
    run_stop_green
    expect_check_ran
    [[ -s $stop_dir/target/.agent-hooks/last-green ]] || fail "fingerprint file was not written"
}

test_stop_hook_skips_unchanged() {
    run_stop_green
    expect_check_skipped
}

test_stop_hook_blocks_on_failure() {
    printf 'fn main() { }\n' >"$stop_dir/b.rs"
    run_stop "echo boom; false"
    expect_status 2
    expect_contains "echo boom; false failed — fix before finishing:" "$stderr" stderr
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

test_stop_hook_green_clears_blocks() {
    printf 'fn main() { let _y = 2; }\n' >"$stop_dir/b.rs"
    run_stop false
    expect_status 2
    [[ -f $stop_dir/target/.agent-hooks/blocks ]] || fail "a red run must record the block count"
    run_stop_green
    expect_check_ran
    [[ ! -e $stop_dir/target/.agent-hooks/blocks ]] || fail "a green run must clear the block count"
}

test_stop_hook_skips_markdown_change() {
    echo 'notes' >"$stop_dir/notes.md"
    echo 'license text' >"$stop_dir/LICENSE-MIT"
    run_stop_green
    expect_check_skipped
}

test_stop_hook_reruns_on_fixture_change() {
    mkdir -p "$stop_dir/tests/fixtures"
    echo '{"result": "success"}' >"$stop_dir/tests/fixtures/x.json"
    run_stop_green
    expect_check_ran
    echo '{"result": "error"}' >"$stop_dir/tests/fixtures/x.json"
    run_stop_green
    expect_check_ran
}

test_stop_hook_reruns_on_new_untracked_rs() {
    printf 'pub fn c() {}\n' >"$stop_dir/c.rs"
    run_stop_green
    expect_check_ran
}

test_stop_hook_reruns_on_deleted_file() {
    git -C "$stop_dir" add b.rs
    run_stop_green
    expect_check_skipped # staging alone changes no input
    rm "$stop_dir/b.rs"
    run_stop_green
    expect_check_ran
    run_stop_green
    expect_check_skipped
}

test_stop_hook_reruns_on_toolchain_change() {
    toolchain_id=toolchain-2
    run_stop_green
    expect_check_ran
}

test_stop_hook_default_toolchain_id_works() {
    rm -f "$check_ran"
    run_with_input '{"hook_event_name":"Stop","stop_hook_active":false}' \
        env -u X4_TOOLCHAIN_ID_CMD CLAUDE_PROJECT_DIR="$stop_dir" \
        X4_STOP_CHECK_CMD="touch $(printf %q "$check_ran")" "$stop_hook"
    expect_status 0
    expect_check_ran # the real rustc/clippy versions differ from the stubbed id
}

run_guard() { # <command>
    local json
    json=$(jq -n --arg command "$1" '{tool_name: "Bash", tool_input: {command: $command}}')
    run_with_input "$json" "$guard_hook"
}

expect_blocked() { # <command...>
    local command
    for command in "$@"; do
        run_guard "$command"
        [[ $status -eq 2 ]] || fail "expected exit 2 for: $command (got $status)"
        expect_contains "guard-bash: blocked:" "$stderr" "stderr for: $command"
    done
}

expect_allowed() { # <command...>
    local command
    for command in "$@"; do
        run_guard "$command"
        [[ $status -eq 0 ]] || fail "expected exit 0 for: $command (got $status, stderr: $stderr)"
        [[ -z $stdout$stderr ]] || fail "expected no output for: $command, got: $stdout$stderr"
    done
}

test_guard_blocks_commit_no_verify() {
    expect_blocked 'git commit --no-verify -m x' 'git commit -m x --no-verify' \
        'git commit --no-ver -m x' 'cd /tmp && git -C . commit --no-verify' \
        'bash -c "git commit --no-verify -m x"' 'echo "$(git commit --no-verify -m x)"'
}

test_guard_blocks_commit_short_n() {
    expect_blocked 'git commit -n -m x' 'git commit -an -m x' 'git commit -anm x' \
        'FOO=1 git commit -nm x' 'timeout 60 git commit -n' '/usr/bin/git commit -n'
}

test_guard_blocks_push_no_verify() {
    expect_blocked 'git push --no-verify' 'git push origin master --no-verify' \
        'sh -lc "git push --no-verify"'
}

test_guard_blocks_hooks_path_change() {
    expect_blocked 'git config core.hooksPath /dev/null' 'git config --unset core.hooksPath' \
        'git config --global core.hooksPath .githooks' 'git config set core.hooksPath x' \
        'git config core.hookspath .githooks-off' 'git config --remove-section core'
}

test_guard_blocks_hooks_path_override() {
    expect_blocked 'git -c core.hooksPath=/dev/null commit -m x' \
        'git -c core.hookspath= push' 'git --config-env=core.hooksPath=EMPTY commit -m x' \
        "GIT_CONFIG_PARAMETERS=\"'core.hooksPath'='/dev/null'\" git commit -m x"
}

test_guard_blocks_hook_state_writes() {
    expect_blocked 'echo x > target/.agent-hooks/last-green' \
        'echo x >>target/.agent-hooks/blocks' 'echo x 2>target/.agent-hooks/last-green' \
        'echo x | tee target/.agent-hooks/last-green' 'rm -rf target/.agent-hooks' \
        'mv /tmp/x target/.agent-hooks/last-green' 'cp /tmp/x target/.agent-hooks/last-green' \
        'touch target/.agent-hooks/last-green' 'sed -i s/a/b/ target/.agent-hooks/last-green'
}

test_guard_allows_lookalikes() {
    expect_allowed 'grep no-verify AGENTS.md' 'grep -rn -- --no-verify scripts' \
        'cat target/.agent-hooks/last-green' 'ls target/.agent-hooks' \
        'git commit -m "docs: never use --no-verify or -n"' 'git commit -am "x" 2>&1' \
        'git commit -m -n' 'git commit -F /tmp/message.txt' 'git log -n 5 --oneline' \
        'git push -n origin master' 'git push origin master' \
        'git config core.hooksPath .githooks' 'git config --get core.hooksPath' \
        'cargo clean && rm -rf target' 'git status # git commit -n' \
        $'git commit -m "$(cat <<\'EOF\'\nfix: x\n\ngit commit --no-verify is blocked\nEOF\n)"' \
        $'cat > /tmp/notes <<EOF\ngit commit -n\nEOF\necho done'
}

test_guard_allows_escaped_backticks_in_double_quotes() {
    expect_allowed 'git commit -m "docs: explain \`git commit -n\` is blocked"'
}

test_guard_allows_escaped_dollar_paren() {
    expect_allowed 'git commit -m "docs: \$(git commit -n) example"'
}

test_guard_allows_quoted_heredoc_body() {
    expect_allowed \
        $'git commit -m "$(cat <<\'EOF\'\ndocs: guard the `git commit --no-verify` form\nEOF\n)"' \
        $'git commit -m "$(cat <<\\EOF\ndocs: guard the `git commit --no-verify` form\nEOF\n)"' \
        $'git commit -m "$(cat <<\'EOF\'\ndocs: the $(git commit -n) form\nEOF\n)"' \
        $'git commit -m "$(cat <<\'EOF\'\ndon\'t run `git commit -n`\nEOF\n)" && echo done' \
        $'git commit -m $(cat <<"EOF"\ndocs: guard the `git commit --no-verify` form\nEOF\n)' \
        $'cat <<"EOF"\n$(git commit -n) and `git commit -n`\nEOF' \
        $'cat <<-\'EOF\'\n\t`git commit -n`\n\tEOF\necho done' \
        $'cat <<\'A\' <<"B"\n`git commit -n`\nA\n$(git commit -n)\nB\necho done'
}

test_guard_allows_escaped_backticks_in_pr_body() {
    expect_allowed 'gh pr create --title t --body "Blocks \`git commit -n\`"'
}

test_guard_allows_text_around_substitutions() {
    expect_allowed 'echo "$(date); git commit -n"' \
        'echo "$(date) \`git commit -n\` $(whoami)"' \
        $'cat <<EOF\n$(date)\ngit commit -n\nEOF\necho done' \
        $'cat <<EOF\n\\$(git commit -n) and \\`git commit -n\\`\nEOF\necho done' \
        $'cat <<EOF\n$(date) \\$(git commit -n)\nEOF\necho done'
}

test_guard_blocks_unescaped_backticks() {
    expect_blocked 'git commit -m "x `git commit -n`"' \
        'git commit -m "x \\`git commit -n`"' \
        'git commit -m "$(date) `git commit -n`"' \
        'echo "`echo \`git commit -n\``"'
}

test_guard_blocks_escape_lookalikes() {
    expect_blocked 'echo "\\$(git commit -n)"' \
        'bash -c "echo \$(git commit -n)"' \
        'bash -c "echo \`git commit -n\`"' \
        'echo "$(echo a; git commit -n)"' \
        'echo "$(echo "x")$(git commit -n)"' \
        $'cat <<EOF\n\\\\$(git commit -n)\nEOF' \
        $'cat <<\'EOF\'\nx\nEOF\ngit commit -n' \
        $'cat <<\'EOF\' && git commit -n\nx\nEOF'
}

# A `)` that does not close the substitution (a case pattern, `${x//)/}`) must not end it.
test_guard_blocks_ambiguous_close_paren() {
    expect_blocked 'echo "$(case x in x) git commit -n;; esac)"' \
        $'echo "$(case $x in\n a) echo a ;;\n *) git commit -n ;;\nesac\n)"' \
        'gh pr create --body "$(case $x in a) git commit -n;; esac)"' \
        'echo "$(echo ${x//)/}; git commit -n)"' \
        $'cat <<EOF\n$(case x in x) git commit -n;; esac)\nEOF' \
        'echo "$(echo `echo )`; git commit -n)"'
}

test_guard_blocks_unquoted_heredoc_substitution() {
    expect_blocked $'cat <<EOF\n$(git commit -n)\nEOF' \
        $'cat <<EOF\n`git commit -n`\nEOF' \
        $'cat <<-EOF\n\t$(git commit -n)\n\tEOF' \
        $'git commit -m "$(cat <<EOF\n$(git commit -n)\nEOF\n)"'
}

test_guard_ignores_other_tools() {
    run_with_input '{"tool_name":"Read","tool_input":{"file_path":"/tmp/x"}}' "$guard_hook"
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

# A minimal repository layout for lint-policy.sh, plus a stub `cargo` whose `cargo tree`
# output per crate comes from tree-<crate>.txt next to it (clean by default).
new_policy_root() {
    local dir crate
    dir=$(mktemp -d "$tmp_root/policy.XXXXXX")
    for crate in x4-core x4-rpc; do
        mkdir -p "$dir/crates/$crate/src"
        printf '[package]\nname = "%s"\n\n[lints]\nworkspace = true\n' "$crate" >"$dir/crates/$crate/Cargo.toml"
        printf '//! %s\n\n#[expect(clippy::too_many_lines, reason = "warnings are fine here")]\nfn f() {}\n' \
            "$crate" >"$dir/crates/$crate/src/lib.rs"
        printf '%s v0.1.0 (%s)\n' "$crate" "$dir/crates/$crate" >"$dir/tree-$crate.txt"
    done
    printf '#!/usr/bin/env bash\nwhile (($#)); do [[ $1 == -p ]] && crate=$2; shift; done\ncat "%s/tree-$crate.txt"\n' \
        "$dir" >"$dir/cargo"
    chmod +x "$dir/cargo"
    echo "$dir"
}

run_policy() { # <root>
    run_with_input '' env PATH="$1:$PATH" "$lint_policy" "$1"
}

test_lint_policy_passes_clean_tree() {
    local dir
    dir=$(new_policy_root)
    run_policy "$dir"
    expect_status 0
}

test_lint_policy_rejects_inner_allow_and_expect() {
    local dir
    dir=$(new_policy_root)
    printf '#![allow(dead_code)]\n' >>"$dir/crates/x4-core/src/lib.rs"
    printf '#![expect(\n    clippy::module_name_repetitions,\n    reason = "x"\n)]\n' >"$dir/crates/x4-rpc/src/a.rs"
    run_policy "$dir"
    expect_status 1
    expect_contains "crates/x4-core/src/lib.rs: crate-wide lint exception #![allow(dead_code)]" "$stderr" stderr
    expect_contains "crates/x4-rpc/src/a.rs: crate-wide lint exception" "$stderr" stderr
}

test_lint_policy_rejects_group_exceptions() {
    local dir
    dir=$(new_policy_root)
    printf '#[expect(clippy::pedantic, reason = "x")]\nfn g() {}\n' >>"$dir/crates/x4-core/src/lib.rs"
    printf '#[allow(\n    warnings\n)]\nfn h() {}\n' >"$dir/crates/x4-rpc/src/b.rs"
    run_policy "$dir"
    expect_status 1
    expect_contains "crates/x4-core/src/lib.rs: lint exception for a whole lint group or for warnings" "$stderr" stderr
    expect_contains "crates/x4-rpc/src/b.rs: lint exception for a whole lint group or for warnings" "$stderr" stderr
}

test_lint_policy_ignores_exceptions_in_comments() {
    local dir
    dir=$(new_policy_root)
    printf '/// Never write `#![allow(clippy::all)]`.\n// #[allow(warnings)]\nfn k() {}\n' \
        >>"$dir/crates/x4-core/src/lib.rs"
    run_policy "$dir"
    expect_status 0
}

test_lint_policy_rejects_missing_workspace_lints() {
    local dir
    dir=$(new_policy_root)
    printf '[package]\nname = "x4-rpc"\n' >"$dir/crates/x4-rpc/Cargo.toml"
    run_policy "$dir"
    expect_status 1
    expect_contains "crates/x4-rpc/Cargo.toml: missing [lints] workspace = true" "$stderr" stderr
}

test_lint_policy_rejects_wrong_dependency_direction() {
    local dir
    dir=$(new_policy_root)
    printf 'gpui-kit v0.8.1\nanyhow v1.0.104\nserde v1.0.0\n' >>"$dir/tree-x4-core.txt"
    printf 'x4-core v0.1.0 (%s)\n' "$dir/crates/x4-core" >>"$dir/tree-x4-rpc.txt"
    run_policy "$dir"
    expect_status 1
    expect_contains "x4-core: depends on gpui-kit" "$stderr" stderr
    expect_contains "x4-core: depends on anyhow" "$stderr" stderr
    expect_contains "x4-rpc: depends on x4-core" "$stderr" stderr
    [[ $stderr != *serde* ]] || fail "serde is an allowed dependency, got: $stderr"
}

cases=(
    format_hook_formats_rust_file
    format_hook_ignores_non_rust
    format_hook_ignores_files_outside_project
    format_hook_reports_syntax_error
    format_hook_tolerates_unresolved_module
    stop_hook_runs_and_records_green
    stop_hook_skips_unchanged
    stop_hook_blocks_on_failure
    stop_hook_gives_up_after_three
    stop_hook_green_clears_blocks
    stop_hook_skips_markdown_change
    stop_hook_reruns_on_fixture_change
    stop_hook_reruns_on_new_untracked_rs
    stop_hook_reruns_on_deleted_file
    stop_hook_reruns_on_toolchain_change
    stop_hook_default_toolchain_id_works
    guard_blocks_commit_no_verify
    guard_blocks_commit_short_n
    guard_blocks_push_no_verify
    guard_blocks_hooks_path_change
    guard_blocks_hooks_path_override
    guard_blocks_hook_state_writes
    guard_allows_lookalikes
    guard_allows_escaped_backticks_in_double_quotes
    guard_allows_escaped_dollar_paren
    guard_allows_quoted_heredoc_body
    guard_allows_escaped_backticks_in_pr_body
    guard_allows_text_around_substitutions
    guard_blocks_unescaped_backticks
    guard_blocks_escape_lookalikes
    guard_blocks_ambiguous_close_paren
    guard_blocks_unquoted_heredoc_substitution
    guard_ignores_other_tools
    lint_policy_passes_clean_tree
    lint_policy_rejects_inner_allow_and_expect
    lint_policy_rejects_group_exceptions
    lint_policy_ignores_exceptions_in_comments
    lint_policy_rejects_missing_workspace_lints
    lint_policy_rejects_wrong_dependency_direction
    check_tools_names_missing_tool
    check_tools_passes_when_all_present
)

for case_name in "${cases[@]}"; do
    "test_$case_name"
    echo "ok $case_name"
done
