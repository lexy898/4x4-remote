#!/usr/bin/env bash
# Parsing and threshold logic of scripts/measure.sh, kept apart so that scripts/hooks/test-hooks.sh
# can test it. Sourced, never run: it sets no shell options and starts no process of the app.
#
# Every function reads the files it is given, which are copies of /proc/<pid>/smaps_rollup,
# /proc/<pid>/status and /proc/<pid>/maps, so a test can feed it canned ones.

# Reference limits of `just measure`, in MiB. A breach prints a warning and never changes the
# exit code. They are not the targets in README.md. Private (anonymous) memory and PSS are the
# honest costs of the app; RSS counts every shared file-backed page in full, so the 120 MiB limit
# is looser than the 100 MiB target and a reference only (README.md, "Performance").
RSS_ANON_LIMIT_MIB=16
PSS_LIMIT_MIB=55
RSS_LIMIT_MIB=120

# proc_field <file> <key>: the number on the "<key>: <n> [kB]" line of a /proc status-style file.
# The key must match whole, so "Pss" does not read the "Pss_Dirty" line. Fails, saying why on
# stderr, when the file or the number is missing.
proc_field() {
    local value
    value=$(awk -v key="$2:" '$1 == key { print $2; exit }' "$1") || {
        echo "cannot read $1" >&2
        return 1
    }
    if [[ ! $value =~ ^[0-9]+$ ]]; then
        echo "no numeric '$2:' line in $1" >&2
        return 1
    fi
    echo "$value"
}

# uss_kib <smaps_rollup>: unique set size, the memory freed when the process exits
# (Private_Clean + Private_Dirty).
uss_kib() {
    local clean dirty
    clean=$(proc_field "$1" Private_Clean) || return 1
    dirty=$(proc_field "$1" Private_Dirty) || return 1
    echo $((clean + dirty))
}

# gl_stack_mapped <maps>: yes when Mesa's OpenGL stack (libgallium, libEGL_mesa or libGLX_mesa) is
# mapped, no when it is not; that is the sign of a Vulkan-only process. Matching the file name
# keeps the glvnd dispatch libraries (libEGL, libGLdispatch), which Vulkan also loads, out of it.
gl_stack_mapped() {
    local found=0
    grep -Eq '/lib(gallium|EGL_mesa|GLX_mesa)[^/]*$' "$1" || found=$?
    case $found in
        0) echo yes ;;
        1) echo no ;;
        *)
            echo "cannot read $1" >&2
            return 1
            ;;
    esac
}

# report_line <first_frame_ms> <smaps_rollup> <status> <maps> <binary_bytes>: the one-line report
# of `just measure`. rss_mib is the whole resident set (VmRSS) and only a reference.
report_line() {
    local first_frame_ms=$1 rollup=$2 proc_status=$3 maps=$4 binary_bytes=$5
    local pss uss rss_anon rss threads gl
    pss=$(proc_field "$rollup" Pss) || return 1
    uss=$(uss_kib "$rollup") || return 1
    rss_anon=$(proc_field "$proc_status" RssAnon) || return 1
    rss=$(proc_field "$proc_status" VmRSS) || return 1
    threads=$(proc_field "$proc_status" Threads) || return 1
    gl=$(gl_stack_mapped "$maps") || return 1
    awk -v ms="$first_frame_ms" -v pss="$pss" -v uss="$uss" -v anon="$rss_anon" -v rss="$rss" \
        -v threads="$threads" -v gl="$gl" -v bin="$binary_bytes" 'BEGIN {
        printf "first_frame_ms=%d pss_mib=%.1f uss_mib=%.1f rss_anon_mib=%.1f rss_mib=%.1f threads=%d gl_stack_mapped=%s binary_mib=%.1f\n",
            ms, pss / 1024, uss / 1024, anon / 1024, rss / 1024, threads, gl, bin / 1048576
    }'
}

# warn_above <name> <kib> <limit-mib> <kind>: a WARN line on stderr when kib is over the limit.
warn_above() {
    (($2 > $3 * 1024)) || return 0
    awk -v name="$1" -v kib="$2" -v limit="$3" -v kind="$4" 'BEGIN {
        printf "WARN: %s=%.1f is above the %d MiB %s\n", name, kib / 1024, limit, kind
    }' >&2
}

# report_warnings <smaps_rollup> <status> <maps>: one "WARN: ..." line on stderr per breached
# target. Returns 0 whatever it finds; fails only when a figure cannot be read.
report_warnings() {
    local rollup=$1 proc_status=$2 maps=$3
    local pss rss_anon rss gl
    pss=$(proc_field "$rollup" Pss) || return 1
    rss_anon=$(proc_field "$proc_status" RssAnon) || return 1
    rss=$(proc_field "$proc_status" VmRSS) || return 1
    gl=$(gl_stack_mapped "$maps") || return 1
    warn_above rss_anon_mib "$rss_anon" "$RSS_ANON_LIMIT_MIB" target
    warn_above pss_mib "$pss" "$PSS_LIMIT_MIB" target
    warn_above rss_mib "$rss" "$RSS_LIMIT_MIB" "reference limit"
    if [[ $gl == yes ]]; then
        echo "WARN: gl_stack_mapped=yes: Mesa's OpenGL stack (libgallium, libEGL_mesa or libGLX_mesa) is loaded, which a Vulkan-only process should not need" >&2
    fi
}

# cpu_ticks <stat>: utime + stime, in clock ticks, from a copy of /proc/<pid>/stat: the CPU time
# of every thread of the process so far. The fields are counted after the last ")", because the
# command name before it may hold spaces and parentheses.
cpu_ticks() {
    local line rest fields
    line=$(<"$1") || {
        echo "cannot read $1" >&2
        return 1
    }
    rest=${line##*) }
    # rest starts at field 3 (state); utime and stime are fields 14 and 15.
    read -r -a fields <<<"$rest"
    if [[ ! ${fields[11]:-} =~ ^[0-9]+$ || ! ${fields[12]:-} =~ ^[0-9]+$ ]]; then
        echo "no utime and stime in $1" >&2
        return 1
    fi
    echo $((fields[11] + fields[12]))
}

# cpu_line <stat-before> <stat-after> <seconds> <ticks-per-second>: the CPU line of the report,
# the process's CPU time between the two copies of /proc/<pid>/stat, taken <seconds> apart, as a
# percentage of one core.
cpu_line() {
    local before after
    before=$(cpu_ticks "$1") || return 1
    after=$(cpu_ticks "$2") || return 1
    awk -v ticks=$((after - before)) -v seconds="$3" -v hz="$4" 'BEGIN {
        printf "cpu_percent=%.2f window_s=%.1f\n", ticks / hz / seconds * 100, seconds
    }'
}

# elapsed_seconds <from> <to>: the seconds between two values of $EPOCHREALTIME. Bash writes them
# with the decimal separator of the locale (a comma in Russian, for instance) and awk reads only
# a dot, so a comma is turned into a dot first.
elapsed_seconds() {
    awk -v from="${1/,/.}" -v to="${2/,/.}" 'BEGIN { print to - from }'
}
