# 4x4

A native desktop client for a remote [Transmission](https://transmissionbt.com/) daemon, written in Rust.

## Status

Early development — stage 1 (protocol and core). The libraries can talk to a Transmission daemon
and keep track of its torrents, but the app still opens an empty window titled "4x4"; nothing is
usable yet.

## What it will do

- Connect to a Transmission daemon by host, port and RPC path, show the connection state, and
  reconnect automatically. While offline, the last data stays visible and is marked as stale.
- Show the state of every torrent at a glance: a distinct color and icon for downloading, seeding,
  paused, queued, checking and error, with a progress bar in every row.
- Highlight what is active and dim what is idle.
- Sort by clicking a column header; show, hide, reorder and resize columns.
- Filter by state from a sidebar with live counts, and search by name.
- Show torrent details: general information, and a file tree with per-file progress, Low / Normal /
  High priority and the option to skip individual files.
- Add torrents from `.torrent` files (file dialog or drag and drop) and magnet links, choosing the
  destination folder and, for `.torrent` files, which files to download.
- Act as the system handler for `.torrent` files and `magnet:` links.
- Start, pause, start now, verify and reannounce one or several torrents; copy the magnet link or
  hash; use a context menu and keyboard shortcuts for every action.
- Remove torrents safely: removing from the list and removing together with the files are separate
  commands, and deleting files always asks for explicit confirmation.
- Download sequentially and show how much of each file is already playable from its start.
- Show total speeds, free disk space and connection state in a status bar; toggle alternative speed
  limits and set global download and upload limits.
- Follow the system light or dark theme, and remember the window, columns, sorting and filter
  between runs.
- Stay light on memory and gentle on low-powered servers, as a single native executable.

More is planned after that.

## Requirements

- Transmission 4.1 or newer
- Linux (other platforms are not supported yet) with a Wayland or X11 session
- A Vulkan driver (for example `vulkan-radeon`, `vulkan-intel` or `nvidia-utils`). Without one,
  4x4 draws through OpenGL, which uses more memory.

## Build from source

On Arch Linux:

```bash
sudo pacman -S --needed rust just cargo-nextest cargo-deny cargo-machete taplo-cli typos jq
sudo pacman -S --needed base-devel pkgconf fontconfig freetype2 libxkbcommon libxkbcommon-x11 \
    libxcb wayland vulkan-icd-loader
just setup
just run
```

The second command installs the system libraries the GUI toolkit builds and links against; Wayland and
the Vulkan loader are loaded at runtime. `Ctrl+Q` or closing the window quits.

## Performance

`just measure` builds the release binary, starts it, waits 5 seconds and prints one line: the time to
the first rendered frame, then the memory as proportional (`pss_mib`), unique (`uss_mib`),
anonymous (`rss_anon_mib`) and resident (`rss_mib`) size, the thread count, whether Mesa's OpenGL
stack is loaded (`gl_stack_mapped`) and the binary size. It adds a `WARN:` line on stderr when a figure
is above one of its own reference limits (16 MiB private memory, 55 MiB PSS, 120 MiB RSS) or when
the OpenGL stack is mapped, which it should not be where a Vulkan driver is installed. The exit
code stays 0 unless the measurement itself failed.

The honest memory metrics are the private (anonymous, `RssAnon`) memory and the PSS, not the RSS. RSS
counts every resident page the app maps, including shared driver libraries (`libLLVM`, `libgallium`)
that the desktop shell and the browser already keep in memory anyway, so it overstates what 4x4
costs and swings with the page cache. PSS divides each shared page among the processes that map it,
which is the fair share, though it depends on what else is running. Private memory is what 4x4 alone
holds and what only swap can take back. USS (private clean and dirty pages) adds the file pages only
4x4 maps, such as its own code, which the kernel can drop without swap. The RSS stays in the report as
a reference.

4x4 creates its GPU context Vulkan-first: it uses a hardware Vulkan adapter when there is one and
loads the OpenGL driver only when Vulkan cannot draw the window (no Vulkan driver, some virtual
machines, old GPUs). Then it tries hardware OpenGL, and only then software renderers. GPUI itself
asks for Vulkan and OpenGL at once, which keeps Mesa's OpenGL driver (`libgallium`, EGL and seven
driver threads) loaded next to the Vulkan one, so 4x4 builds a patched copy of GPUI's renderer
([`vendor/gpui-pre-wgpu`](vendor/gpui-pre-wgpu/PATCHED.md)) until upstream fixes this
([zed#63327](https://github.com/zed-industries/zed/issues/63327)).

Medians of three runs on 2026-10-28 under the same conditions (KDE Plasma 6 on Wayland, AMD Radeon
780M with RADV, the window on the built-in screen without focus), before and after that change:

| Metric                     | Target              | Vulkan and OpenGL | Vulkan-first         |
| -------------------------- | ------------------- | ----------------- | -------------------- |
| First frame                | ≤ 500 ms            | 210 ms            | 197 ms               |
| Resident memory (RSS)      | ≤ 100 MiB           | 150.0 MiB         | 110.6 MiB (not met)  |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 60.8 MiB          | 48.4 MiB             |
| Unique memory (USS)        | —                   | 51.3 MiB          | 41.6 MiB             |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 17.2 MiB          | 12.6 MiB             |
| Threads                    | —                   | 33                | 26                   |
| Binary size                | —                   | 31.4 MiB          | 31.4 MiB             |

The 100 MiB RSS target is still missed, but RSS mostly counts shared, file-backed pages: RADV
itself links `libLLVM` (about 61 MiB resident in any Vulkan process here, and every Vulkan app maps
it), and the 4x4 binary adds about 24 MiB. PSS shares such pages out among the processes that map
them. 16 MiB of private memory and 55 MiB of PSS are proposed working targets, for the owner to
accept. Without Vulkan, the OpenGL fallback takes about 160 MiB RSS and 26 MiB of private memory.

These figures vary with how much of the shared driver stays resident: on the same laptop under heavy
memory pressure (about 15 GiB in swap), a run showed 61 MiB RSS and the same 12.6 MiB of private
memory.

## Development

Read [`AGENTS.md`](AGENTS.md) before changing anything: it holds the commands, the architecture and
the rules every change must follow.

### Architecture

The workspace has three crates. `x4-rpc` speaks Transmission's JSON-RPC 2.0 protocol over HTTP.
`x4-core` holds the app's model and logic, with no UI code, down to what the main window shows
(table rows, subtitles, filter counts, status bar), so the views only draw it. `x4-app` is the `4x4`
binary with the window.

All network traffic runs on one worker thread, `x4-service`, which `x4-core` starts. It polls the
daemon on a schedule (faster while the window is visible), reconnects with a growing pause when the
daemon stops answering, and carries out the actions the user asks for, one request at a time. The
window (connected to it in the next stage) talks to it through two channels: commands go in, and
snapshots of the state and the results of actions come out. Sending a command never blocks, the
window awaits the events without blocking its thread, and closing it does not wait for the worker
to finish its last request, so the window never waits on the network.

### Live tests

`just test-live` runs the tests in `crates/x4-rpc/tests/live.rs` against a real Transmission 4.1+
daemon; `just test` and `just check` skip them. They never touch existing torrents or the daemon's
settings. One test only reads the server version. Each of the others writes a random 3 MiB file
into a test directory, adds it as a trackerless torrent, works only on that torrent (stop, start,
verify, reannounce, sequential download, file priority, adding it twice), and removes it together
with its file. Every test checks that every other torrent kept its status, sequential mode, file
priorities and wanted files. The torrent and the file are removed even when a test fails.

An interrupted run (Ctrl-C, a killed process) cannot clean up, so each test prints the hash and
the file of its torrent as soon as the daemon has added it (`live test: added torrent <hash> with
data file <path>`). To recover, remove the torrent with that hash in any Transmission client,
choosing to delete its local data. If no such line was printed, the run most likely stopped before
the torrent was added: delete the single `4x4-live-<hex>.bin` file from the test directory instead.
The next run refuses to start while any torrent downloads into the test directory, so a leftover
torrent cannot go unnoticed; remove it the same way.

The test directory is one directory seen from both sides: the daemon reads it under one path and
this machine writes to it under another (for example over NFS), so both must be able to write
there. A test creates it if it is missing (its parent must exist) and then removes it again; a
directory that already existed is left in place. No torrent may already download into it.

| Variable             | Meaning                                  | Default                                    |
| -------------------- | ---------------------------------------- | ------------------------------------------ |
| `X4_LIVE_URL`        | the daemon's RPC endpoint                | `http://192.168.1.5:9091/transmission/rpc` |
| `X4_LIVE_REMOTE_DIR` | the test directory as the daemon sees it | `/mnt/hdd/torrents/4x4-test`               |
| `X4_LIVE_LOCAL_DIR`  | the same directory on this machine       | `/mnt/pi-hdd/torrents/4x4-test`            |

```bash
X4_LIVE_URL=http://my-daemon:9091/transmission/rpc \
X4_LIVE_REMOTE_DIR=/srv/torrents/4x4-test X4_LIVE_LOCAL_DIR=/mnt/daemon/4x4-test just test-live
```

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
