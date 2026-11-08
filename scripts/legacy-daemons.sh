#!/usr/bin/env bash
# Runs old Transmission daemons in Docker, for tests of the legacy protocol. Run by
# `just legacy-daemons up|down|status|env <name>`.
#
# Three daemons (v3.00, v4.0.5, v4.1.0-beta.2) and a seeder run on the Docker network x4-legacy.
# Each daemon publishes its RPC port on 127.0.0.1 only, runs as the invoking user and keeps its
# state in <state>/<name>/{config,downloads}. The seeder makes a test torrent from random files
# and every daemon downloads it from the seeder over LPD (local peer discovery): DHT and port
# forwarding are off and the torrent's tracker refuses at once, so nothing leaves the machine.
# The seeder is limited to 16 kB/s, so the daemons keep downloading, and stay connected, for long.
#
#   up           build the images, start everything, wait until each daemon has a peer
#   status       per daemon: the version it reports and its connected peers
#   env <name>   export lines for the live tests (X4_LIVE_URL, X4_LIVE_REMOTE_DIR,
#                X4_LIVE_LOCAL_DIR), to `eval`
#   down         remove the containers, the network and the state directory
#
# Env: X4_LEGACY_DIR, the state directory (default ~/.cache/x4-scratch/legacy-daemons); `down`
# removes it, so without X4_LEGACY_DIR it refuses any other path.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
default_state=$HOME/.cache/x4-scratch/legacy-daemons
state=${X4_LEGACY_DIR:-$default_state}
network=x4-legacy
peer_timeout=180

# name:image tag:base image:loopback port (empty port: no RPC published)
daemons=(
    "v3.00:v3.00:debian:12:19300"
    "v4.0.5:v4.0.5:ubuntu:24.04:19405"
    "v4.1.0-beta.2:v4.1.0-beta.2:debian:13:19412"
)
names=(v3.00 v4.0.5 v4.1.0-beta.2)

usage() {
    echo "usage: legacy-daemons.sh up|down|status|env <name>" >&2
    exit 2
}

port_of() { # <name>
    local entry
    for entry in "${daemons[@]}"; do
        [[ $entry == "$1":* ]] && echo "${entry##*:}" && return 0
    done
    echo "unknown daemon: $1 (one of: ${names[*]})" >&2
    return 1
}

base_of() { # <name>
    local entry rest
    for entry in "${daemons[@]}"; do
        if [[ $entry == "$1":* ]]; then
            rest=${entry#*:*:}
            echo "${rest%:*}"
            return 0
        fi
    done
}

write_settings() { # <name> [extra json lines]
    local dir=$state/$1/config
    mkdir -p "$dir" "$state/$1/downloads"
    cat >"$dir/settings.json" <<'JSON'
{
    "rpc-enabled": true,
    "rpc-bind-address": "0.0.0.0",
    "rpc-port": 9091,
    "rpc-whitelist-enabled": false,
    "rpc-host-whitelist-enabled": false,
    "rpc-authentication-required": false,
    "download-dir": "/downloads",
    "incomplete-dir-enabled": false,
    "dht-enabled": false,
    "lpd-enabled": true,
    "pex-enabled": true,
    "port-forwarding-enabled": false,
    "blocklist-enabled": false,
    "start-added-torrents": true,
JSON
    if [[ $1 == seeder ]]; then
        cat >>"$dir/settings.json" <<'JSON'
    "speed-limit-up": 16,
    "speed-limit-up-enabled": true
}
JSON
    else
        cat >>"$dir/settings.json" <<'JSON'
    "speed-limit-up-enabled": false
}
JSON
    fi
}

start_container() { # <name> <image tag> [<port>]
    local name=$1 tag=$2 port=${3:-} publish=()
    [[ -n $port ]] && publish=(-p "127.0.0.1:$port:9091")
    docker rm -f "x4-legacy-$name" >/dev/null 2>&1 || true
    docker run -d --name "x4-legacy-$name" --network "$network" \
        --user "$(id -u):$(id -g)" "${publish[@]}" \
        -v "$state/$name/config:/config" -v "$state/$name/downloads:/downloads" \
        "x4-legacy:$tag" transmission-daemon -f -g /config >/dev/null
}

