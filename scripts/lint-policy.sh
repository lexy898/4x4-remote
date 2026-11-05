#!/usr/bin/env bash
# Mechanical checks for the AGENTS.md rules that clippy and cargo-deny cannot express.
# Run by `just lint-policy` (part of `just check` and `just pre-commit`).
#
#   (a) no crate-wide lint exceptions: no inner `#![allow(...)]` or `#![expect(...)]` in crates/
#   (b) no exception for a whole clippy lint group or for `warnings`
#   (c) every crates/*/Cargo.toml inherits the workspace lints (`[lints]` `workspace = true`)
#   (d) dependencies point one way: x4-core and x4-rpc never depend directly on GPUI crates,
#       gpui-kit, anyhow or x4-app, and x4-rpc never depends on x4-core
#   (e) the gpui-pre-wgpu patch is applied: x4-app builds the vendored copy in
#       vendor/gpui-pre-wgpu, not the registry crate, and that copy is patched (see its PATCHED.md)
#   (f) the one `unsafe`: an `unsafe_code` lint attribute only in crates/x4-app/src/allocator.rs, at
#       most once, as an `#[expect]` on a `let` (not on a whole function or module, where more
#       unsafe calls would pass), that file holds exactly one `unsafe` keyword (clippy limits a
#       block to one operation, but not a statement to one block), and the workspace level of
#       `unsafe_code` is `deny` (`forbid` cannot be excepted) and of
#       `multiple_unsafe_ops_per_block` is `warn` or `deny`
#   (g) the links of the Markdown files git tracks (but those in vendor/ and .specs/) resolve: a
#       link or image with a relative target names a file or directory that exists, and an
#       `#anchor`, alone or after a `.md` target, a heading of that file, as GitHub makes anchors
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

# The reason text may say anything; only the lint names count.
strip_reasons() {
    sed -E 's/reason[[:space:]]*=[[:space:]]*"([^"\\]|\\.)*"//g'
}

groups='all|pedantic|nursery|restriction|style|complexity|perf|suspicious|correctness|cargo'
unsafe_home=crates/x4-app/src/allocator.rs

