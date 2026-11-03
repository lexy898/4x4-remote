#!/usr/bin/env bash
# Tests for the agent and git hook scripts, the repository's helper scripts and its packaging.
# Every case works in a throwaway directory under a temp root; the real repository
# and its target/.agent-hooks state are never touched. Prints "ok <case>" per case, or
# "skip <case>: <why>" for a case that needs a tool this machine lacks (makepkg off Arch Linux),
# and exits 1 on the first failure.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
format_hook=$root/scripts/hooks/format-rust-file.sh
stop_hook=$root/scripts/hooks/stop-check.sh
guard_hook=$root/scripts/hooks/guard-bash.sh
check_tools=$root/scripts/check-tools.sh
lint_policy=$root/scripts/lint-policy.sh
measure_lib=$root/scripts/measure-lib.sh
measure=$root/scripts/measure.sh
virtual_session=$root/scripts/virtual-session.sh
pkgbuild=$root/packaging/arch/PKGBUILD
icon=$root/packaging/linux/io.github.lexy898.FourByFour.svg

# Do not leak a surrounding git hook or agent session into the temp repos.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE CLAUDE_PROJECT_DIR X4_STOP_CHECK_CMD X4_TOOLCHAIN_ID_CMD

tmp_root=$(mktemp -d)
# Processes a case starts on purpose and stops itself (the virtual session's decoys). A failing
# case exits at once, so the trap stops whatever of them still runs; a pid is signalled only while
# its process still has the marker the case gave it, never once it has ended and the pid is reused.
started_pids=()
started_marker="X4_HOOK_TEST_ROOT=$tmp_root"
clean_up() {
    local pid
    for pid in "${started_pids[@]}"; do
        if grep -qzxF -- "$started_marker" "/proc/$pid/environ" 2>/dev/null; then
            kill -KILL "$pid" 2>/dev/null || true
        fi
    done
    rm -rf "$tmp_root"
}
trap clean_up EXIT

case_name=
skipped=
status=0
stdout=
stderr=

fail() {
    echo "FAIL $case_name: $*" >&2
    exit 1
}