remote() { # <name> <transmission-remote argument>...
    local name=$1
    shift
    docker exec "x4-legacy-$name" transmission-remote localhost:9091 "$@"
}

wait_for_rpc() { # <name>
    local i
    for ((i = 0; i < 60; i++)); do
        remote "$1" -si >/dev/null 2>&1 && return 0
        sleep 1
    done
    echo "x4-legacy-$1 did not answer on its RPC port in 60 s" >&2
    return 1
}

peers_of() { # <name>: the number of connected peers, 0 when unknown
    local line
    line=$(remote "$1" -t all -i 2>/dev/null | grep -m1 'Peers: connected to' || true)
    line=${line##*connected to }
    echo "${line%%,*}" | grep -E '^[0-9]+$' || echo 0
}

up() {
    local entry name tag base port
    for entry in "${daemons[@]}"; do
        name=${entry%%:*}
        base=$(base_of "$name")
        docker build --quiet --build-arg "BASE_IMAGE=$base" -t "x4-legacy:$name" \
            "$here/legacy-daemons" >/dev/null
    done
    docker network inspect "$network" >/dev/null 2>&1 || docker network create "$network" >/dev/null

    for name in "${names[@]}" seeder; do
        write_settings "$name"
    done
    for entry in "${daemons[@]}"; do
        name=${entry%%:*}
        port=${entry##*:}
        start_container "$name" "$name" "$port"
    done
    start_container seeder v4.1.0-beta.2
    for name in "${names[@]}" seeder; do
        wait_for_rpc "$name"
    done

    # A tracker that refuses at once: the torrent has tracker entries and nothing leaves the host.
    docker exec "x4-legacy-seeder" sh -c '
        set -e
        cd /downloads
        mkdir -p x4-fixture
        for i in 1 2 3 4; do
            [ -f "x4-fixture/file$i.bin" ] || head -c 16777216 /dev/urandom >"x4-fixture/file$i.bin"
        done
        transmission-create -t http://127.0.0.1:1/announce -o x4-fixture.torrent x4-fixture
    ' >/dev/null
    remote seeder -a /downloads/x4-fixture.torrent -w /downloads >/dev/null
    for name in "${names[@]}"; do
        cp "$state/seeder/downloads/x4-fixture.torrent" "$state/$name/downloads/x4-fixture.torrent"
        remote "$name" -a /downloads/x4-fixture.torrent >/dev/null
    done

    local deadline=$((SECONDS + peer_timeout)) missing
    while :; do
        missing=
        for name in "${names[@]}"; do
            (($(peers_of "$name") >= 1)) || missing="$missing $name"
        done
        [[ -z $missing ]] && break
        if ((SECONDS >= deadline)); then
            echo "no LPD peer within ${peer_timeout} s for:$missing (do multicast packets cross the" \
                "Docker bridge $network? try: docker network inspect $network)" >&2
            return 1
        fi
        sleep 3
    done
    status
}

status() {
    local name version
    for name in "${names[@]}"; do
        version=$(remote "$name" -si 2>/dev/null | grep -m1 'Daemon version' || true)
        version=${version#*: }
        echo "$name  version ${version:-unknown}  peers $(peers_of "$name")"
    done
}

env_lines() { # <name>
    local port
    port=$(port_of "$1")
    echo "export X4_LIVE_URL=http://127.0.0.1:$port/transmission/rpc"
    echo "export X4_LIVE_REMOTE_DIR=/downloads/4x4-test"
    echo "export X4_LIVE_LOCAL_DIR=$state/$1/downloads/4x4-test"
}

down() {
    if [[ -z ${X4_LEGACY_DIR:-} && $state != "$HOME/.cache/x4-scratch/"* ]]; then
        echo "refusing to remove $state: outside $HOME/.cache/x4-scratch" >&2
        return 1
    fi
    local name
    for name in "${names[@]}" seeder; do
        docker rm -f "x4-legacy-$name" >/dev/null 2>&1 || true
    done
    docker network rm "$network" >/dev/null 2>&1 || true
    rm -rf -- "$state"
}

case ${1:-} in
up) up ;;
status) status ;;
down) down ;;
env)
    [[ $# -eq 2 ]] || usage
    env_lines "$2"
    ;;
*) usage ;;
esac