# (a) and (b). Attributes may span lines, so each file is flattened to one line first. A `//`
# comment is dropped (a whole line, doc comments included, or the end of a line of code), so
# examples in docs and notes after an attribute do not count. The `//` must start outside a
# string: one in a string, such as a URL in a `reason`, hides nothing. A `/* */` comment is not
# dropped; between an attribute and its `let` it counts as code, a false violation on the safe side.
mapfile -d '' rust_files < <(find crates -type f -name '*.rs' -print0 | LC_ALL=C sort -z)
for file in "${rust_files[@]}"; do
    flat=$(sed -E 's:^(([^"/]|/[^/]|"([^"\\]|\\.)*")*)//.*$:\1:' "$file" | tr '\n' ' ')
    while IFS= read -r attribute; do
        [[ -n $attribute ]] || continue
        violation "$file" "crate-wide lint exception $attribute; use #[expect(lint, reason = \"...\")] on the smallest item"
    done < <(grep -oE '#![[:space:]]*\[[[:space:]]*(allow|expect|cfg_attr[[:space:]]*\([^]]*(allow|expect))[[:space:]]*\([^]]*\]' <<<"$flat" || true)
    while IFS= read -r attribute; do
        [[ -n $attribute ]] || continue
        names=$(strip_reasons <<<"$attribute")
        if grep -qE "clippy::($groups)\\b|\\bwarnings\\b" <<<"$names"; then
            violation "$file" "lint exception for a whole lint group or for warnings in $attribute; name the single lint"
        fi
    done < <(grep -oE '#!?[[:space:]]*\[[[:space:]]*(allow|expect|cfg_attr)[[:space:]]*\([^]]*\]' <<<"$flat" || true)
    # (f) Any level counts: a `forbid` or `deny` of unsafe_code beside the workspace's would be
    # another place that decides about unsafe code.
    unsafe_attributes=0
    while IFS= read -r attribute; do
        [[ -n $attribute ]] || continue
        names=$(strip_reasons <<<"$attribute")
        if grep -qE '\bunsafe_code\b' <<<"$names"; then
            unsafe_attributes=$((unsafe_attributes + 1))
        fi
    done < <(grep -oE '#!?[[:space:]]*\[[[:space:]]*(allow|expect|warn|deny|forbid|cfg_attr)[[:space:]]*\([^]]*\]' <<<"$flat" || true)
    if ((unsafe_attributes > 0)) && [[ $file != "$unsafe_home" ]]; then
        violation "$file" "unsafe_code exception outside $unsafe_home; the owner approved one unsafe call, there"
    elif ((unsafe_attributes > 1)); then
        violation "$file" "$unsafe_attributes unsafe_code exceptions; the owner approved one unsafe call"
    elif ((unsafe_attributes == 1)); then
        # The attribute covers what it is put on, so it goes on the statement of the one call. The
        # reasons are stripped first, so that a `]` in one does not end the attribute early.
        stripped=$(strip_reasons <<<"$flat")
        if ! grep -qE '#[[:space:]]*\[[[:space:]]*expect[[:space:]]*\([^]]*\bunsafe_code\b[^]]*\][[:space:]]*let[[:space:]]' <<<"$stripped"; then
            violation "$file" "the unsafe_code exception must be an #[expect] on the let of the one unsafe call"
        else
            # Comments and reasons are gone, and `unsafe_code` is no match (`_` is a word character).
            unsafe_keywords=$({ grep -oE '\bunsafe\b' <<<"$stripped" || true; } | wc -l)
            if ((unsafe_keywords != 1)); then
                violation "$file" "$unsafe_keywords 'unsafe' keywords; the owner approved one unsafe call"
            fi
        fi
    fi
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
# Cargo only warns about that while it builds the unpatched registry crate. A re-vendored copy
# without the patch re-applied resolves fine but loads OpenGL again, so the patched renderer
# function must be there too.
check_vendored_patch() {
    local tree warnings package renderer=vendor/gpui-pre-wgpu/src/wgpu_renderer.rs
    local redo='re-vendor it and re-apply the patch (vendor/gpui-pre-wgpu/PATCHED.md)'
    if ! grep -qsE 'fn create_context_vulkan_first\b' "$renderer"; then
        violation "$renderer" \
            "has no create_context_vulkan_first, so the vendored copy is not patched; $redo"
    fi
    warnings=$(mktemp)
    if ! tree=$(cargo tree -p x4-app -i gpui-pre-wgpu --locked --target all 2>"$warnings"); then
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

# (f) `forbid` cannot be overridden, not even by #[expect], so the workspace level is `deny`.
unsafe_level=$(taplo get -f Cargo.toml workspace.lints.rust.unsafe_code 2>/dev/null || true)
if [[ $unsafe_level != deny ]]; then
    violation Cargo.toml "workspace.lints.rust.unsafe_code is '${unsafe_level:-unset}', not deny (the one unsafe call needs #[expect], which a forbid rejects, and any other level lets more unsafe in)"
fi

# Clippy counts the operations of one block; with the keyword count above, one statement cannot
# hold a second unsafe call either.
unsafe_ops_level=$(taplo get -f Cargo.toml workspace.lints.clippy.multiple_unsafe_ops_per_block 2>/dev/null || true)
if [[ $unsafe_ops_level != warn && $unsafe_ops_level != deny ]]; then
    violation Cargo.toml "workspace.lints.clippy.multiple_unsafe_ops_per_block is '${unsafe_ops_level:-unset}', not warn or deny (an unsafe block may hold only the one call)"
fi

# (g) Text moved between README.md and docs/ takes its links along, and a link that no longer
# resolves fails nowhere else. markdown_items prints one line per heading and per link of a
# Markdown file, outside fenced code blocks:
#   heading <TAB> <line> <TAB> <its text, without code marks, link targets and HTML tags>
#   link <TAB> <line> <TAB> <target>
# The lines of a paragraph are joined first, so that a link may wrap, and code spans are blanked,
# so that a `[text](target)` in one is no link. It works on bytes (LC_ALL=C), so that any awk
# gives the same; md_slug lowercases the heading text by the letters of Unicode.
markdown_items() { # <file>
    LC_ALL=C awk '
        function spaces(n,    s) { s = ""; while (n-- > 0) s = s " "; return s }
        # The text with each code span blanked to spaces of its length; a run of backticks
        # without a closing run of the same length stays as it is.
        function blank_code(s,    out, p, n, run, after, q, m, skipped) {
            out = ""
            while ((p = index(s, "`")) > 0) {
                out = out substr(s, 1, p - 1)
                s = substr(s, p)
                n = 0
                while (substr(s, n + 1, 1) == "`") n++
                run = substr(s, 1, n)
                after = substr(s, n + 1)
                skipped = 0
                while ((q = index(after, run)) > 0) {
                    m = 0
                    while (substr(after, q + m, 1) == "`") m++
                    if (m == n) break
                    skipped += q + m - 1
                    after = substr(after, q + m)
                }
                if (q > 0) {
                    out = out spaces(n + skipped + q - 1 + n)
                    s = substr(s, n + skipped + q + n)
                } else {
                    out = out run
                    s = substr(s, n + 1)
                }
            }
            return out s
        }
        # A target, without the title after it or the <> around it.
        function link(at, target) {
            sub(/^[ \t\n]+/, "", target)
            if (substr(target, 1, 1) == "<") {
                target = substr(target, 2)
                sub(/>.*/, "", target)
            } else {
                sub(/[ \t\n].*/, "", target)
            }
            if (target != "") printf "link\t%d\t%s\n", at, target
        }
        # The links of the paragraph gathered so far, each with the line it starts on.
        function flush(    text, offset, k, at, target) {
            if (count == 0) return
            text = blank_code(para)
            offset = 0
            while (match(text, /!?\[[^]]*\]\([^)]*\)/)) {
                offset += RSTART
                at = lines[1]
                for (k = 2; k <= count; k++) if (starts[k] <= offset) at = lines[k]
                target = substr(text, RSTART, RLENGTH)
                sub(/^!?\[[^]]*\]\(/, "", target)
                sub(/\)$/, "", target)
                link(at, target)
                offset += RLENGTH - 1
                text = substr(text, RSTART + RLENGTH)
            }
            para = ""
            count = 0
        }
        function add_line(line) {
            if (count > 0) para = para "\n"
            count++
            starts[count] = length(para) + 1
            lines[count] = NR
            para = para line
        }
        # The fence a line opens, three or more backticks or tildes, or "".
        function fence_of(line,    c, n) {
            sub(/^[ \t]*/, "", line)
            c = substr(line, 1, 1)
            if (c != "`" && c != "~") return ""
            n = 0
            while (substr(line, n + 1, 1) == c) n++
            if (n < 3 || (c == "`" && index(substr(line, n + 1), "`") > 0)) return ""
            return substr(line, 1, n)
        }
        function closes(line, fence,    c, n) {
            sub(/^[ \t]*/, "", line)
            sub(/[ \t]*$/, "", line)
            c = substr(fence, 1, 1)
            n = 0
            while (substr(line, n + 1, 1) == c) n++
            return n >= length(fence) && n == length(line)
        }
        # The number of the # marks that make a line a heading (1 to 6), or 0.
        function heading_level(line) {
            if (!match(line, /^ ? ? ?#+([ \t]|$)/)) return 0
            sub(/^ */, "", line)
            match(line, /^#+/)
            return RLENGTH <= 6 ? RLENGTH : 0
        }
        # A heading as GitHub shows it, which its anchor is made of.
        function heading_text(line,    out, item) {
            sub(/^ *#+[ \t]*/, "", line)
            sub(/[ \t]+#+[ \t]*$/, "", line)
            sub(/^#+[ \t]*$/, "", line)
            sub(/[ \t]+$/, "", line)
            gsub(/`/, "", line)
            gsub(/<[^>]*>/, "", line)
            out = ""
            while (match(line, /!?\[[^]]*\]\([^)]*\)/)) {
                out = out substr(line, 1, RSTART - 1)
                item = substr(line, RSTART, RLENGTH)
                sub(/^!?\[/, "", item)
                sub(/\]\(.*$/, "", item)
                out = out item
                line = substr(line, RSTART + RLENGTH)
            }
            return out line
        }
        fence != "" {
            if (closes($0, fence)) fence = ""
            next
        }
        fence_of($0) != "" {
            flush()
            fence = fence_of($0)
            next
        }
        /^[ \t]*$/ { flush(); next }
        heading_level($0) > 0 {
            flush()
            printf "heading\t%d\t%s\n", NR, heading_text($0)
            add_line($0)
            flush()
            next
        }
        # A link reference definition, "[label]: target"; not a footnote, "[^1]: text".
        /^ ? ? ?\[[^]^][^]]*\]:/ {
            flush()
            target = $0
            sub(/^[^]]*\]:/, "", target)
            link(NR, target)
            next
        }
        { add_line($0) }
        END { flush() }
    ' "$1"
}

# GitHub's anchor of a heading: lowercase, with only letters, digits, spaces, - and _ kept, and
# each space a -.
md_slug() { # <heading text>: sets md_slug_value
    local LC_ALL=C.UTF-8
    local text=${1,,}
    text=${text//[^[:alnum:] _-]/}
    md_slug_value=${text// /-}
}

declare -A md_anchors=() md_scanned=()
md_links=()

# Records the anchors of a file's headings in md_anchors, as "<file>#<anchor>", where a slug met
# before gets -1, -2 and so on, as on GitHub; with `links`, appends its links to md_links too.
md_scan() { # <file> [links]
    local file=$1 kind line text base slug count
    local -A seen=()
    while IFS=$'\t' read -r kind line text; do
        if [[ $kind == link ]]; then
            [[ -z ${2-} ]] || md_links+=("$file"$'\t'"$line"$'\t'"$text")
            continue
        fi
        md_slug "$text"
        base=$md_slug_value
        slug=$base
        # The keys carry a prefix: a heading of no letters has an empty slug, and no array takes
        # an empty key.
        while [[ -n ${seen["s:$slug"]+set} ]]; do
            count=$((${seen["s:$base"]} + 1))
            seen["s:$base"]=$count
            slug=$base-$count
        done
        seen["s:$slug"]=0
        md_anchors["$file#$slug"]=1
    done < <(markdown_items "$file")
    md_scanned[$file]=1
}

check_markdown_links() {
    local listing file line target path anchor resolved dir entry
    if ! listing=$(git -c core.quotePath=false ls-files -- '*.md' ':(exclude)vendor/' ':(exclude).specs/'); then
        violation "$root" "git ls-files failed, cannot check the links of the Markdown files"
        return
    fi
    while IFS= read -r file; do
        # A file deleted from the work tree but not yet from the index has no links left.
        [[ -n $file && -f $file ]] || continue
        md_scan "$file" links
    done <<<"$listing"
    for entry in "${md_links[@]}"; do
        IFS=$'\t' read -r file line target <<<"$entry"
        # A URL (https:, mailto: and the like) is not looked at.
        if [[ $target =~ ^[A-Za-z][A-Za-z0-9+.-]*: || $target == //* ]]; then
            continue
        fi
        path=${target%%#*}
        path=${path%%\?*}
        anchor=
        [[ $target != *#* ]] || anchor=${target#*#}
        if [[ -z $path ]]; then
            resolved=$file
        else
            # A path that starts with / is taken from the top of the repository, as on GitHub.
            if [[ $path == /* ]]; then
                resolved=${path#/}
            else
                dir=.
                [[ $file != */* ]] || dir=${file%/*}
                resolved=$dir/$path
                resolved=${resolved#./}
            fi
            if [[ ! -e $resolved ]]; then
                violation "$file:$line" "link ($target): $resolved does not exist"
                continue
            fi
        fi
        # An anchor of another kind of file, such as a line of code, is not looked at.
        [[ -n $anchor && $resolved == *.md && -f $resolved ]] || continue
        [[ -n ${md_scanned[$resolved]+set} ]] || md_scan "$resolved"
        if [[ -z ${md_anchors["$resolved#$anchor"]+set} ]]; then
            violation "$file:$line" "link ($target): $resolved has no heading with the anchor #$anchor"
        fi
    done
}

check_markdown_links

if ((failures > 0)); then
    echo "lint-policy: $failures violation(s); see AGENTS.md (Code Style, Architecture, Dependencies)" >&2
    exit 1
fi
