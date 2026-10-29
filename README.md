# 4x4

A native desktop client for a remote [Transmission](https://transmissionbt.com/) daemon, written in Rust.

## Status

Early development — stage 3, actions, under way. The window connects to the daemon set in
`config.toml` and shows every torrent in a table, with its state, progress and speeds; a sidebar
filters them by state, with live counts, and a search box finds them by name. The status bar shows
the total download and upload speeds, the free space and the state of the connection; it switches
the alternative speed limits and sets the global download and upload limits. The window remembers
its columns, sorting, filter, theme and size between runs. Several torrents can be selected, and
started, paused, started ahead of the queue, verified, reannounced and removed, from the keyboard,
the toolbar and a context menu; removing asks first, and deleting the files too only after an
explicit confirmation.

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
every column fits at its first width, which leaves room for its usual text: "Downloading", a time
left such as "23 h 59 m", or a header with its sort mark. After that it opens at the size it had
last time, and maximized if it was (see [What 4x4 remembers](#what-4x4-remembers)).

The window has four parts: a toolbar along the top, with the search box, the Start and Pause
buttons and the Remove menu on the left and the theme button and the "Columns" menu on the right;
the filter sidebar down the left side; the torrent table, which takes the rest; and the status bar
along the bottom.

The table lists the torrents, sorted by name at first. Each row shows:

- a colored dot for the state (blue downloading, green seeding, yellow queued or checking, red
  error, grey paused), the name, and under it what the torrent is doing, such as "Seeding · 3 peers
  downloading from you"; an error or a tracker problem is written in red or yellow;
- a progress bar in the state's color, with "63% of 22.9 GiB" under it;
- the size, the state, the download (↓) and upload (↑) speeds, the time left, the share ratio and
  the connected peers. A speed is in the state's color and bold while data moves that way, and
  muted otherwise.

Rows where nothing happens (an idle seeder, a paused or queued torrent) are dimmed, so the active
ones stand out; an error is never dimmed, and a selected row keeps its full highlight. Until the daemon's list of torrents has come, the table
stays blank; after that, when there is nothing to show, it says "No torrents", or "No torrents
match" when the filter or the search hides every torrent.

Click a column header to sort by it; click it again to reverse the order. ▲ or ▼ after the header
shows the order. Size, progress, speeds, ratio and peers sort largest first at the first click;
names sort from A to Z, the time left soonest first (then an unknown time, then "—"), and the state
with errors first and paused torrents last. Drag a header to move the column, and drag the edge of a header to resize it.
"Columns" in the toolbar shows and hides columns; the name column always stays. The sort order and
the columns are remembered between runs.

Click a row or use the Up and Down keys, Page Up, Page Down, Home and End to select a torrent.
Several torrents can be selected: Ctrl+click adds a torrent to the selection or takes it out,
Shift+click selects the range from the last plain click or Ctrl+click to the row clicked,
Shift+Up and Shift+Down grow or shrink that range by a row, and Shift+Home and Shift+End stretch it
to the first or the last torrent. Ctrl+A selects every torrent the list shows, and only those;
Escape or Ctrl+Shift+A selects none. Every selected row has the same
highlight. The selection stays on the same torrents when the list is re-sorted or updated, and an
update never scrolls the list. A selected torrent that the filter or the search hides, or that is
gone from the daemon, leaves the selection, and it is not selected again when it shows again. Tab
moves on from the table to the search box and the buttons. The table has the keyboard focus when
the window opens.

The selected torrents that the list shows are what an action works on, in the order shown; a
torrent the filter or the search hides is never touched. With the table focused:

- `Ctrl+S` starts them, or queues them if the daemon's download queue is full; ▷ in the toolbar
  does the same;
- `Ctrl+P` pauses them; ⏸ in the toolbar does the same;
- `Space` pauses them if any of them runs, and starts them if all are stopped;
- `Ctrl+Shift+S` starts them at once, ahead of the queue;
- `Ctrl+Shift+R` verifies their downloaded data, which reads all of it on the server;
- `Ctrl+R` asks their trackers for more peers;
- `Delete` asks whether to remove them from the list; their downloaded files stay on the server.
  "Remove from list…" in the toolbar's Remove menu (the trash can) does the same;
- `Shift+Delete` asks whether to remove them and delete their files from the server; "Remove and
  delete files…" in the Remove menu does the same;
- `Ctrl+C` copies their magnet links, one per line, in the order shown. The links come from the
  daemon, so they are on the clipboard a moment later, and only if 4x4 still has the focus then;
  otherwise a notification asks for `Ctrl+C` again. A newer copy takes the place of one still
  waiting for the daemon, so the clipboard gets what was asked for last. If the daemon no longer
  has some of the torrents, the other links are copied and a notification says how many are
  missing;
- `Ctrl+Shift+C` copies their info hashes, one per line, in the order shown, at once, also while
  the daemon is not connected.

Right-click a torrent for a menu of all of these: Start, Pause, Start now, Verify and Reannounce;
Copy magnet link and Copy hash; "Remove from list…" and "Remove and delete files…". A right-click
on a torrent outside the selection selects only that torrent, as a file manager does; a
right-click inside the selection keeps it, so the menu acts on every selected torrent. The menu
works on the torrents that were selected when it opened, even if the list changes while it is
open. Each item shows its key, and an item that can do nothing is disabled. A right-click below
the rows opens no menu. While the menu is open, it keeps the keys: the list's keys do nothing to
the torrents (Delete removes nothing, and Home, End, Page Up and Page Down do not move the
selection), and Tab, Shift+Tab, `Ctrl+F` and `/` do not leave it; Up, Down and Enter choose an
item, and Escape closes the menu. Choosing an item gives the keys back to the table.

On Wayland 4x4 itself holds the copied text and hands it to the app that pastes it: the text is
lost when 4x4 quits, unless a clipboard manager keeps it.

Removing always asks first. The dialog names the first five torrents in full, a long name wrapped
over several lines, says how many more there are, and says what becomes of the files: that they
stay on the server, or how much downloaded data will be deleted for good. It removes the torrents
it named when it opened, even if the list changes while it is open. "Remove from list" has Remove
focused, so Enter removes. "Remove and delete files" has Cancel focused: Enter or Space press the
button that has the focus, so they cancel, and only a click on "Delete files", or Tab to it and
Enter, deletes anything. Escape, Cancel or a click beside the dialog close it and remove nothing.
While the dialog is open, Delete and Shift+Delete open no second one, and holding the key opens it
once; a key held down presses no button in the dialog, so an Enter held on a menu item opens the
dialog without confirming it. While the daemon is not connected, Remove and its keys do nothing,
and the confirm button of an open dialog is disabled until the connection is back; Cancel still
works. Closing the dialog gives the keys back to the table.

Holding a key down sends its action once, not again with every repeat of the key. While the action
sent last waits for the daemon's answer, it is not sent again for the same torrents, so a second
press or a double click does nothing more; after a different action, though, the same one goes
again (Pause, Start and Pause again send all three, and the torrents end up paused).

An action is available only when it can do something to at least one of the torrents (Start needs
a paused or failed one, Pause one that is not stopped, Start now a stopped or queued one, and asking
for peers one that is downloading or seeding), and only while the daemon is connected; only copying
the hashes works without it. Otherwise its button and its menu item are disabled and its key does
nothing. The toolbar buttons' tooltips name their keys. An action shows nothing when it works: the
list shows the change at the next update, in a second or two. When it fails, a notification at the
top right says what failed, for how many torrents, and why; it stays until clicked. Each kind of
action has its own notification: a newer failure of a pause replaces the older one, and leaves the
one of a failed verify in place. If the daemon did not answer, the action may have happened all the
same, and the notification says so: the list shows the result. Copying magnet links fails the same
way, with a notification, when the daemon cannot send them or no longer has the torrents; the
clipboard then keeps what it had.

The sidebar on the left filters the list by state:

- All;
- Downloading, also the torrents queued to download;
- Seeding, also the torrents queued to seed;
- Paused;
- Active, the torrents where data moves right now;
- Error, a torrent stopped by an error or one whose tracker reports an error.

Each filter has an icon in the color of its state and the number of torrents it matches. The
numbers count every torrent, whatever the filter and the search show, and appear once the daemon's
list of torrents has come; the Error count turns red when any torrent has an error. Click a filter
to show its torrents; the chosen one is highlighted and remembered between runs.

The search box in the toolbar narrows the list, as you type, to the torrents whose names contain
every word typed, in any order and ignoring case. It works together with the filter. `Ctrl+F` moves
to the search box from anywhere in the window, and `/` from anywhere but a text field (see
[Keys](#keys) for keyboard layouts); `Escape` in the box empties it and goes back to the table. The
search is not remembered between runs.

The status bar at the bottom shows:

- a dot for the connection: green when connected, grey while connecting, yellow while
  reconnecting and red when the daemon is too old (4x4 needs Transmission 4.1 or newer). Next to
  it, "Connecting…" or "Reconnecting (attempt N)… timed out" and the like; the tooltip says more
  about why the daemon could not be reached;
- the turtle, which switches the daemon's alternative speed limits on and off (`Ctrl+T`). It is
  highlighted, in yellow, while they apply, and its tooltip gives them, such as "Alternative speed
  limits: ↓ 50 KiB/s ↑ 50 KiB/s";
- the global speed limits, such as "↓ 500 KiB/s · ↑ no limit". A click on them, or `Ctrl+L`, opens
  a popover that sets them (see below);
- the free space where new torrents are saved;
- the total download and upload speeds, ↓ and ↑.

The speed limits popover has, for the download and the upload limit each, a switch that turns the
limit on and a field for it in KiB/s, as every speed in 4x4 is shown (the daemon keeps limits in
kB/s, 1000 bytes per second; 4x4 converts them). Each field starts with the daemon's value, also
while that limit is off. Typing a speed into a field switches its limit on, and clearing the field
switches it off. `Enter` in a field, or Apply, sends the limits that changed, and only those, and
closes the popover; `Escape`, a click beside it, or moving the focus out of it (`Tab` past Apply,
`Shift+Tab`, `Ctrl+F`) closes it and changes nothing. A limit is a whole number from 1 to 1,000,000
KiB/s: anything else is refused with a note under its field, and nothing is sent; a limit switched
on with an empty field asks for a speed. A key held down to open the popover does not type into it
or apply it. While the context menu is open, `Ctrl+T` and `Ctrl+L` do nothing. The turtle
and the limits are disabled until the daemon has sent its settings, and while it is not connected;
an open popover then keeps what was typed, and Apply waits for the connection. A change that fails
shows a notification, as an action does, one for the turtle and one for each limit.

When the daemon stops answering, the last data stays on screen, with the list, the counts in the
sidebar and the numbers in the status bar dimmed, until it answers again. If the part of 4x4 that
talks to the daemon stops or cannot start, the status bar says "Service stopped" or "Service could
not start", the data is dimmed for good, and the reason is in the log.

The button in the toolbar switches the theme: the system's (light or dark, following the desktop),
light, or dark. The choice is remembered between runs.

While the window is minimized or does not have the focus, 4x4 asks the daemon for updates every
10 seconds instead of every 1.5 seconds, to go easy on a small server.

### Keys

| Key                                                 | Where                     | What it does                                 |
| --------------------------------------------------- | ------------------------- | -------------------------------------------- |
| `Ctrl+F`                                            | anywhere in the window    | moves to the search box and selects its text |
| `/`                                                 | anywhere but a text field | the same as `Ctrl+F`                         |
| `Escape`                                            | in the search box         | empties it and goes back to the table        |
| `Escape`, `Ctrl+Shift+A`                            | in the table              | clear the selection                          |
| `Up`, `Down`, `Page Up`, `Page Down`, `Home`, `End` | in the table              | select a torrent                             |
| `Shift+Up`, `Shift+Down`                            | in the table              | grow or shrink the selected range by a row   |
| `Shift+Home`, `Shift+End`                           | in the table              | stretch the selected range to either end     |
| `Ctrl+A`                                            | in the table              | selects every torrent the list shows         |
| `Ctrl+S`                                            | in the table              | starts the selected torrents                 |
| `Ctrl+P`                                            | in the table              | pauses the selected torrents                 |
| `Space`                                             | in the table              | pauses them if any runs, else starts them    |
| `Ctrl+Shift+S`                                      | in the table              | starts them now, ahead of the queue          |
| `Ctrl+Shift+R`                                      | in the table              | verifies their data                          |
| `Ctrl+R`                                            | in the table              | asks their trackers for more peers           |
| `Delete`                                            | in the table              | asks whether to remove them from the list    |
| `Shift+Delete`                                      | in the table              | asks whether to remove them with their files |
| `Ctrl+C`                                            | in the table              | copies their magnet links                    |
| `Ctrl+Shift+C`                                      | in the table              | copies their info hashes                     |
| `Up`, `Down`, `Enter`                               | in the context menu       | move to an item and choose it                |
| `Escape`                                            | in the context menu       | closes it                                    |
| `Enter`, `Space`                                    | in a removal dialog       | press the button that has the focus          |
| `Escape`                                            | in a removal dialog       | closes it and removes nothing                |
| `Ctrl+T`                                            | anywhere in the window    | switches the alternative speed limits        |
| `Ctrl+L`                                            | anywhere in the window    | opens the speed limits                       |
| `Enter`                                             | in the speed limits       | applies them                                 |
| `Escape`                                            | in the speed limits       | closes them and changes nothing              |
| `Tab`, `Shift+Tab`                                  | anywhere                  | move to the next or the previous control     |
| `Ctrl+Q`                                            | anywhere                  | quits 4x4                                    |

In the search box, `Ctrl+A` selects the text, not the torrents, `Ctrl+C` copies the selected text,
`Space` types a space, and `Delete` deletes a character, never a torrent.
`Ctrl+F` works on any keyboard layout. `/` is matched by the character the key types, not by its
place on the keyboard: on the Russian layout it is the key that types `/` there, not the key marked
`/`, which types a period. Verify is `Ctrl+Shift+R` rather than Transmission's `Ctrl+V`, which is
kept for pasting a magnet link.

## What 4x4 remembers

Between runs 4x4 remembers, in `state.toml`:

- the columns: their order, which are shown and their widths;
- the column the list is sorted by, and the direction;
- the filter chosen in the sidebar;
- the theme chosen with the toolbar button (system, light or dark);
- the size of the window and whether it is maximized. Not its position: on Wayland the desktop
  places windows, and an app can neither read nor set where its window is.

The search text, the selected torrent and the scroll position are not remembered.

4x4 saves a second after the last change, so that dragging a column edge or the window edge writes
the file once, and at once when the window is closed or 4x4 quits (`Ctrl+Q`). Typing in the search
box never writes it. When a save fails (a full disk, a directory 4x4 cannot write to), the window
keeps working: the status bar says "Layout not saved" (its tooltip says what is not saved) until a
save works again, the log gives the reason (again only when it changes), and 4x4 tries again at the
next change and when the window closes.

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
file written by a newer version of 4x4, one that cannot be read, or a corrupt one that cannot be
moved aside is left alone: 4x4 starts from the defaults and saves nothing in that session.

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

`just measure` builds the release binary, starts it with a temporary state directory (so its
`state.toml` and its log go there and are removed afterwards; it still reads `config.toml`), waits
5 seconds and prints one line: the time to the first rendered frame, then the memory as
proportional (`pss_mib`), unique (`uss_mib`), anonymous (`rss_anon_mib`) and resident (`rss_mib`)
size, the thread count, whether Mesa's OpenGL stack is loaded (`gl_stack_mapped`) and the binary
size. It adds a `WARN:` line on stderr when a figure is above one of its own reference limits (16 MiB private memory, 55 MiB PSS, 120 MiB RSS) or when
the OpenGL stack is mapped, which it should not be where a Vulkan driver is installed. The exit
code stays 0 unless the measurement itself failed. `--cpu <seconds>` adds a second line, the CPU
time of all its threads over that many seconds after the memory figures, as a percentage of one
core (`cpu_percent`); `--synthetic` measures the release build of the
[synthetic demo](#synthetic-demo) instead of the app, and arguments after `--` go to the program, as
in `just measure --synthetic --cpu 30 -- --static`.

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

Medians of three runs of the stage 1 window, which was still empty, on 2026-10-28 under the same
conditions (KDE Plasma 6 on Wayland, AMD Radeon 780M with RADV, the window on the built-in screen
without focus), before and after that change:

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

### The main window (stage 2)

Measured on 2026-10-29 on the same laptop, with release builds, inside the headless KWin of
`just virtual` (a 1440 by 900 virtual screen paced at 60 Hz and drawn on the real GPU through RADV;
the window has the focus, so 4x4 polls every 1.5 seconds). The virtual session adds startup time
of its own, so the stage 1 window was measured again there: 387 ms to its first frame, against 197
ms on the desktop. Compare these columns with each other, not with the table above. Medians of
three runs, except the owner's daemon, run once to keep its load low. The targets are those of the
app with a real daemon; the 2000-torrent columns also carry the synthetic daemon (see below), and
"not met" marks every cell over its target:

| Metric                     | Target              | Stage 1 window | Owner's daemon      | 2000 torrents, idle | 2000 torrents, busy |
| -------------------------- | ------------------- | -------------- | ------------------- | ------------------- | ------------------- |
| First frame                | ≤ 500 ms            | 387 ms         | 465 ms              | 486 ms              | 494 ms              |
| Resident memory (RSS)      | ≤ 100 MiB           | 70.2 MiB       | 79.4 MiB            | 98.9 MiB            | 100.0 MiB           |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 51.8 MiB       | 60.7 MiB (not met)  | 80.5 MiB (not met)  | 81.6 MiB (not met)  |
| Unique memory (USS)        | —                   | 43.2 MiB       | 52.0 MiB            | 71.7 MiB            | 72.8 MiB            |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 13.7 MiB       | 17.4 MiB (not met)  | 37.3 MiB (not met)  | 38.6 MiB (not met)  |
| Threads                    | —                   | 32             | 34                  | 33                  | 33                  |
| CPU over 30 s, one core    | near 0 % when idle  | —              | 0.10 %              | 0.03 %              | 2.23 %              |
| Binary size                | —                   | 31.4 MiB       | 36.6 MiB            | 36.2 MiB            | 36.2 MiB            |

The owner's daemon had 6 torrents, all seeding without traffic (`just virtual just measure --cpu
30`, the release build of `4x4`). The 2000 torrents are the [synthetic demo](#synthetic-demo), the
release build of its `x4-synthetic` binary, measured after a 10 second settle
(`X4_MEASURE_SETTLE_SECONDS=10 just virtual just measure --synthetic --cpu 30`, with
`-- --static` for idle); while busy, 513 of them change their rates or progress on every poll. The
synthetic daemon runs inside the measured process, so its own work, making up the answers, counts
in those two columns.

Much of the startup time the virtual session adds comes from Mesa's shader cache: the session
starts with an empty `XDG_CACHE_HOME`, so RADV compiles every shader of the first start. A second
start of the demo in the same session, with the cache the first one left, drew its first frame
133 ms sooner (medians of three sessions: 490 ms, then 357 ms), while with Mesa's cache disabled
(`MESA_SHADER_CACHE_DISABLE=true`) the second start was not faster (434 ms, then 457 ms). With the
cache warm the demo also had about 7 MiB less PSS and RSS and 6 fewer threads, so the figures above
are those of a first start.

- Stage 2 adds 78 ms to the first frame, 3.7 MiB of private memory and 8.9 MiB of PSS to the empty
  stage 1 window, and the binary grew by 5.2 MiB. The proposed private-memory and PSS targets are
  missed by 1.4 and 5.7 MiB.
- 2000 torrents add 20.1 MiB of private memory to the same demo with 6 torrents (17.2 MiB), both
  release builds of `x4-synthetic`. 15.7 MiB of it is the memory allocator's arena of the thread
  that talks to the daemon: what one read of the whole list took at its peak (the synthetic
  daemon's JSON, the parsing of it and the torrents made from it), which the allocator keeps for
  the next read rather than returning. The main thread's heap, where the window keeps its rows and
  their text, grows by 2.8 MiB, and smaller mappings by 1.6 MiB. A real daemon would not add the
  synthetic daemon's part; that has not been measured.
- With nothing changing, the window costs no CPU to speak of: 0.03 % with the synthetic daemon and
  0.10 % with the owner's, which is asked for news every 1.5 seconds. It redraws only when
  something it shows has changed.
- Scrolling 2000 torrents by one row per frame for 10 seconds (`--scroll` of the release build,
  `cargo run --release -p x4-app --features synthetic --bin x4-synthetic -- --scroll`, six runs,
  idle and busy; a debug build takes about 115 ms per frame) gave 3601 intervals between frames,
  with a median of 16.67 to 16.68 ms, a 99th percentile of 17.4 to 17.6 ms in each run and a
  longest of 18.0 ms. No interval reached two frames (33 ms) on the virtual output's 60 Hz; the
  real screen's pace was not measured. Many intervals (43 %) are just over 16.7 ms only because a
  60 Hz pace jitters around 16.67 ms. GPUI's own timing of each frame's drawing and presenting
  (`ZED_MEASUREMENTS=1`) had a median of 7.5 ms and a 99th percentile of 14.3 ms; 3 of 3619 frames
  took over 16.7 ms, the longest 19.2 ms.
- In 2 minutes against the owner's daemon (a debug build), 4x4 sent 169 requests, 1.4 per second:
  the recently active torrents and the session totals every 1.5 seconds, the free space every 30
  seconds, and the whole list and the session settings every minute.

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

### Windows off screen

`just virtual <command>` runs a command inside a headless KWin with a D-Bus session and XDG
directories of its own, runtime directory included, and without your desktop's `DISPLAY`, so the
windows it opens show on no monitor, take no focus and leave the desktop's settings alone;
`spectacle -b -n -f -o /tmp/shot.png` inside it captures the virtual screen. Whatever the session
started is stopped when it ends. For example, `just virtual just measure` measures the app without a
window on your screens. It needs KDE Plasma (`kwin_wayland` and `dbus-run-session`) and works
nowhere else.

### Synthetic demo

`just demo-synthetic` opens the main window over a synthetic daemon of 2000 torrents in every
state, made up in memory: nothing goes over the network, and none of 4x4's files is read or written
(the log goes to stderr only). The rates of the active torrents change on every poll. `--static`
keeps every rate at 0, an idle daemon; `--scroll` scrolls the table by one row per frame for 10
seconds, logs the intervals between the frames and quits; `--select N` selects the row at index `N`
(from 0) once the list is there, and `--extend K` then extends the selection `K` rows down, as
Shift+Down would. The made-up daemon carries out the actions on its own torrents: pausing,
starting (a torrent that lacks data downloads, a complete one seeds), starting ahead of the queue,
verifying (the torrent checks for three updates, then goes back to what it was), removing and
the speed settings. `--press KEY`, which can be given several times, presses each key in turn
once the list is there and selected, one every three seconds, as on the keyboard: for example
`just virtual just demo-synthetic --static --select 5 --press ctrl-p --press space` pauses the
sixth torrent and starts it again, and `--press shift-delete` opens the dialog that would delete
its files. `just virtual just demo-synthetic --scroll` runs it off
screen. The demo is the `x4-synthetic` binary of the `synthetic` feature, so the `4x4` build
does not contain it.

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
