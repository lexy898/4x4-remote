#!/usr/bin/env bash
# Mechanical checks for the AGENTS.md rules that clippy and cargo-deny cannot express.
# Run by `just lint-policy` (part of `just check` and `just pre-commit`).
#
#   (a) no crate-wide lint exceptions: no inner `#![allow(...)]` or `#![expect(...)]` in crates/
#   (b) no exception for a whole clippy lint group or for `warnings`
#   (c) every crates/*/Cargo.toml inherits the workspace lints (`[lints]` `workspace = true`)
#   (d) dependencies point one way: x4-core and x4-rpc never depend directly on GPUI crates,
#       gpui-kit, anyhow or x4-app, and x4-rpc never depends on x4-core
#   (e) the gpui-pre-wgpu patch is applied: x4-app builds the vendored, patched copy in
#       vendor/gpui-pre-wgpu (see its PATCHED.md), not the registry crate
#
# Prints one line per violation (file or crate, and the rule) and exits 1 if there is any.
# Usage: scripts/lint-policy.sh [repository root] (default: the repository of this script).
set -euo pipefail

root=${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
cd "$root"

failures=0

violation() { # <file or crate> <rule>
    echo "lint-policy: $1: $2" >&2
    failures=$((failures + 1))
}

groups='all|pedantic|nursery|restriction|style|complexity|perf|suspicious|correctness|cargo'

# (a) and (b). Attributes may span lines, so each file is flattened to one line first;
# whole-line comments (including doc comments) are dropped so examples in docs do not count.
mapfile -d '' rust_files < <(find crates -type f -name '*.rs' -print0 | LC_ALL=C sort -z)
for file in "${rust_files[@]}"; do
    flat=$(sed -E 's:^[[:space:]]*//.*$::' "$file" | tr '\n' ' ')
    while IFS= read -r attribute; do
        [[ -n $attribute ]] || continue
        violation "$file" "crate-wide lint exception $attribute; use #[expect(lint, reason = \"...\")] on the smallest item"
    done < <(grep -oE '#![[:space:]]*\[[[:space:]]*(allow|expect|cfg_attr[[:space:]]*\([^]]*(allow|expect))[[:space:]]*\([^]]*\]' <<<"$flat" || true)
    while IFS= read -r attribute; do
        [[ -n $attribute ]] || continue
        # The reason text may say anything; only the lint names count.
        names=$(sed -E 's/reason[[:space:]]*=[[:space:]]*"([^"\\]|\\.)*"//g' <<<"$attribute")
        if grep -qE "clippy::($groups)\\b|\\bwarnings\\b" <<<"$names"; then
            violation "$file" "lint exception for a whole lint group or for warnings in $attribute; name the single lint"
        fi
    done < <(grep -oE '#!?[[:space:]]*\[[[:space:]]*(allow|expect|cfg_attr)[[:space:]]*\([^]]*\]' <<<"$flat" || true)
done

# (c)
for manifest in crates/*/Cargo.toml; do
    [[ -f $manifest ]] || continue
    if [[ $(taplo get -f "$manifest" lints.workspace 2>/dev/null || true) != true ]]; then
        violation "$manifest" "missing [lints] workspace = true (every crate inherits the workspace lints)"
    fi
done

# (d)
direct_deps() { # <crate>
    cargo tree -p "$1" -e normal,build --depth 1 --prefix none --locked --target all |
        tail -n +2 | awk 'NF { print $1 }' | LC_ALL=C sort -u
}

check_direction() { # <crate> <forbidden-name-regex> <rule>
    local deps dep
    if ! deps=$(direct_deps "$1"); then
        violation "$1" "cargo tree failed, cannot check the dependency direction"
        return
    fi
    while IFS= read -r dep; do
        if [[ -n $dep && $dep =~ $2 ]]; then
            violation "$1" "depends on $dep; $3"
        fi
    done <<<"$deps"
}

check_direction x4-core '^(gpui(-.*)?|anyhow|x4-app)$' \
    "x4-core has no UI dependencies and returns thiserror errors (x4-app -> x4-core -> x4-rpc)"
check_direction x4-rpc '^(gpui(-.*)?|anyhow|x4-app|x4-core)$' \
    "x4-rpc has no UI dependencies, returns thiserror errors and depends on no other 4x4 crate"

# (e) A gpui-kit bump that brings another gpui-pre-wgpu version leaves the patch unused, and
# Cargo only warns about that while it builds the unpatched registry crate.
check_vendored_patch() {
    local tree warnings package
    local redo='re-vendor it and re-apply the patch (vendor/gpui-pre-wgpu/PATCHED.md)'
    warnings=$(mktemp)
    if ! tree=$(cargo tree -p x4-app -i gpui-pre-wgpu --locked --offline --target all 2>"$warnings"); then
        violation x4-app "cargo tree failed, cannot check that gpui-pre-wgpu is the vendored copy: $(<"$warnings")"
        rm -f "$warnings"
        return
    fi
    if grep -qi 'patch .*gpui-pre-wgpu.* was not used in the crate graph' "$warnings"; then
        violation x4-app "Cargo reports the vendored gpui-pre-wgpu patch as unused; $redo"
    fi
    rm -f "$warnings"
    package=$(head -n 1 <<<"$tree")
    if [[ $package != 'gpui-pre-wgpu v'*' ('*'/vendor/gpui-pre-wgpu)' ]]; then
        violation x4-app "gpui-pre-wgpu resolves to '$package', not vendor/gpui-pre-wgpu; $redo"
    fi
}

check_vendored_patch

if ((failures > 0)); then
    echo "lint-policy: $failures violation(s); see AGENTS.md (Code Style, Architecture, Dependencies)" >&2
    exit 1
fi
