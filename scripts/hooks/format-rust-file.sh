#!/usr/bin/env bash
# PostToolUse hook (Edit|Write|MultiEdit): format the Rust file the agent just
# touched. Reads the hook JSON on stdin. Exit 2 hands rustfmt's complaint
# (usually a syntax error) back to the agent.
set -euo pipefail

path=$(jq -r '.tool_input.file_path // empty')

if [[ $path != *.rs || ! -f $path ]]; then
    exit 0
fi

if ! output=$(rustfmt --edition 2024 "$path" 2>&1); then
    echo "rustfmt failed on $path:" >&2
    echo "$output" >&2
    exit 2
fi
