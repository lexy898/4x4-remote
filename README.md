# 4x4

A native desktop client for a remote [Transmission](https://transmissionbt.com/) daemon, written in Rust.

## Status

Early development — stage 0 (project foundation). The app opens a window titled "4x4"; nothing
is usable yet.

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
- A Vulkan driver (for example `vulkan-radeon`, `vulkan-intel` or `nvidia-utils`)

## Build from source

On Arch Linux:

```bash
sudo pacman -S --needed just cargo-nextest cargo-deny cargo-machete taplo-cli typos
sudo pacman -S --needed base-devel pkgconf fontconfig freetype2 libxkbcommon libxkbcommon-x11 \
    libxcb wayland vulkan-icd-loader
just setup
just run
```

The second command installs the system libraries the GUI toolkit builds and links against; Wayland and
the Vulkan loader are loaded at runtime. `Ctrl+Q` or closing the window quits.

## Performance

`just measure` builds the release binary, starts it, waits 5 seconds and prints the time to the
first rendered frame, the resident memory (RSS) and the binary size. Medians of three runs on
2026-10-28 (KDE Plasma 6.8 on Wayland, AMD Radeon 780M with RADV):

| Metric          | Target     | Measured  |
| --------------- | ---------- | --------- |
| First frame     | ≤ 500 ms   | 140 ms    |
| Resident memory | ≤ 100 MiB  | 170.2 MiB |
| Binary size     | —          | 31.4 MiB  |

The memory target is missed: about 115 MiB of the RSS is Mesa's OpenGL driver (`libgallium` and
`libLLVM`), which GPUI maps while probing the OpenGL adapter at startup even though it renders
with Vulkan. These are shared, file-backed pages; the app's private (anonymous) memory is about
17 MiB.

## Development

Read [`AGENTS.md`](AGENTS.md) before changing anything: it holds the commands, the architecture and
the rules every change must follow.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
