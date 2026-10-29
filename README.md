# 4x4

A native desktop client for a remote [Transmission](https://transmissionbt.com/) daemon, written in Rust.

## Status

Early development — stage 2 (the main window, read-only), in progress. The window connects to the
daemon set in `config.toml` and shows every torrent in a table, with its state, progress and
speeds, and the total download and upload speeds, the free space and the state of the connection in
its status bar. The filters and the search come next. 4x4 only reads from the daemon so far; it
changes nothing there.

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

## The main window

The window opens at once, before the daemon has answered, and connects in the background; nothing
in the window ever waits for the network. The first time it opens at 1200 by 720 pixels, where
every column fits.

The table lists the torrents, sorted by name at first. Each row shows:

- a colored dot for the state (blue downloading, green seeding, yellow queued or checking, red
  error, grey paused), the name, and under it what the torrent is doing, such as "Seeding · 3 peers
  downloading from you"; an error or a tracker problem is written in red or yellow;
- a progress bar in the state's color, with "63% of 22.9 GiB" under it;
- the size, the state, the download (↓) and upload (↑) speeds, the time left, the share ratio and
  the connected peers. A speed is in the state's color and bold while data moves that way, and
  muted otherwise.

Rows where nothing happens (an idle seeder, a paused or queued torrent) are dimmed, so the active
ones stand out; an error is never dimmed. Until the daemon's list of torrents has come, the table
stays blank; after that, when there is nothing to show, it says "No torrents", or "No torrents
match" when the filter hides every torrent.

Click a column header to sort by it; click it again to reverse the order. ▲ or ▼ after the header
shows the order. Size, progress, speeds, ratio and peers sort largest first at the first click;
names sort from A to Z, the time left soonest first, and the state with errors first and paused
torrents last. Drag a header to move the column, and drag the edge of a header to resize it.
"Columns" in the toolbar shows and hides columns; the name column always stays.

Click a row or use the Up and Down keys, Page Up, Page Down, Home and End to select a torrent;
Escape clears the selection. The selection stays on the same torrent when the list is re-sorted or
updated, and an update never scrolls the list. Tab moves on from the table to the buttons. The
table has the keyboard focus when the window opens.

The status bar at the bottom shows:

- a dot for the connection: green when connected, grey while connecting, yellow while
  reconnecting and red when the daemon is too old (4x4 needs Transmission 4.1 or newer). Next to
  it, "Connecting…" or "Reconnecting (attempt N)… timed out" and the like; the tooltip says more
  about why the daemon could not be reached;
- "Alt speed" while the daemon's alternative speed limits apply;
- the free space where new torrents are saved;
- the total download and upload speeds, ↓ and ↑.

When the daemon stops answering, the last data stays on screen, with the list and the numbers in the
status bar dimmed, until it answers again. If the part of 4x4 that talks to the daemon stops or
cannot start, the status bar says "Service stopped" or "Service could not start", the data is
dimmed for good, and the reason is in the log.

The button in the toolbar switches the theme: the system's (light or dark, following the desktop),
light, or dark. It is not remembered between runs yet.

While the window is minimized or does not have the focus, 4x4 asks the daemon for updates every
10 seconds instead of every 1.5 seconds, to go easy on a small server.

## Files and logs

4x4 keeps its files in the XDG directories (`$XDG_CONFIG_HOME` and `$XDG_STATE_HOME`, which default
to `~/.config` and `~/.local/state`):

| File                                | What it is                                                              |
| ----------------------------------- | ----------------------------------------------------------------------- |
| `~/.config/4x4/config.toml`         | Your settings. 4x4 only reads it and never changes it.                  |
| `~/.local/state/4x4/state.toml`     | What 4x4 remembers between runs. It is safe to delete.                  |
| `~/.local/state/4x4/4x4.<date>.log` | The log, one file per day (UTC date); the last 7 daily files are kept.  |

`config.toml` is optional. It holds the daemon's address; only plain `http://` is supported, and a
missing key takes the default:

```toml
[connection]
url = "http://192.168.1.5:9091/transmission/rpc"
```

4x4 reads it at startup and connects to that address. A file it cannot use is reported in the log
and in the status bar ("Config file not used"; the tooltip says why, and "Dismiss" hides it), and
the defaults are used; neither ever shows the URL, which may hold a password.

4x4 writes `state.toml` atomically, so a crash cannot leave half a file. If the file cannot be
parsed, 4x4 moves it to `state.toml.bad` (replacing an older one) and starts from the defaults. A
file written by a newer version of 4x4 is left alone, and 4x4 does not save over it.

The log lines also go to stderr. `RUST_LOG` sets the level for both (for example
`RUST_LOG=x4_app=debug`); without it 4x4 logs its own lifecycle events and the problems of its
libraries. If the log directory cannot be used, 4x4 logs to stderr only and still starts.

Debug builds (`just run`) give the window the Wayland app id `io.github.lexy898.FourByFour.Devel`
instead of `io.github.lexy898.FourByFour`, so window rules made for test windows (for example,
open on another screen without taking focus) never catch the window you actually use. Set
`X4_DEVEL=1` to give a release build the same id; `just measure` does.

## Localization

The interface is in English. Every text the app shows is a message in
[`crates/x4-app/i18n/en/x4.ftl`](crates/x4-app/i18n/en/x4.ftl), written in
[Fluent](https://projectfluent.org/), which has plural rules for each language built in ("1 peer",
"3 peers"). The messages are embedded into the binary. Misspelling a message id in the code is a
compile error, and so is misspelling an argument name in a call that passes arguments. A call
without arguments is not checked for them, so messages with variables are fetched only through
the helpers in `crates/x4-app/src/i18n.rs`, which have tests.

Another language is one more file, `crates/x4-app/i18n/<code>/x4.ftl` (for example `ru`), with the
same messages as the English one. A test checks every language directory: the file must parse,
define the same messages as `en` and use the same `$variables` in each of them, so a language that
misses a message fails the test. English is only the safety net at run time, should a message be
missing anyway. Picking the language from the system locale is not wired in yet, so only English is
used.

Sizes, speeds and ratios are formatted by the app and handed to the messages as ready text, because
Fluent does not format numbers by the rules of a language. The decimal separator of a language is
the message `number-decimal-separator` (a dot in English).

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
binary with the window and the Fluent messages that word everything it shows.

All network traffic runs on one worker thread, `x4-service`, which `x4-core` starts. It polls the
daemon on a schedule (faster while the window is visible), reconnects with a growing pause when the
daemon stops answering, and carries out the actions the user asks for, one request at a time. The
window talks to it through two channels: commands go in, and snapshots of the state and the results
of actions come out. Sending a command never blocks, the window awaits the events without blocking
its thread, and closing it does not wait for the worker to finish its last request, so the window
never waits on the network. When several snapshots queue up while the window is busy, it shows only
the newest, and it redraws only when what it shows has changed, so an idle daemon costs no frames.

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
