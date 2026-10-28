#!/usr/bin/env bash
# PostToolUse hook (Edit|Write|MultiEdit): format the Rust file the agent just
# touched if it lies inside the project. Reads the hook JSON on stdin. Exit 2 hands
# rustfmt's complaint (usually a syntax error) back to the agent.
#
# rustfmt cannot format a file that declares a module whose file does not exist yet
# (`mod x;` written before x.rs). That is a normal step while adding a module, so the
# file is left as it is (exit 0); a later edit formats it.
#
# Env: CLAUDE_PROJECT_DIR (default: $PWD).
set -euo pipefail

path=$(jq -r '.tool_input.file_path // empty')

if [[ $path != *.rs || ! -f $path ]]; then
    exit 0
fi

project_dir=$(realpath -- "${CLAUDE_PROJECT_DIR:-$PWD}")
file=$(realpath -- "$path")
if [[ $file != "$project_dir"/* ]]; then
    exit 0
fi

if ! output=$(rustfmt --edition 2024 "$file" 2>&1); then
    if [[ -n $output ]] && ! grep -qv '^Error writing files: failed to resolve mod `' <<<"$output"; then
        exit 0
    fi
    echo "rustfmt failed on $path:" >&2
    echo "$output" >&2
    exit 2
fi
