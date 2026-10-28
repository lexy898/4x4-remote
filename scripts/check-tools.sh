#!/usr/bin/env bash
# Verify the tools the justfile and the hooks need. Run by `just setup` and by
# .githooks/pre-commit. Prints every missing tool and how to install it.
set -euo pipefail

tools=(cargo rustfmt just cargo-nextest cargo-deny cargo-machete taplo typos jq)
# Arch packages for the tools above; cargo and rustfmt come with the Rust toolchain.
pacman_packages="cargo-nextest cargo-deny cargo-machete taplo-cli typos just jq"

missing=()
for tool in "${tools[@]}"; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        missing+=("$tool")
        echo "missing: $tool"
    fi
done

if ((${#missing[@]} > 0)); then
    echo "install with: sudo pacman -S --needed $pacman_packages"
    for tool in "${missing[@]}"; do
        if [[ $tool == cargo || $tool == rustfmt ]]; then
            echo "cargo and rustfmt come with the Rust toolchain: install rustup (https://rustup.rs) or the pacman package rust"
            break
        fi
    done
    exit 1
fi