# skip <why>: marks the case as skipped; the case returns right after.
skip() {
    skipped=$1
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

# The key-table test reads the README at the top of the repository, so a change of it must run the
# check; other Markdown, such as a README further down, still does not.
test_stop_hook_reruns_on_top_readme_change() {
    echo 'keys' >"$stop_dir/README.md"
    run_stop_green
    expect_check_ran
    echo 'other keys' >"$stop_dir/README.md"
    run_stop_green
    expect_check_ran
    mkdir -p "$stop_dir/docs"
    echo 'notes' >"$stop_dir/docs/README.md"
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
        $'git commit -m "$(cat <<\'EOF\'\nuse ${VAR} and `git commit -n` here\nEOF\n)"' \
        $'gh pr create --title t --body "$(cat <<\'EOF\'\nuse ${VAR}, the case where `git commit -n` runs\nEOF\n)"' \
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

# After the ambiguous `)` the text is still quoted or heredoc text: `#` and an apostrophe
# pair there hide nothing from the shell, which runs the substitution.
test_guard_blocks_substitution_after_ambiguous_close_paren() {
    expect_blocked 'echo "$(case x in x) :;; esac) it'\''s $(git commit -n) don'\''t"' \
        'echo "$(case x in x) :;; esac) # $(git commit -n)"' \
        'echo "$(echo ${x//)/}) it'\''s $(git commit -n) don'\''t"' \
        'echo "$(echo ${x//)/}) # $(git commit -n)"' \
        'echo "$(echo ${x:-a}) # $(git commit -n)"' \
        $'cat <<EOF\n$(case x in x) :;; esac) it\'s $(git commit -n) don\'t\nEOF' \
        $'cat <<EOF\n$(case x in x) :;; esac) # $(git commit -n)\nEOF'
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
# output per crate comes from tree-<crate>.txt next to it, and its stderr from
# tree-<crate>.stderr if that exists. Each call appends its arguments to cargo-args.txt.
# Clean by default: x4-app's tree is the vendored crate, the vendored renderer is patched, the
# workspace denies unsafe_code and the allocator holds the one exception to it.
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
    printf '[workspace.lints.rust]\nunsafe_code = "deny"\n' >"$dir/Cargo.toml"
    mkdir -p "$dir/crates/x4-app/src"
    printf '[package]\nname = "x4-app"\n\n[lints]\nworkspace = true\n' >"$dir/crates/x4-app/Cargo.toml"
    printf '//! The allocator.\n\n// SAFETY: a call.\n#[expect(unsafe_code, reason = "the owner allowed this")]\nlet taken = unsafe { f() };\n' \
        >"$dir/crates/x4-app/src/allocator.rs"
    printf 'gpui-pre-wgpu v0.3.10 (%s/vendor/gpui-pre-wgpu)\n' "$dir" >"$dir/tree-x4-app.txt"
    mkdir -p "$dir/vendor/gpui-pre-wgpu/src"
    printf 'fn create_context_vulkan_first() {}\n' >"$dir/vendor/gpui-pre-wgpu/src/wgpu_renderer.rs"
    cat >"$dir/cargo" <<EOF
#!/usr/bin/env bash
echo "\$*" >>"$dir/cargo-args.txt"
while ((\$#)); do [[ \$1 == -p ]] && crate=\$2; shift; done
if [[ -f "$dir/tree-\$crate.stderr" ]]; then cat "$dir/tree-\$crate.stderr" >&2; fi
cat "$dir/tree-\$crate.txt"
EOF
    chmod +x "$dir/cargo"
    echo "$dir"
}

run_policy() { # <root>
    run_with_input '' env PATH="$1:$PATH" "$lint_policy" "$1"
}

test_lint_policy_passes_clean_tree() {
    local dir args
    dir=$(new_policy_root)
    run_policy "$dir"
    expect_status 0
    # Locked, so the check sees what the build gets; every target, so it works on any host;
    # never --offline, which fails on a cold registry cache (CI, `just update`, Dependabot).
    args=$(grep -e '-i gpui-pre-wgpu' "$dir/cargo-args.txt") || fail "cargo tree -i gpui-pre-wgpu was not run"
    expect_contains "-p x4-app" "$args" "cargo arguments"
    expect_contains "--locked" "$args" "cargo arguments"
    expect_contains "--target all" "$args" "cargo arguments"
    [[ $args != *--offline* ]] || fail "cargo tree must not run --offline, got: $args"
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

# What a gpui-kit bump to another gpui-pre-wgpu version looks like: the registry crate is built.
test_lint_policy_rejects_registry_gpui_pre_wgpu() {
    local dir
    dir=$(new_policy_root)
    printf 'gpui-pre-wgpu v0.3.11\n└── gpui-pre-linux v0.3.11\n' >"$dir/tree-x4-app.txt"
    run_policy "$dir"
    expect_status 1
    expect_contains "x4-app: gpui-pre-wgpu resolves to 'gpui-pre-wgpu v0.3.11', not vendor/gpui-pre-wgpu" \
        "$stderr" stderr
    expect_contains "vendor/gpui-pre-wgpu/PATCHED.md" "$stderr" stderr
}

test_lint_policy_rejects_unused_gpui_pre_wgpu_patch() {
    local dir
    dir=$(new_policy_root)
    printf 'warning: patch `gpui-pre-wgpu v0.3.10 (%s/vendor/gpui-pre-wgpu)` was not used in the crate graph\n' \
        "$dir" >"$dir/tree-x4-app.stderr"
    run_policy "$dir"
    expect_status 1
    expect_contains "x4-app: Cargo reports the vendored gpui-pre-wgpu patch as unused" "$stderr" stderr
}

# A re-vendored copy whose patch was not re-applied: Cargo builds it, but it loads OpenGL again.
test_lint_policy_rejects_unpatched_vendored_gpui_pre_wgpu() {
    local dir
    dir=$(new_policy_root)
    printf 'fn create_context() {}\n' >"$dir/vendor/gpui-pre-wgpu/src/wgpu_renderer.rs"
    run_policy "$dir"
    expect_status 1
    expect_contains "vendor/gpui-pre-wgpu/src/wgpu_renderer.rs: has no create_context_vulkan_first" \
        "$stderr" stderr
    rm "$dir/vendor/gpui-pre-wgpu/src/wgpu_renderer.rs"
    run_policy "$dir"
    expect_status 1
    expect_contains "vendor/gpui-pre-wgpu/src/wgpu_renderer.rs: has no create_context_vulkan_first" \
        "$stderr" stderr
}

test_lint_policy_rejects_failed_gpui_pre_wgpu_lookup() {
    local dir
    dir=$(new_policy_root)
    rm "$dir/tree-x4-app.txt" # the stub cargo then fails, like an ambiguous or missing package
    run_policy "$dir"
    expect_status 1
    expect_contains "x4-app: cargo tree failed, cannot check that gpui-pre-wgpu is the vendored copy" \
        "$stderr" stderr
}

# The one `unsafe` is the owner's: an unsafe_code exception, at any level, only in the allocator.
test_lint_policy_rejects_an_unsafe_code_exception_elsewhere() {
    local dir
    dir=$(new_policy_root)
    printf '#[expect(unsafe_code, reason = "x")]\nfn g() {}\n' >>"$dir/crates/x4-core/src/lib.rs"
    printf '#[allow(\n    unsafe_code\n)]\nfn h() {}\n' >"$dir/crates/x4-rpc/src/c.rs"
    printf '#![forbid(unsafe_code)]\n' >"$dir/crates/x4-rpc/src/d.rs"
    printf '#[cfg_attr(test, expect(clippy::too_many_lines, unsafe_code))]\nfn i() {}\n' \
        >"$dir/crates/x4-app/src/other.rs"
    run_policy "$dir"
    expect_status 1
    expect_contains "crates/x4-core/src/lib.rs: unsafe_code exception outside crates/x4-app/src/allocator.rs" \
        "$stderr" stderr
    expect_contains "crates/x4-rpc/src/c.rs: unsafe_code exception outside" "$stderr" stderr
    expect_contains "crates/x4-rpc/src/d.rs: unsafe_code exception outside" "$stderr" stderr
    expect_contains "crates/x4-app/src/other.rs: unsafe_code exception outside" "$stderr" stderr
    [[ $stderr != *"crates/x4-app/src/allocator.rs:"* ]] || fail "the allocator is the home of the exception, got: $stderr"
}

test_lint_policy_rejects_a_second_unsafe_code_exception_in_the_allocator() {
    local dir
    dir=$(new_policy_root)
    printf '#[expect(unsafe_code, reason = "another")]\nlet again = unsafe { g() };\n' \
        >>"$dir/crates/x4-app/src/allocator.rs"
    run_policy "$dir"
    expect_status 1
    expect_contains "crates/x4-app/src/allocator.rs: 2 unsafe_code exceptions" "$stderr" stderr
}

test_lint_policy_ignores_unsafe_code_in_comments_and_reasons() {
    local dir
    dir=$(new_policy_root)
    printf '/// Never write `#[allow(unsafe_code)]`.\n// #![forbid(unsafe_code)]\n' \
        >>"$dir/crates/x4-core/src/lib.rs"
    printf '#[expect(clippy::too_many_lines, reason = "unsafe_code is not meant")]\nfn j() {}\n' \
        >>"$dir/crates/x4-core/src/lib.rs"
    run_policy "$dir"
    expect_status 0
}

# `forbid` cannot be overridden, not even by #[expect], so the allocator would not build.
test_lint_policy_rejects_a_workspace_unsafe_level_other_than_deny() {
    local dir level
    dir=$(new_policy_root)
    for level in forbid warn allow; do
        printf '[workspace.lints.rust]\nunsafe_code = "%s"\n' "$level" >"$dir/Cargo.toml"
        run_policy "$dir"
        expect_status 1
        expect_contains "Cargo.toml: workspace.lints.rust.unsafe_code is '$level', not deny" "$stderr" stderr
    done
    printf '[workspace.lints.rust]\nmissing_docs = "warn"\n' >"$dir/Cargo.toml"
    run_policy "$dir"
    expect_status 1
    expect_contains "Cargo.toml: workspace.lints.rust.unsafe_code is 'unset', not deny" "$stderr" stderr
}

# Canned copies of /proc/<pid>/smaps_rollup, status and maps in a fresh directory, as
# scripts/measure.sh takes them. Sizes are in kB. The extra lines are decoys: keys that start
# like the ones to read (Pss_Dirty, VmHWM, RssFile) and mappings of other libraries.
# write_proc_files <pss> <private-clean> <private-dirty> <vmrss> <rssanon> <threads> [<mapped library>...]
proc_dir=
write_proc_files() {
    local pss=$1 clean=$2 dirty=$3 rss=$4 anon=$5 threads=$6 library
    shift 6
    proc_dir=$(mktemp -d "$tmp_root/proc.XXXXXX")
    {
        printf '560000000000-7ffd5a2fa000 ---p 00000000 00:00 0                          [rollup]\n'
        printf 'Rss:            %8d kB\n' "$rss"
        printf 'Pss:            %8d kB\n' "$pss"
        printf 'Pss_Dirty:      %8d kB\n' 11111
        printf 'Pss_Anon:       %8d kB\n' 22222
        printf 'Shared_Clean:   %8d kB\n' 33333
        printf 'Shared_Dirty:   %8d kB\n' 44444
        printf 'Private_Clean:  %8d kB\n' "$clean"
        printf 'Private_Dirty:  %8d kB\n' "$dirty"
        printf 'Anonymous:      %8d kB\n' 55555
    } >"$proc_dir/smaps_rollup"
    {
        printf 'Name:\t4x4\nState:\tS (sleeping)\nVmPeak:\t  999999 kB\n'
        printf 'VmHWM:\t  %d kB\n' 888888
        printf 'VmRSS:\t  %d kB\n' "$rss"
        printf 'RssAnon:\t  %d kB\n' "$anon"
        printf 'RssFile:\t  %d kB\n' 77777
        printf 'Threads:\t%d\n' "$threads"
    } >"$proc_dir/status"
    {
        printf '55d0c0000000-55d0c1000000 r-xp 00000000 08:01 100   /home/u/4x4/target/release/4x4\n'
        printf '7f0000000000-7f0000100000 r-xp 00000000 08:01 101   /usr/lib/libvulkan.so.1.4.0\n'
        printf '7f0000200000-7f0000300000 r-xp 00000000 08:01 102   /usr/lib/libEGL.so.1.1.0\n'
        printf '7f0000400000-7f0000500000 r-xp 00000000 08:01 103   /usr/lib/libGLdispatch.so.0.0.0\n'
        for library in "$@"; do
            printf '7f0000600000-7f0000700000 r-xp 00000000 08:01 104   %s\n' "$library"
        done
        printf '7ffd5a2da000-7ffd5a2fb000 rw-p 00000000 00:00 0                          [stack]\n'
    } >"$proc_dir/maps"
}

# run_measure <function> <args...>: runs a function of scripts/measure-lib.sh in its own shell,
# so the library's names cannot collide with this script's.
run_measure() {
    run_with_input '' bash -c 'source "$1"; shift; "$@"' _ "$measure_lib" "$@"
}

test_measure_report_line_has_every_metric_in_order() {
    write_proc_files 49152 10240 13312 112640 12800 26
    run_measure report_line 210 "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/maps" 33554432
    expect_status 0
    [[ $stdout == 'first_frame_ms=210 pss_mib=48.0 uss_mib=23.0 rss_anon_mib=12.5 rss_mib=110.0 threads=26 gl_stack_mapped=no binary_mib=32.0' ]] ||
        fail "unexpected report line: $stdout"
    [[ -z $stderr ]] || fail "expected no complaint, got: $stderr"
}

test_measure_report_line_flags_each_gl_stack_library() {
    local library
    for library in /usr/lib/libgallium-26.0.0.so /usr/lib/libEGL_mesa.so.0.0.0 /usr/lib/libGLX_mesa.so.0.0.0 \
        '/usr/lib/libgallium-26.0.0.so (deleted)'; do
        write_proc_files 49152 10240 13312 112640 12800 26 "$library"
        run_measure report_line 210 "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/maps" 33554432
        expect_status 0
        expect_contains 'gl_stack_mapped=yes' "$stdout" "the report line with $library mapped"
    done
}

test_measure_report_line_rejects_unreadable_metrics() {
    write_proc_files 49152 10240 13312 112640 12800 26
    sed -i '/^Pss:/d' "$proc_dir/smaps_rollup"
    run_measure report_line 210 "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/maps" 33554432
    expect_status 1
    expect_contains "Pss:" "$stderr" stderr
    [[ -z $stdout ]] || fail "expected no report line, got: $stdout"
    write_proc_files 49152 10240 13312 112640 12800 26
    sed -i '/^Threads:/d' "$proc_dir/status"
    run_measure report_line 210 "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/maps" 33554432
    expect_status 1
    expect_contains "Threads:" "$stderr" stderr
    write_proc_files 49152 10240 13312 112640 12800 26
    run_measure report_line 210 "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/missing-maps" 33554432
    expect_status 1
}

test_measure_warnings_are_silent_within_the_targets() {
    # Exactly on each limit: RssAnon 16 MiB, PSS 55 MiB, RSS 120 MiB.
    write_proc_files 56320 10240 13312 122880 16384 26
    run_measure report_warnings "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/maps"
    expect_status 0
    [[ -z $stdout && -z $stderr ]] || fail "expected no output, got stdout: $stdout, stderr: $stderr"
}

test_measure_warnings_name_every_breach_on_stderr_and_keep_exit_zero() {
    write_proc_files 56321 10240 13312 122881 16385 26 /usr/lib/libgallium-26.0.0.so
    run_measure report_warnings "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/maps"
    expect_status 0
    [[ -z $stdout ]] || fail "warnings belong on stderr, got stdout: $stdout"
    expect_contains 'WARN: rss_anon_mib=16.0 is above the 16 MiB target' "$stderr" stderr
    expect_contains 'WARN: pss_mib=55.0 is above the 55 MiB target' "$stderr" stderr
    expect_contains 'WARN: rss_mib=120.0 is above the 120 MiB reference limit' "$stderr" stderr
    expect_contains 'WARN: gl_stack_mapped=yes' "$stderr" stderr
    [[ $(grep -c '^WARN: ' <<<"$stderr") -eq 4 ]] || fail "expected exactly four warnings, got: $stderr"
}

test_measure_warnings_report_one_breach_alone() {
    write_proc_files 49152 10240 13312 112640 17408 26
    run_measure report_warnings "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/maps"
    expect_status 0
    [[ $stderr == 'WARN: rss_anon_mib=17.0 is above the 16 MiB target' ]] || fail "unexpected warnings: $stderr"
}

test_measure_warnings_fail_only_when_a_metric_is_unreadable() {
    write_proc_files 49152 10240 13312 112640 12800 26
    sed -i '/^RssAnon:/d' "$proc_dir/status"
    run_measure report_warnings "$proc_dir/smaps_rollup" "$proc_dir/status" "$proc_dir/maps"
    expect_status 1
    expect_contains "RssAnon:" "$stderr" stderr
}

# write_stat <file> <utime> <stime>: a copy of /proc/<pid>/stat whose command name holds spaces
# and parentheses, as a thread name may; utime and stime are fields 14 and 15.
write_stat() {
    printf '4242 (4x4 (x) y) S 1 4242 4242 0 -1 4194304 9000 0 12 0 %d %d 0 0 20 0 26 0 100\n' \
        "$2" "$3" >"$1"
}

test_measure_cpu_line_counts_user_and_system_time_over_the_window() {
    local dir
    dir=$(mktemp -d "$tmp_root/stat.XXXXXX")
    write_stat "$dir/before" 100 50
    write_stat "$dir/after" 130 56
    # 36 ticks at 100 per second are 0.36 s of CPU in 30 s: 1.2 % of one core.
    run_measure cpu_line "$dir/before" "$dir/after" 30 100
    expect_status 0
    [[ $stdout == 'cpu_percent=1.20 window_s=30.0' ]] || fail "unexpected CPU line: $stdout"
    run_measure cpu_line "$dir/before" "$dir/before" 30.04 100
    [[ $stdout == 'cpu_percent=0.00 window_s=30.0' ]] || fail "unexpected idle CPU line: $stdout"
}

test_measure_elapsed_seconds_reads_either_decimal_separator() {
    run_measure elapsed_seconds 1793246118,250000 1793246148,750000
    [[ $stdout == 30.5 ]] || fail "unexpected seconds with commas: $stdout"
    run_measure elapsed_seconds 1793246118.250000 1793246119.000000
    [[ $stdout == 0.75 ]] || fail "unexpected seconds with dots: $stdout"
}

test_measure_cpu_line_rejects_an_unreadable_stat() {
    local dir
    dir=$(mktemp -d "$tmp_root/stat.XXXXXX")
    write_stat "$dir/before" 100 50
    run_measure cpu_line "$dir/before" "$dir/missing" 30 100
    expect_status 1
    printf '4242 (4x4) S 1 4242\n' >"$dir/short"
    run_measure cpu_line "$dir/before" "$dir/short" 30 100
    expect_status 1
    expect_contains "no utime and stime" "$stderr" stderr
    [[ -z $stdout ]] || fail "expected no CPU line, got: $stdout"
}

# A fake build for scripts/measure.sh: `just` and `cargo` that record their arguments, and in
# <dir>/target the binaries of the app and of the synthetic demo, which record theirs and
# their XDG_STATE_HOME, write a state file and a log there as the app does, log their first frame
# like the app and then sleep until the script stops them.
new_measure_env() {
    local dir name
    dir=$(mktemp -d "$tmp_root/measure.XXXXXX")
    mkdir -p "$dir/bin" "$dir/target/release" "$dir/home" "$dir/tmp"
    for name in just cargo; do
        printf '#!/usr/bin/env bash\necho "$*" >>%q\n' "$dir/$name.txt" >"$dir/bin/$name"
        chmod +x "$dir/bin/$name"
    done
    for name in 4x4 x4-synthetic; do
        printf '#!/usr/bin/env bash\necho "$*" >%q\necho "${XDG_STATE_HOME-unset}" >%q\nmkdir -p "$XDG_STATE_HOME/4x4" && touch "$XDG_STATE_HOME/4x4/state.toml" "$XDG_STATE_HOME/4x4/4x4.log"\necho "INFO x4_app::app: first frame rendered elapsed_ms=42" >&2\nexec sleep 30\n' \
            "$dir/args-$name.txt" "$dir/state-$name.txt" >"$dir/target/release/$name"
        chmod +x "$dir/target/release/$name"
    done
    echo "$dir"
}

run_measure_script() { # <dir> <arg...>
    local dir=$1
    shift
    run_with_input '' env -u XDG_STATE_HOME PATH="$dir/bin:$PATH" CARGO_TARGET_DIR="$dir/target" \
        HOME="$dir/home" TMPDIR="$dir/tmp" X4_MEASURE_SETTLE_SECONDS=1 "$BASH" "$measure" "$@"
}

test_measure_measures_the_synthetic_demo_with_its_arguments_and_cpu() {
    local dir
    dir=$(new_measure_env)
    run_measure_script "$dir" --synthetic --cpu 1 -- --static 'two words'
    expect_status 0
    [[ $(wc -l <<<"$stdout") -eq 2 ]] || fail "expected the report and the CPU line, got: $stdout"
    [[ $stdout =~ ^first_frame_ms=42\ pss_mib=[0-9.]+\ .*binary_mib= ]] || fail "unexpected report: $stdout"
    [[ $stdout =~ $'\n'cpu_percent=[0-9]+\.[0-9]{2}\ window_s=1\.[0-9]$ ]] || fail "unexpected CPU line: $stdout"
    [[ $(<"$dir/cargo.txt") == "build -p x4-app --release --locked --features synthetic --bin x4-synthetic" ]] ||
        fail "unexpected build: $(<"$dir/cargo.txt")"
    [[ ! -e $dir/just.txt ]] || fail "the app was built too: $(<"$dir/just.txt")"
    [[ $(<"$dir/args-x4-synthetic.txt") == "--static two words" ]] ||
        fail "unexpected arguments: $(<"$dir/args-x4-synthetic.txt")"
    [[ $(<"$dir/state-x4-synthetic.txt") == "$dir/tmp/"* ]] ||
        fail "the demo did not get a temp state dir: $(<"$dir/state-x4-synthetic.txt")"
}

test_measure_measures_the_app_by_default() {
    local dir
    dir=$(new_measure_env)
    run_measure_script "$dir"
    expect_status 0
    [[ $stdout =~ ^first_frame_ms=42\ pss_mib=[0-9.]+\ [^$'\n']*$ ]] || fail "expected one report line, got: $stdout"
    [[ $(<"$dir/just.txt") == release ]] || fail "unexpected build: $(<"$dir/just.txt")"
    [[ -e $dir/args-4x4.txt ]] || fail "the app did not run"
}

# The measured app saves its state and writes its log under XDG_STATE_HOME: a temp dir, so that a
# measurement never saves over the owner's state.toml, removed at the end.
test_measure_gives_the_app_a_temp_state_dir() {
    local dir state
    dir=$(new_measure_env)
    run_measure_script "$dir"
    expect_status 0
    state=$(<"$dir/state-4x4.txt")
    [[ $state == "$dir/tmp/"* ]] || fail "the state dir is not a temp dir: '$state'"
    [[ ! -e $state ]] || fail "the state dir was left behind: $state"
    [[ -z $(ls -A "$dir/home") ]] || fail "something was written to HOME: $(ls -A "$dir/home")"
}

# The app ignores a relative XDG_STATE_HOME and saves into ~/.local/state instead, so the temp
# state dir must be absolute even when TMPDIR is relative; the caller's directory is its base.
test_measure_gives_the_app_an_absolute_state_dir_under_a_relative_tmpdir() {
    local dir state
    dir=$(new_measure_env)
    run_with_input '' env -C "$dir" -u XDG_STATE_HOME PATH="$dir/bin:$PATH" \
        CARGO_TARGET_DIR="$dir/target" HOME="$dir/home" TMPDIR=tmp X4_MEASURE_SETTLE_SECONDS=1 \
        "$BASH" "$measure"
    expect_status 0
    state=$(<"$dir/state-4x4.txt")
    [[ $state == "$dir/tmp/"* ]] || fail "the state dir is not an absolute temp dir: '$state'"
    [[ ! -e $state ]] || fail "the state dir was left behind: $state"
    [[ -z $(ls -A "$dir/tmp") ]] || fail "temp files were left behind: $(ls -A "$dir/tmp")"
}

test_measure_rejects_unknown_options() {
    local dir option
    dir=$(new_measure_env)
    for option in --bogus "--cpu x" "--cpu" "--example synthetic"; do
        # Unquoted on purpose: "--cpu x" is an option and its value.
        run_measure_script "$dir" $option
        expect_status 2
        expect_contains "usage: scripts/measure.sh" "$stderr" "stderr for $option"
    done
    [[ ! -e $dir/just.txt && ! -e $dir/cargo.txt ]] || fail "a bad command line built something"
}

# A directory with a fake `dbus-run-session`, which runs its command, and a fake `kwin_wayland`,
# which runs its --exit-with-session script as KWin does once the virtual screen is up and prints
# some noise of its own. Each writes its arguments, the XDG dirs and the display variables it
# sees to <dir>/<tool>.txt. Also an owner's home with a kdeglobals, and an empty TMPDIR for the
# session's directory.
virtual_dir=
virtual_seen='^(XDG_[A-Z_]+|WAYLAND_DISPLAY|DISPLAY|XAUTHORITY)='
new_virtual_env() {
    virtual_dir=$(mktemp -d "$tmp_root/virtual.XXXXXX")
    mkdir -p "$virtual_dir/bin" "$virtual_dir/home/.config/fontconfig" "$virtual_dir/tmp"
    printf '[General]\nColorScheme=Owner\n' >"$virtual_dir/home/.config/kdeglobals"
    printf '<fontconfig/>\n' >"$virtual_dir/home/.config/fontconfig/fonts.conf"
    cat >"$virtual_dir/bin/dbus-run-session" <<EOF
#!/usr/bin/env bash
seen='$virtual_seen'
{ echo "args: \$*"; env | grep -E "\$seen" || true; } >"$virtual_dir/dbus-run-session.txt"
[[ \$1 == -- ]] && shift
exec "\$@"
EOF
    cat >"$virtual_dir/bin/kwin_wayland" <<EOF
#!/usr/bin/env bash
seen='$virtual_seen'
{ echo "args: \$*"; env | grep -E "\$seen" || true; } >"$virtual_dir/kwin_wayland.txt"
echo "kwin noise"
while ((\$#)); do
    if [[ \$1 == --exit-with-session ]]; then "\$2"; exit 0; fi
    shift
done
exit 1
EOF
    chmod +x "$virtual_dir/bin/dbus-run-session" "$virtual_dir/bin/kwin_wayland"
}

# run_virtual <command...>: the helper with the fakes first on PATH, in the fake home, with the
# owner's XDG dirs left to their defaults and the owner's displays, X authority and runtime dir set
# as on a desktop.
run_virtual() {
    mkdir -p "$virtual_dir/owner-runtime"
    run_with_input '' env -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_STATE_HOME -u XDG_CACHE_HOME \
        -u X4_VIRTUAL_KEEP PATH="$virtual_dir/bin:$PATH" HOME="$virtual_dir/home" \
        TMPDIR="$virtual_dir/tmp" DISPLAY=:0 XAUTHORITY="$virtual_dir/home/.Xauthority" \
        WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR="$virtual_dir/owner-runtime" \
        "$BASH" "$virtual_session" "$@"
}

test_virtual_session_propagates_the_exit_code() {
    new_virtual_env
    run_virtual bash -c 'exit 7'
    expect_status 7
    run_virtual true
    expect_status 0
    expect_contains "args: -- kwin_wayland --virtual" "$(<"$virtual_dir/dbus-run-session.txt")" \
        "the dbus-run-session call"
    expect_contains "--exit-with-session" "$(<"$virtual_dir/kwin_wayland.txt")" "the kwin call"
}

test_virtual_session_gives_kwin_and_the_command_temp_xdg_dirs() {
    local name dir seen
    new_virtual_env
    # The command also overwrites its kdeglobals, which must not reach the owner's.
    run_virtual bash -c 'for name in CONFIG DATA STATE CACHE; do
            dir=XDG_${name}_HOME; echo "$name=${!dir}"; [[ -d ${!dir} ]] || echo "$name missing"
        done
        cat "$XDG_CONFIG_HOME/kdeglobals" "$XDG_CONFIG_HOME/fontconfig/fonts.conf"
        echo "display=${DISPLAY-unset}"
        echo changed >"$XDG_CONFIG_HOME/kdeglobals"'
    expect_status 0
    for name in CONFIG DATA STATE CACHE; do
        dir=$(sed -n "s/^$name=//p" <<<"$stdout")
        [[ $dir == "$virtual_dir/tmp/"* ]] || fail "XDG_${name}_HOME is not in TMPDIR: '$dir'"
        [[ $dir != "$virtual_dir/home"* ]] || fail "XDG_${name}_HOME is in HOME: '$dir'"
        for seen in kwin_wayland dbus-run-session; do
            expect_contains "XDG_${name}_HOME=$dir" "$(<"$virtual_dir/$seen.txt")" "the env of $seen"
        done
    done
    [[ $stdout != *missing* ]] || fail "a session dir does not exist: $stdout"
    expect_contains "ColorScheme=Owner" "$stdout" "the session's kdeglobals"
    expect_contains "<fontconfig/>" "$stdout" "the session's fontconfig"
    expect_contains "display=unset" "$stdout" "the command's DISPLAY"
    [[ $(<"$virtual_dir/home/.config/kdeglobals") == $'[General]\nColorScheme=Owner' ]] ||
        fail "the owner's kdeglobals changed"
    [[ -z $(ls -A "$virtual_dir/tmp") ]] || fail "the session dir was left behind: $(ls -A "$virtual_dir/tmp")"
    [[ $stdout != *"kwin noise"* && $stderr != *"kwin noise"* ]] ||
        fail "KWin's own output belongs in its log, got: $stdout $stderr"
}

# Services on the session's bus (the portal, ksecretd) must not reach the owner's X server or
# runtime dir: KWin, the bus and the command see the session's Wayland socket, no DISPLAY and no
# XAUTHORITY, and a runtime dir of the session's own.
test_virtual_session_keeps_the_owner_display_and_runtime_dir_away() {
    local seen socket runtime
    new_virtual_env
    run_virtual bash -c 'echo "runtime=$XDG_RUNTIME_DIR mode=$(stat -c %a "$XDG_RUNTIME_DIR")"
        echo "wayland=$WAYLAND_DISPLAY display=${DISPLAY-unset} xauthority=${XAUTHORITY-unset}"'
    expect_status 0
    socket=$(sed -n 's/.*--socket \([^ ]*\).*/\1/p' "$virtual_dir/kwin_wayland.txt")
    [[ $socket == x4-virtual-* ]] || fail "unexpected KWin socket: '$socket'"
    runtime=$(sed -n 's/^runtime=\([^ ]*\) .*/\1/p' <<<"$stdout")
    [[ $runtime == "$virtual_dir/tmp/x4-virtual."*/runtime ]] || fail "unexpected runtime dir: '$runtime'"
    expect_contains "mode=700" "$stdout" "the runtime dir's mode"
    expect_contains "wayland=$socket display=unset xauthority=unset" "$stdout" "the command's display"
    for seen in dbus-run-session kwin_wayland; do
        expect_contains "WAYLAND_DISPLAY=$socket" "$(<"$virtual_dir/$seen.txt")" "the env of $seen"
        expect_contains "XDG_RUNTIME_DIR=$runtime" "$(<"$virtual_dir/$seen.txt")" "the env of $seen"
        if grep -Eq '^(DISPLAY|XAUTHORITY)=' "$virtual_dir/$seen.txt"; then
            fail "$seen sees the owner's X server: $(<"$virtual_dir/$seen.txt")"
        fi
    done
}

# A fontconfig dir and a kdeglobals that are symlinks (into a dotfiles repository, say) are copied
# as a real dir and file, so the session's writes never follow the links into the owner's files.
test_virtual_session_copies_linked_settings_as_files() {
    local config
    new_virtual_env
    config=$virtual_dir/home/.config
    mkdir -p "$virtual_dir/dotfiles"
    mv "$config/fontconfig" "$config/kdeglobals" "$virtual_dir/dotfiles/"
    ln -s "$virtual_dir/dotfiles/fontconfig" "$config/fontconfig"
    ln -s "$virtual_dir/dotfiles/kdeglobals" "$config/kdeglobals"
    run_virtual bash -c 'cd "$XDG_CONFIG_HOME"
        [[ -L fontconfig || -L kdeglobals ]] && echo "linked"
        [[ -d fontconfig && -f fontconfig/fonts.conf && -f kdeglobals ]] && echo "copied"
        echo changed >kdeglobals; echo changed >fontconfig/fonts.conf'
    expect_status 0
    [[ $stdout == copied ]] || fail "expected real copies, got: $stdout"
    [[ $(<"$virtual_dir/dotfiles/kdeglobals") == $'[General]\nColorScheme=Owner' ]] ||
        fail "the owner's kdeglobals changed"
    [[ $(<"$virtual_dir/dotfiles/fontconfig/fonts.conf") == '<fontconfig/>' ]] ||
        fail "the owner's fonts.conf changed"
}

test_virtual_session_keeps_arguments_with_spaces() {
    new_virtual_env
    # The literal $HOME and * must reach the command unexpanded.
    run_virtual printf '<%s>' 'a b' '' "it's" '$HOME' '*'
    expect_status 0
    [[ $stdout == "<a b><><it's><\$HOME><*>" ]] || fail "unexpected arguments: $stdout"
}

test_virtual_session_names_a_missing_tool() {
    new_virtual_env
    rm "$virtual_dir/bin/kwin_wayland"
    run_with_input '' env PATH="$virtual_dir/bin" "$BASH" "$virtual_session" true
    expect_status 127
    expect_contains "kwin_wayland not found" "$stderr" stderr
    expect_contains "KDE" "$stderr" stderr
    new_virtual_env
    rm "$virtual_dir/bin/dbus-run-session"
    run_with_input '' env PATH="$virtual_dir/bin" "$BASH" "$virtual_session" true
    expect_status 127
    expect_contains "dbus-run-session not found" "$stderr" stderr
}

test_virtual_session_keeps_its_dir_on_request() {
    local kept
    new_virtual_env
    run_with_input '' env PATH="$virtual_dir/bin:$PATH" HOME="$virtual_dir/home" \
        TMPDIR="$virtual_dir/tmp" X4_VIRTUAL_KEEP=1 "$BASH" "$virtual_session" true
    expect_status 0
    kept=$(sed -n 's/^virtual-session: kept //p' <<<"$stderr")
    [[ -n $kept && $kept == "$virtual_dir/tmp/"* ]] || fail "expected the kept dir on stderr, got: $stderr"
    expect_contains "kwin noise" "$(<"$kept/kwin.log")" "kwin.log"
}

# Services the session's D-Bus starts (the desktop portal, ksecretd) outlive KWin and write to the
# session's dirs; the fake KWin leaves such a process behind, with the session's environment.
# The fake KWin leaves behind a process with the session's environment, one that ignores SIGTERM,
# and two decoys that must survive: one whose XDG_DATA_HOME only starts like the session's, and
# one without it, like the owner's own processes.
test_virtual_session_stops_what_the_session_left_running() {
    local name pid
    new_virtual_env
    cat >"$virtual_dir/bin/kwin_wayland" <<EOF
#!/usr/bin/env bash
export "$started_marker"
sleep 300 </dev/null >/dev/null 2>&1 &
echo \$! >"$virtual_dir/leftover.pid"
bash -c 'trap "" TERM; while :; do sleep 1; done' </dev/null >/dev/null 2>&1 &
echo \$! >"$virtual_dir/stubborn.pid"
env XDG_DATA_HOME="\${XDG_DATA_HOME}2" sleep 300 </dev/null >/dev/null 2>&1 &
echo \$! >"$virtual_dir/near-miss.pid"
env -u XDG_DATA_HOME sleep 300 </dev/null >/dev/null 2>&1 &
echo \$! >"$virtual_dir/unrelated.pid"
while ((\$#)); do
    if [[ \$1 == --exit-with-session ]]; then "\$2"; exit 0; fi
    shift
done
EOF
    run_virtual true
    for name in leftover stubborn near-miss unrelated; do
        started_pids+=("$(<"$virtual_dir/$name.pid")")
    done
    expect_status 0
    for name in near-miss unrelated; do
        pid=$(<"$virtual_dir/$name.pid")
        kill -0 "$pid" 2>/dev/null || fail "the $name decoy was stopped"
        kill "$pid"
    done
    for name in leftover stubborn; do
        pid=$(<"$virtual_dir/$name.pid")
        if kill -0 "$pid" 2>/dev/null; then
            fail "the $name process the session left running is still there"
        fi
    done
    [[ -z $(ls -A "$virtual_dir/tmp") ]] || fail "the session dir was left behind: $(ls -A "$virtual_dir/tmp")"
}

# KWin's socket lives in the session's runtime dir under TMPDIR, and a Unix socket's path holds at
# most 107 bytes: a longer TMPDIR is refused with a clear message, before KWin starts.
test_virtual_session_refuses_a_tmpdir_too_long_for_the_socket() {
    local long
    new_virtual_env
    long=$virtual_dir/tmp/$(printf 'd%.0s' {1..80})
    mkdir -p "$long"
    run_with_input '' env PATH="$virtual_dir/bin:$PATH" HOME="$virtual_dir/home" TMPDIR="$long" \
        "$BASH" "$virtual_session" true
    expect_status 2
    expect_contains "more than the 107 a Unix socket allows" "$stderr" stderr
    expect_contains "TMPDIR" "$stderr" stderr
    [[ ! -e $virtual_dir/kwin_wayland.txt ]] || fail "KWin was started"
    [[ -z $(ls -A "$long") ]] || fail "the session dir was left behind: $(ls -A "$long")"
}

test_virtual_session_reports_a_session_that_never_ran() {
    new_virtual_env
    printf '#!/usr/bin/env bash\necho "cannot open the virtual output"\nexit 1\n' >"$virtual_dir/bin/kwin_wayland"
    run_virtual true
    expect_status 125
    expect_contains "the command did not run" "$stderr" stderr
    expect_contains "cannot open the virtual output" "$stderr" "stderr with KWin's log"
}

# The Arch package. makepkg exists only on Arch Linux, so the cases that run it skip elsewhere (as
# on the CI runner); the cases that run the PKGBUILD's functions themselves, the `package` recipe
# with a stand-in makepkg, and the icon checks run everywhere.

# commit_all <repo> <message>: commits everything in <repo>, with a test identity and unsigned.
commit_all() {
    git -C "$1" add -A
    git -C "$1" -c user.name=test -c user.email=test@example.invalid -c commit.gpgsign=false \
        commit -q --allow-empty -m "$2"
}

# The values of <key> in the .SRCINFO that $stdout holds, one per line, in order.
srcinfo_values() { # <key>
    sed -nE "s/^[[:space:]]*$1 = //p" <<<"$stdout"
}

expect_srcinfo() { # <key> <value>...
    local key=$1 expected
    shift
    expected=$(printf '%s\n' "$@")
    [[ $(srcinfo_values "$key") == "$expected" ]] ||
        fail "expected $key to be '$*', got: '$(srcinfo_values "$key" | paste -sd '|')'"
}

# makepkg reads the PKGBUILD as `just package` gives it: a copy, with X4_REPO naming the checkout.
# The owner's makepkg.conf stays out of it.
test_package_build_file_describes_4x4_git_built_from_the_local_master() {
    local dir
    if ! command -v makepkg >/dev/null 2>&1; then
        skip "makepkg not found; it exists only on Arch Linux"
        return
    fi
    dir=$(mktemp -d "$tmp_root/pkgbuild.XXXXXX")
    cp "$pkgbuild" "$dir/PKGBUILD" 2>/dev/null || fail "no PKGBUILD at $pkgbuild"
    run_with_input '' env -C "$dir" XDG_CONFIG_HOME="$dir/config" X4_REPO="$dir/checkout" \
        makepkg --printsrcinfo
    expect_status 0
    expect_srcinfo pkgbase 4x4-git
    expect_srcinfo pkgname 4x4-git
    expect_srcinfo arch x86_64
    expect_srcinfo license 'MIT OR Apache-2.0'
    expect_srcinfo depends glibc libgcc hicolor-icon-theme libxcb libxkbcommon libxkbcommon-x11 \
        vulkan-icd-loader wayland
    expect_srcinfo makedepends cargo git fontconfig freetype2
    expect_srcinfo optdepends \
        'vulkan-driver: draw through Vulkan; without one 4x4 falls back to OpenGL and uses more memory' \
        'xdg-desktop-portal-impl: the file dialog for .torrent files'
    expect_srcinfo provides 4x4
    expect_srcinfo conflicts 4x4
    expect_srcinfo options '!lto' '!debug'
    # The local master branch of the checkout: only what is committed there is built.
    expect_srcinfo source "4x4::git+file://$dir/checkout#branch=master"
    expect_srcinfo b2sums SKIP
}

# Run by hand, without the recipe, makepkg stops at once and names the recipe.
test_package_build_file_stops_without_just_package() {
    local dir
    if ! command -v makepkg >/dev/null 2>&1; then
        skip "makepkg not found; it exists only on Arch Linux"
        return
    fi
    dir=$(mktemp -d "$tmp_root/pkgbuild.XXXXXX")
    cp "$pkgbuild" "$dir/PKGBUILD" 2>/dev/null || fail "no PKGBUILD at $pkgbuild"
    run_with_input '' env -u X4_REPO -C "$dir" XDG_CONFIG_HOME="$dir/config" makepkg --printsrcinfo
    ((status != 0)) || fail "makepkg read the PKGBUILD without X4_REPO: $stdout"
    expect_contains "X4_REPO: run makepkg through just package, which sets X4_REPO" "$stderr" stderr
    [[ $stdout != *pkgname* ]] || fail "makepkg printed a .SRCINFO: $stdout"
}

# pkgver() in a stand-in of makepkg's $srcdir, whose 4x4 is a clone with three commits and a
# version line outside [workspace.package] too.
test_package_version_is_the_workspace_version_with_the_commit_count_and_hash() {
    local dir checkout hash
    dir=$(mktemp -d "$tmp_root/pkgver.XXXXXX")
    checkout=$dir/src/4x4
    mkdir -p "$checkout"
    git -C "$checkout" init -q
    printf '%s\n' '[workspace]' 'members = []' '' '[workspace.package]' 'version = "1.2.3"' \
        'edition = "2024"' '' '[workspace.metadata.other]' 'version = "9.9.9"' >"$checkout/Cargo.toml"
    commit_all "$checkout" "first"
    commit_all "$checkout" "second"
    commit_all "$checkout" "third"
    hash=$(git -C "$checkout" rev-parse --short=7 HEAD)
    run_with_input '' env -C "$dir/src" X4_REPO="$dir" bash -e -c 'source "$1" && pkgver' _ "$pkgbuild"
    expect_status 0
    [[ $stdout == "1.2.3.r3.g$hash" ]] || fail "expected the version 1.2.3.r3.g$hash, got: '$stdout'"
}

# package() in a stand-in of makepkg's $srcdir and $pkgdir, with a stand-in binary and the
# repository's own desktop file, icon and licenses: exactly those five files, with pacman's modes.
test_package_installs_the_binary_desktop_file_icon_and_licenses() {
    local dir checkout listing expected
    dir=$(mktemp -d "$tmp_root/package-files.XXXXXX")
    checkout=$dir/src/4x4
    mkdir -p "$checkout/target/release" "$checkout/packaging/linux" "$dir/pkg"
    printf '#!/bin/sh\n' >"$checkout/target/release/4x4"
    cp "$root/packaging/linux/io.github.lexy898.FourByFour.desktop" "$checkout/packaging/linux/"
    cp "$icon" "$checkout/packaging/linux/" 2>/dev/null || fail "no icon at $icon"
    cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$checkout/"
    run_with_input '' env -C "$dir/src" X4_REPO="$dir" srcdir="$dir/src" pkgdir="$dir/pkg" \
        bash -e -c 'source "$1" && package' _ "$pkgbuild"
    expect_status 0
    listing=$(cd "$dir/pkg" && find . -type f -printf '%m %p\n' | LC_ALL=C sort)
    expected="644 ./usr/share/applications/io.github.lexy898.FourByFour.desktop
644 ./usr/share/icons/hicolor/scalable/apps/io.github.lexy898.FourByFour.svg
644 ./usr/share/licenses/4x4-git/LICENSE-APACHE
644 ./usr/share/licenses/4x4-git/LICENSE-MIT
755 ./usr/bin/4x4"
    [[ $listing == "$expected" ]] || fail "unexpected files in the package: $listing"
}

# `just package` with a stand-in makepkg on PATH, which writes its arguments, its working dir and
# the variables the recipe sets to makepkg.log, keeps the PKGBUILD it was given, rewrites pkgver=
# in it as makepkg does, and lists the package as `--packagelist` does. The checkout has another
# branch checked out, with a commit and an uncommitted change of its own; only master is built.
test_package_recipe_builds_the_committed_master_under_the_cache_dir() {
    local dir repo cache master run
    dir=$(mktemp -d "$tmp_root/package-recipe.XXXXXX")
    repo=$dir/checkout
    cache=$dir/home/.cache/4x4-pkg
    mkdir -p "$dir/bin" "$dir/home" "$repo/packaging/arch"
    cp "$root/justfile" "$repo/"
    printf 'pkgver=0.1.0.r1.gaaaaaaa\n# master\n' >"$repo/packaging/arch/PKGBUILD"
    git -C "$repo" init -q -b master
    commit_all "$repo" "feat: the tip of master"
    master=$(git -C "$repo" log -1 --format='%h %s' master)
    git -C "$repo" switch -q -c feature
    printf 'pkgver=0.1.0.r1.gaaaaaaa\n# feature\n' >"$repo/packaging/arch/PKGBUILD"
    commit_all "$repo" "feat: a feature"
    printf 'pkgver=0.1.0.r1.gaaaaaaa\n# uncommitted\n' >"$repo/packaging/arch/PKGBUILD"
    cat >"$dir/bin/makepkg" <<EOF
#!/usr/bin/env bash
{
    echo "args: \$*"
    echo "pwd: \$PWD"
    for name in X4_REPO BUILDDIR SRCDEST PKGDEST; do echo "\$name: \${!name-unset}"; done
} >>"$dir/makepkg.log"
if [[ \$* == --force ]]; then
    cp PKGBUILD "$dir/built-PKGBUILD"
    sed -i 's/^pkgver=.*/pkgver=0.1.0.r2.gbbbbbbb/' PKGBUILD
elif [[ \$* == --packagelist ]]; then
    echo "\$PKGDEST/4x4-git-\$(sed -n 's/^pkgver=//p' PKGBUILD)-1-x86_64.pkg.tar.zst"
fi
EOF
    chmod +x "$dir/bin/makepkg"
    run_with_input '' env -u XDG_CACHE_HOME HOME="$dir/home" PATH="$dir/bin:$PATH" \
        just --justfile "$repo/justfile" package
    expect_status 0
    expect_contains "building master: $master" "$stdout" stdout
    [[ ${stdout##*$'\n'} == "$cache/packages/4x4-git-0.1.0.r2.gbbbbbbb-1-x86_64.pkg.tar.zst" ]] ||
        fail "expected the package's path last, got: $stdout"
    run="pwd: $cache
X4_REPO: $repo
BUILDDIR: $cache/build
SRCDEST: $cache/sources
PKGDEST: $cache/packages"
    [[ $(<"$dir/makepkg.log") == "args: --force"$'\n'"$run"$'\n'"args: --packagelist"$'\n'"$run" ]] ||
        fail "expected makepkg --force, then --packagelist, in the cache dir; got: $(<"$dir/makepkg.log")"
    [[ $(<"$dir/built-PKGBUILD") == $'pkgver=0.1.0.r1.gaaaaaaa\n# master' ]] ||
        fail "makepkg was not given master's PKGBUILD: $(<"$dir/built-PKGBUILD")"
    [[ $(<"$repo/packaging/arch/PKGBUILD") == $'pkgver=0.1.0.r1.gaaaaaaa\n# uncommitted' ]] ||
        fail "the checkout's PKGBUILD changed: $(<"$repo/packaging/arch/PKGBUILD")"
    [[ -d $cache/build && -d $cache/sources && -d $cache/packages ]] ||
        fail "expected build, sources and packages in $cache: $(ls -A "$cache")"
    [[ $(ls -A "$dir/home") == .cache && $(ls -A "$dir/home/.cache") == 4x4-pkg ]] ||
        fail "the recipe wrote outside ~/.cache/4x4-pkg: $(cd "$dir/home" && find . | paste -sd ' ')"
}

need_python3() {
    command -v python3 >/dev/null 2>&1 || fail "python3 not found; the icon checks need it"
}

# What makes <file> unfit as the app icon, one problem per line: it is not XML, its root is not an
# SVG <svg>, its viewBox is missing or not square, or it has a <text> (whose font differs from
# desktop to desktop), an <image> or a link to anything outside the file.
svg_icon_problems() { # <file>
    python3 -I - "$1" <<'EOF'
import re
import sys
import xml.etree.ElementTree as ET

SVG = "{http://www.w3.org/2000/svg}"
LINKS = ("href", "{http://www.w3.org/1999/xlink}href")

try:
    root = ET.parse(sys.argv[1]).getroot()
except (ET.ParseError, OSError) as error:
    print(f"not XML: {error}")
    sys.exit(0)
if root.tag != SVG + "svg":
    print(f"the root element is {root.tag}, not an SVG <svg>")
try:
    box = [float(value) for value in re.split(r"[\s,]+", root.get("viewBox", "").strip())]
except ValueError:
    box = []
if len(box) != 4 or box[2] <= 0 or box[2] != box[3]:
    print(f"the viewBox is not square: {root.get('viewBox')!r}")
for element in root.iter():
    name = element.tag.removeprefix(SVG)
    if name in ("text", "image"):
        print(f"a <{name}> element")
    for attribute in LINKS:
        link = element.get(attribute)
        if link is not None and not link.startswith("#"):
            print(f"a link outside the file: {link}")
EOF
}

test_icon_is_a_square_svg_without_text_images_or_links() {
    local problems
    need_python3
    problems=$(svg_icon_problems "$icon")
    [[ -z $problems ]] || fail "$icon: $problems"
}

# The icon check itself catches each kind of unfit SVG, and passes a fit one with a link inside it.
test_icon_check_catches_what_does_not_belong_in_an_icon() {
    local dir svg name problem problems
    need_python3
    dir=$(mktemp -d "$tmp_root/icon.XXXXXX")
    svg='<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"'
    printf '%s viewBox="0 0 16 16"><rect id="a" width="16" height="16"/><use href="#a"/></svg>\n' \
        "$svg" >"$dir/fit.svg"
    printf '%s viewBox="0 0 16 16"><rect' "$svg" >"$dir/broken.svg"
    printf '<svg viewBox="0 0 16 16"/>\n' >"$dir/no-namespace.svg"
    printf '%s viewBox="0 0 16 8"/>\n' "$svg" >"$dir/wide.svg"
    printf '%s width="16" height="16"/>\n' "$svg" >"$dir/no-view-box.svg"
    printf '%s viewBox="0 0 16 16"><text>4x4</text></svg>\n' "$svg" >"$dir/text.svg"
    printf '%s viewBox="0 0 16 16"><image href="#a"/></svg>\n' "$svg" >"$dir/image.svg"
    printf '%s viewBox="0 0 16 16"><use href="other.svg#a"/></svg>\n' "$svg" >"$dir/link.svg"
    printf '%s viewBox="0 0 16 16"><use xlink:href="https://example.invalid/a.svg#a"/></svg>\n' \
        "$svg" >"$dir/xlink.svg"
    problems=$(svg_icon_problems "$dir/fit.svg")
    [[ -z $problems ]] || fail "the check rejected a fit icon: $problems"
    while read -r name problem; do
        problems=$(svg_icon_problems "$dir/$name.svg")
        expect_contains "$problem" "$problems" "the problems of $name.svg"
    done <<'EOF'
broken not XML
no-namespace not an SVG <svg>
wide the viewBox is not square
no-view-box the viewBox is not square
text a <text> element
image a <image> element
link a link outside the file: other.svg#a
xlink a link outside the file: https://example.invalid/a.svg#a
EOF
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
    stop_hook_reruns_on_top_readme_change
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
    guard_blocks_substitution_after_ambiguous_close_paren
    guard_blocks_unquoted_heredoc_substitution
    guard_ignores_other_tools
    lint_policy_passes_clean_tree
    lint_policy_rejects_inner_allow_and_expect
    lint_policy_rejects_group_exceptions
    lint_policy_ignores_exceptions_in_comments
    lint_policy_rejects_missing_workspace_lints
    lint_policy_rejects_wrong_dependency_direction
    lint_policy_rejects_registry_gpui_pre_wgpu
    lint_policy_rejects_unused_gpui_pre_wgpu_patch
    lint_policy_rejects_unpatched_vendored_gpui_pre_wgpu
    lint_policy_rejects_failed_gpui_pre_wgpu_lookup
    lint_policy_rejects_an_unsafe_code_exception_elsewhere
    lint_policy_rejects_a_second_unsafe_code_exception_in_the_allocator
    lint_policy_ignores_unsafe_code_in_comments_and_reasons
    lint_policy_rejects_a_workspace_unsafe_level_other_than_deny
    measure_report_line_has_every_metric_in_order
    measure_report_line_flags_each_gl_stack_library
    measure_report_line_rejects_unreadable_metrics
    measure_warnings_are_silent_within_the_targets
    measure_warnings_name_every_breach_on_stderr_and_keep_exit_zero
    measure_warnings_report_one_breach_alone
    measure_warnings_fail_only_when_a_metric_is_unreadable
    measure_cpu_line_counts_user_and_system_time_over_the_window
    measure_cpu_line_rejects_an_unreadable_stat
    measure_elapsed_seconds_reads_either_decimal_separator
    measure_measures_the_synthetic_demo_with_its_arguments_and_cpu
    measure_measures_the_app_by_default
    measure_gives_the_app_a_temp_state_dir
    measure_gives_the_app_an_absolute_state_dir_under_a_relative_tmpdir
    measure_rejects_unknown_options
    virtual_session_propagates_the_exit_code
    virtual_session_gives_kwin_and_the_command_temp_xdg_dirs
    virtual_session_keeps_the_owner_display_and_runtime_dir_away
    virtual_session_copies_linked_settings_as_files
    virtual_session_keeps_arguments_with_spaces
    virtual_session_names_a_missing_tool
    virtual_session_keeps_its_dir_on_request
    virtual_session_stops_what_the_session_left_running
    virtual_session_refuses_a_tmpdir_too_long_for_the_socket
    virtual_session_reports_a_session_that_never_ran
    package_build_file_describes_4x4_git_built_from_the_local_master
    package_build_file_stops_without_just_package
    package_version_is_the_workspace_version_with_the_commit_count_and_hash
    package_installs_the_binary_desktop_file_icon_and_licenses
    package_recipe_builds_the_committed_master_under_the_cache_dir
    icon_is_a_square_svg_without_text_images_or_links
    icon_check_catches_what_does_not_belong_in_an_icon
    check_tools_names_missing_tool
    check_tools_passes_when_all_present
)

for case_name in "${cases[@]}"; do
    skipped=
    "test_$case_name"
    if [[ -n $skipped ]]; then
        echo "skip $case_name: $skipped"
    else
        echo "ok $case_name"
    fi
done
