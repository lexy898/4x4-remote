#!/usr/bin/env bash
# Stop / SubagentStop hook: an agent may not finish while `just check-fast` is red.
#
# The check only runs when Rust-relevant files changed since the last green run
# (fingerprint of path + content of *.rs, *.toml, Cargo.lock, *.ftl). A red
# result blocks the stop (exit 2, output on stderr goes to the agent). After
# 3 blocked stops on the same fingerprint the hook gives up (exit 0) so a
# check the agent cannot fix never traps it in a loop.
#
# Env: CLAUDE_PROJECT_DIR (default: $PWD), X4_STOP_CHECK_CMD (default: just check-fast).
set -euo pipefail

project_dir=${CLAUDE_PROJECT_DIR:-$PWD}
state_dir=$project_dir/target/.agent-hooks
check_cmd=${X4_STOP_CHECK_CMD:-just check-fast}
max_blocks=3
output_lines=80

cd "$project_dir"
mkdir -p "$state_dir"

fingerprint() {
    git ls-files -co --exclude-standard -z -- '*.rs' '*.toml' 'Cargo.lock' '*.ftl' |
        LC_ALL=C sort -zu |
        while IFS= read -r -d '' file; do
            # Tracked files deleted from the work tree are still listed; skipping
            # them makes the deletion itself change the fingerprint.
            if [[ -f $file ]]; then
                sha256sum -- "$file"
            fi
        done |
        sha256sum |
        cut -d' ' -f1
}

current=$(fingerprint)
green_file=$state_dir/last-green
blocks_file=$state_dir/blocks

if [[ -f $green_file && $(<"$green_file") == "$current" ]]; then
    exit 0
fi

if output=$(bash -c "$check_cmd" </dev/null 2>&1); then
    echo "$current" >"$green_file"
    rm -f "$blocks_file"
    exit 0
fi

# Red: count consecutive blocked stops for this exact fingerprint.
count=1
if [[ -f $blocks_file ]] && read -r blocked_fingerprint blocked_count <"$blocks_file" &&
    [[ $blocked_fingerprint == "$current" && $blocked_count =~ ^[0-9]+$ ]]; then
    count=$((blocked_count + 1))
fi

if ((count < max_blocks)); then
    echo "$current $count" >"$blocks_file"
    {
        echo "$check_cmd failed — fix before finishing:"
        echo "$output" | tail -n "$output_lines"
    } >&2
    exit 2
fi

rm -f "$blocks_file"
echo "giving up after $max_blocks blocked stops; checks are still red — tell the user" >&2
# stderr is not shown for exit 0; systemMessage on stdout is what reaches the user.
echo '{"systemMessage": "4x4 checks are still red after 3 blocked stops — tell the user before finishing"}'
exit 0
