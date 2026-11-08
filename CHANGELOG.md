# Changelog

All notable changes to 4x4 Remote are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). What the version number promises: the
`config.toml` file, the command line (files, magnet links, `--version`) and the desktop entry.

## [1.0.0] - Unreleased

First release.

### Added

- A native desktop client for a remote Transmission daemon (3.00 or newer), written in Rust on GPUI,
  for Linux with Wayland or X11. It draws through Vulkan and falls back to OpenGL.
- Connection to the daemon set in a Connection dialog (host, port, path, and a user name and
  password where the daemon asks for them), with a Test button; switching daemons without a
  restart; automatic reconnection; the last data stays visible, dimmed, while the daemon does not
  answer.
- A table of every torrent with a state dot, progress bar, size, speeds, time left, ratio and peers;
  sorting, and showing, hiding, reordering and resizing columns; a sidebar that filters by state
  with live counts; search by name; idle torrents are dimmed so the active ones stand out.
- Start, pause, start now, verify, reannounce and remove, for one or several torrents, from the
  keyboard, the toolbar and a context menu. Removing asks first, and deleting the files needs an
  explicit confirmation. Copy magnet links and info hashes.
- A details panel with four tabs. General: paths, size, hash, dates, share ratio, the torrent's
  creator, comment and pieces, and the worst tracker problem in plain words. Files: a tree with
  per-file progress, Low, Normal and High priority, skipping files, sequential download (which
  needs Transmission 4.1 or newer on the server; on an older one the switch is disabled and says
  so) and how much of each file already plays from its start. Peers: who the torrent is connected
  to, with country flags. Trackers: each tracker's state, last and next announce and counts.
- Adding torrents from `.torrent` files (the desktop's file dialog, drag and drop, or launch
  arguments) and magnet links (from the clipboard or a form), choosing the folder on the server
  (with its free space and the folders used lately), the files to download and whether to start at
  once.
- One 4x4 per user: starting it again, as a file manager or a browser does, hands its files and
  links to the one that runs. It offers itself as an app for `.torrent` files and `magnet:` links,
  and never makes itself their default.
- A status bar with the total speeds, the free space and the connection; the alternative speed
  limits (the turtle) and the global download and upload limits.
- A system, light or dark theme. The window size, columns, sorting, filter, theme and details panel
  are remembered between runs.
- Light on a small server: updates are asked for every 1.5 seconds while the window has the focus
  and every 10 seconds while it does not, and a torrent's details are read only while they are
  shown.
- `4x4 --version`.
- A log on stderr and in a daily file (the last 7 daily files are kept); `config.toml` (readable by
  you only) and `state.toml` in the XDG directories.
- The Arch Linux package `4x4-remote-git`, built from GitHub's `master`
  (`packaging/arch/PKGBUILD`).
- The interface is in English and ready for translations (Fluent messages).
