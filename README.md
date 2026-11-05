# 4x4 Remote

4x4 Remote (4x4 for short; its command is `4x4`) is a native desktop client for a remote
[Transmission](https://transmissionbt.com/) daemon, written in Rust.

## Status

Early development — stage 8, the connection. The window connects to the Transmission daemon set in
its Connection dialog, with a user name and password when the daemon asks for them, and switches to
another daemon without a restart; with none set, it says so and opens the dialog. It shows every
torrent in a table, with its state, progress and speeds; a sidebar filters them by state, with live
counts, and a search box finds them by name. The status bar shows the total download and upload
speeds, the free space and the state of the connection; it switches the alternative speed limits and
sets the global download and upload limits. The window remembers its columns, sorting, filter, theme
and size between runs. Several torrents can be selected, and started, paused, started ahead of the
queue, verified, reannounced and removed, from the keyboard, the toolbar and a context menu;
removing asks first, and deleting the files too only after an explicit confirmation. A panel under
the table shows the selected torrent in four tabs. General gives where its data is, its size, hash,
dates, share ratio and metainfo, and how its trackers are doing, with the worst problem in plain
words. Files shows its files as a tree, with each one's progress and how much of it already plays
from the start; each file or folder can be skipped or given a Low, Normal or High priority, and the
torrent can be downloaded in order. Peers lists who the torrent is connected to, with their
countries, and Trackers lists each tracker with its state, its last and next announce and the counts
it knows, to see why a torrent has few peers or is stuck. Whether the panel is shown, its height and
its tab are remembered too. Torrents are added from `.torrent` files, chosen in the desktop's file
dialog or dropped on the window, and from magnet links on the clipboard, in a dialog that picks the
folder on the server (with its free space and the folders used lately), the files to download and
whether to start at once. One 4x4 runs for each user: starting it again, as with a file or a link,
hands them to the 4x4 that runs. On Arch Linux, 4x4 installs as a package built from this checkout
and shows in the application menu with its icon; one command makes it the default app for `.torrent`
files and magnet links, and undoes that again (see [Install](#install)).

## What it will do

- Connect to a Transmission daemon by host, port and RPC path, with a user name and password when
  it asks for them, show the connection state, and reconnect automatically. While offline, the
  last data stays visible and is marked as stale.
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

The second command installs the system libraries the GUI toolkit builds and links against; Wayland
and the Vulkan loader are loaded at runtime. `Ctrl+Q` or closing the window quits.

## Install

On Arch Linux, 4x4 installs as the package `4x4-git`, built from the local `master` branch of this
checkout, so build it after what you want is merged into `master`:

```bash
git switch master
git pull
just package
sudo pacman -U ~/.cache/4x4-pkg/packages/4x4-git-<version>-1-x86_64.pkg.tar.zst
```

`just package` builds the package with `makepkg` (from `packaging/arch/PKGBUILD`) and prints the
path of the file it made as its last line; that file is what `pacman -U` installs. The version
names the commit it was built from, such as `0.1.0.r231.g1a2b3c4` (the workspace version, the
number of commits and the commit). The build needs the tools and libraries of
[Build from source](#build-from-source); makepkg names any package that is missing, and
`just package` never installs anything itself. The first build takes about four minutes.
Everything it makes stays in `~/.cache/4x4-pkg` (crates come from cargo's own cache, `~/.cargo`):
the build dir keeps the compiled crates, about 2 GiB, so later builds are faster, and all of it is
safe to delete.

The package is always built from the commits of the local `master` branch: not from the working
tree, so uncommitted changes are left out, and not from the branch checked out; the first line
`just package` prints names the commit it builds. While `master` has no `packaging/arch/PKGBUILD`,
`just package` stops before it writes anything and says to merge and pull first. Outside a git
repository (a copy of the files without `.git`) it also stops before it writes anything, but it
shows git's message and does not mention merging. If the local `master` is behind the
`origin/master` you last fetched, `just package` warns and still builds the local `master`; it
fetches nothing itself. After the checkout has moved to another directory, the next build starts
over from it and takes the full four minutes again. The package installs `/usr/bin/4x4`, its
desktop file in `/usr/share/applications` and its icon in `/usr/share/icons/hicolor`, so 4x4 Remote
shows in the application menu with its icon at once, without logging out. It also installs a second,
hidden desktop file for debug builds (described below). It depends on `geoip-database`, which holds
the country files that the peers' flags come from (see [Files and logs](#files-and-logs)), and
installs, next to the licenses of 4x4, the notice of the flag pictures it embeds
(`LICENSE-country-flag-icons` in `/usr/share/licenses/4x4-git`; see [License](#license)).

To update, run the same four commands again; pacman replaces the installed version.

To uninstall, first run `just default-handler --undo` if you made 4x4 the default app for
`.torrent` files and magnet links (described below), then `sudo pacman -R 4x4-git`. Your
`config.toml`, `state.toml` and logs stay (see [Files and logs](#files-and-logs)). The build files
in `~/.cache/4x4-pkg`, about 2 GiB, can be deleted: `rm -rf ~/.cache/4x4-pkg`.

To open `.torrent` files and magnet links with 4x4, in Dolphin and in a browser, make it their
default app once after installing:

```bash
just default-handler
```

It shows what `application/x-bittorrent` (`.torrent` files) and `x-scheme-handler/magnet` (magnet
links) open with now, asks on the terminal whether to go on, runs `xdg-mime default` for both and
prints what they open with afterwards. `xdg-mime` is the standard tool that records the choice in
`~/.config/mimeapps.list`, the file where your desktop keeps which app opens which kind of file. It
adds a line for each of the two types under `[Default Applications]`: two lines. On KDE it also
adds the two types under `[Added Associations]`, four lines in all, and then runs `kbuildsycoca6`,
which rebuilds KDE's cache of the installed applications (in `~/.cache`). Nothing else changes. A
browser may ask once more the first time it meets a magnet link, offering 4x4 as the system
default. The command needs 4x4 installed, refuses to run as root, and refuses without a terminal
unless `--yes` answers the question. If a list for your desktop (such as
`~/.config/kde-mimeapps.list`) still names another app, it says so and exits 1; the lines it wrote
stay in `mimeapps.list`, and `--undo` removes them.

`just default-handler --undo` gives them back: after asking, it removes the entries that name 4x4
for these two types from those two sections, whoever made them (an entry you made for 4x4 by hand
goes too). A section header that the first command had to add and that is empty afterwards goes
as well. The rest of the file stays as it was, byte for byte. Both types then open with what the
system picks again (Deluge, for example). It works after uninstalling too: the entries stay in the
file until it runs. If `mimeapps.list` already named another app for either type under
`[Default Applications]`, the first command replaced that line, and the undo cannot bring it
back: `xdg-mime default <app>.desktop <type>` sets that app again.

The installed 4x4 and a debug build (`just run`) run side by side. The debug build has its own app
id (`io.github.lexy898.FourByFour.Devel`), so neither hands its files and links to the other, and
window rules for the one never catch the other. But both read and write the same `config.toml` and
`state.toml`, and the same log: whichever saves last decides the server, the window size, columns
and filter the next start has. A server saved in one does not switch the other while it runs.

KDE and GNOME take a window's icon from the desktop file named after its app id, so the package also
installs `io.github.lexy898.FourByFour.Devel.desktop`, which gives the windows of `just run` the
same icon in the title bar and the task manager. It is hidden from the menu and search and opens
no files or links. Its command runs the installed 4x4 under the development id (`X4_DEVEL=1`), which
hands over to a running debug build, as a second start of 4x4 does, or opens a window of its own
when none runs. Until the package is installed, debug windows show the desktop's generic icon.

The icon, a gearshift pattern, is one SVG, `packaging/linux/io.github.lexy898.FourByFour.svg`,
installed into `hicolor/scalable/apps`, which serves every size. A separate drawing for small
sizes would be installed under the same name into `16x16/apps`, `22x22/apps`, `24x24/apps` and
`32x32/apps` of `hicolor` too, four more `install` lines in the PKGBUILD's `package()`; KDE takes
the icon of a fixed size when one matches the size it draws, and the scalable one for every other.

## Connection

4x4 talks to one Transmission daemon at a time: the server set in the Connection dialog. The
toolbar's Connection button (the gear on the right, before the theme button), `Ctrl+,` from
anywhere in the window and a click on the connection dot in the status bar open it. It never opens
over another dialog, and `Ctrl+,` does nothing while a menu or the speed limits are open. There is
no built-in server: the first time 4x4 starts, no server is set, so it asks nothing of any daemon,
the status bar says "No server set" (its tooltip says where to set one) and the dialog opens at
once. So it does when `config.toml` cannot be used (see [Files and logs](#files-and-logs)).

The dialog has the fields Host (a name or an IP address; an IPv6 address with or without its
brackets), Port (9091 at first, Transmission's own), Path (`/transmission/rpc` at first) and, for a
daemon that asks for a login, User name and Password; for one that does not, leave both empty. The
password is masked, and the eye at the end of its field shows it and hides it again. Under the
fields a muted line shows the address they make, such as
`http://192.168.1.10:9091/transmission/rpc`, which never holds the user name or the password. A
whole address pasted into Host, such as `http://alice:secret@nas:9091/transmission/rpc`, `nas:9091`
or `nas:9091/transmission/rpc`, is spread over the fields at once; a typed one is spread when the
focus leaves Host, or at Save or Test. Only plain `http://` works: an `https://` address is refused
with "HTTPS is not supported yet". The port is a number from 1 to 65535, the path cannot hold `?`,
`#` or spaces, and a user name cannot hold a colon. The user name and the password are taken as
typed, spaces included.

Test asks the server for its version once (one `session_get`, given up after 5 seconds without an
answer), off the window's thread, so that the window never waits. The button says "Testing…" until
the answer comes; a line under the fields then says, in green, "Connected: Transmission 4.1.3
(…)", or in red why not: "The server cannot be reached", "The server did not answer in time", "The
server rejected the credentials", "The server does not accept requests from this computer" (the
daemon's whitelist), "Transmission 4.1 or newer required", and the like. Any edit takes the line
away, and drops the answer of a test still running, so that the line always speaks of the fields as
they are. Save needs no test.

Save, or `Enter` in a field, checks the fields first: each problem is shown under its field, the
focus goes to the first one, and nothing is saved. With the server the window has already, Save
only closes the dialog. Otherwise it writes the server to `config.toml` and switches to it at once,
without a restart: 4x4 stops talking to the old daemon, forgets all it showed of it (the list, the
selection, the details panel, the numbers in the status bar, the notifications, the speed limits,
and the actions and copies of magnet links still waiting for its answers) and connects to the new
one. What you arranged stays: the columns, the sorting, the filter, the search, the theme and the
details panel with its tab, and so do the torrents waiting to be added. If the file cannot be
written, the dialog stays open and says why, and nothing changes. Cancel or `Escape` closes the
dialog and changes nothing; a click beside it does nothing, so that what was typed is not lost by a
slip. While the dialog is open, the torrents to add wait for it.

The user name and the password are kept in `config.toml` as plain text, in a file only you can read
(mode 0600); 4x4 uses no keyring. They go with every request (HTTP basic authentication), and over
plain `http://` the network carries them unencrypted, as it does for any Transmission client without
https. The log names a server as `host:port` and never shows the password. Once connected, the
dot's tooltip names the server, such as "Connected to nas:9091".

## The main window

The window opens at once, before the daemon has answered, and connects in the background; nothing
in the window ever waits for the network. With no server set, it connects to nothing and opens the
Connection dialog (see [Connection](#connection)). The first time it opens at 1200 by 720 pixels,
where every column fits at its first width, which leaves room for its usual text: "Downloading", a
time left such as "23 h 59 m", or a header with its sort mark. After that it opens at the size it
had last time, and maximized if it was (see [What 4x4 remembers](#what-4x4-remembers)).

The window has five parts: a toolbar along the top, with the search box, the Add file and Add
magnet buttons, the Start and Pause buttons and the Remove menu on the left and the Connection
button, the theme button and the "Columns" menu on the right;
the filter sidebar down the left side; the torrent table, which takes the rest; the details panel
under the table; and the status bar along the bottom.

The table lists the torrents, sorted by name at first. Each row shows:

- a colored dot for the state (blue downloading, green seeding, yellow queued or checking, red
  error, grey paused), the name, and under it what the torrent is doing, such as "Seeding · 3 peers
  downloading from you"; an error or a tracker problem is written in red or yellow;
- a progress bar in the state's color, with "63% of 22.9 GiB" under it;
- the size, the state, the download (↓) and upload (↑) speeds, the time left, the share ratio and
  the connected peers. A speed is in the state's color and bold while data moves that way, and
  muted otherwise.

Rows where nothing happens (an idle seeder, a paused or queued torrent) are dimmed, so the active
ones stand out; an error is never dimmed, and a selected row keeps its full highlight. Until the
daemon's list of torrents has come, the table stays blank; after that, when there is nothing to
show, it says "No torrents", or "No torrents match" when the filter or the search hides every
torrent.

Click a column header to sort by it; click it again to reverse the order. ▲ or ▼ after the header
shows the order. Size, progress, speeds, ratio and peers sort largest first at the first click;
names sort from A to Z, the time left soonest first (then an unknown time, then "—"), and the state
with errors first and paused torrents last. Drag a header to move the column, and drag the edge of a
header to resize it. "Columns" in the toolbar shows and hides columns; the name column always stays.
The sort order and the columns are remembered between runs.

Click a row or use the Up and Down keys, Page Up, Page Down, Home and End to select a torrent.
Several torrents can be selected: Ctrl+click adds a torrent to the selection or takes it out,
Shift+click selects the range from the last plain click or Ctrl+click to the row clicked, Shift+Up
and Shift+Down grow or shrink that range by a row, and Shift+Home and Shift+End stretch it to the
first or the last torrent. Ctrl+A selects every torrent the list shows, and only those; Escape or
Ctrl+Shift+A selects none. Every selected row has the same highlight. The selection stays on the
same torrents when the list is re-sorted or updated, and an update never scrolls the list. A click
or a right-click acts on the torrent the button went down on, even if an update moves another one
under the pointer before the button comes up, and on nothing if that torrent has left the list by
then. A selected torrent that the filter or the search hides, or that is gone from the daemon,
leaves the selection, and it is not selected again when it shows again. With no row to start from
(nothing clicked yet, or the selection cleared with Escape), Up, Down, Page Up, Page Down and Home
select the first torrent, and End the last. Tab moves on from the table to the details panel (the
copy buttons of the General tab, or the switch and the file list of the Files tab), the turtle and
the speed limits in the status bar, then to the search box and the toolbar's buttons; a disabled
button is passed over. The table has the keyboard focus when the window opens.

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
  daemon, so they are on the clipboard a moment later (see the clipboard notes below). A newer copy
  takes the place of one still waiting for the daemon, so the clipboard gets what was asked for
  last. If the daemon no longer has some of the torrents, the other links are copied and a
  notification says how many are missing;
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
item, and Escape closes the menu. Choosing an item gives the keys back to the table. The toolbar's
Remove and Columns menus keep the keys in the same way: while one is open, Tab, Shift+Tab, `Ctrl+F`
and `/` do not leave it.

Two notes on the clipboard:

- the magnet links reach the clipboard only if 4x4 still has the focus when the daemon's answer
  comes, a moment after `Ctrl+C`: 4x4 does not set the clipboard while another app has the focus
  (on Wayland it could not). If you have switched to another app by then, nothing is copied and a
  notification asks you to copy them again. The hashes are copied at once, so this never happens to
  them;
- 4x4 itself holds the copied text and hands it to the app that pastes it: when 4x4 quits, the
  copied text is lost unless a clipboard manager keeps it.

Removing always asks first. The dialog names the first five torrents in full, a long name wrapped
over several lines, says how many more there are, and says what becomes of the files: that they stay
on the server, or how much downloaded data will be deleted for good. It removes the torrents it
named when it opened, even if the list changes while it is open. "Remove from list" has Remove
focused, so Enter removes. "Remove and delete files" has Cancel focused: Enter or Space press the
button that has the focus, so they cancel, and only a click on "Delete files", or Tab to it and
Enter, deletes anything. Escape, Cancel or a click beside the dialog close it and remove nothing.
While the dialog is open, Delete and Shift+Delete open no second one, and holding the key opens it
once; a key already held when the dialog opens presses no button in it, so an Enter held on a menu
item opens the dialog without confirming it. The mouse cannot confirm by accident either: the
dialogs ignore clicks during their first moment (half a second, and 1 s for "Delete files"), so that
a double-click on a menu item cannot confirm the dialog it opens, however slowly it is clicked. A
click counts from when the button was pressed, not from when it was let go, and the second click of
a double-click never counts. While the daemon is not connected, Remove and its keys do nothing, and
the confirm button of an open dialog is disabled until the connection is back; Cancel still works.
Closing the dialog gives the keys back to the table.

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

The details panel under the table shows the selected torrent. With none selected it says "Select a
torrent", and with several, how many are selected; it shows a torrent only when exactly one of the
torrents the list shows is selected. `Alt+Enter` hides the panel and shows it again, from anywhere
in the window. It has four tabs, General, Files, Peers and Trackers, which a click or `Alt+1`,
`Alt+2`, `Alt+3` and `Alt+4` open. While a menu or the speed limits popover is open, these five
keys do nothing. The
first time, the panel is open on the General tab, high enough for its usual rows. Drag the line
between the table and the panel to change its height. The panel keeps that height when it is hidden
and between runs; when the window is too low for it and the table's header and two rows, it is drawn
lower, and it grows back with the window. When the General tab does not fit, it scrolls.

The General tab shows, from the daemon's list:

- the path of the data on the server (the download directory and the torrent's name), the size
  (with how much of it is wanted, when only some files are), the info hash, the share ratio, and
  when the torrent was added and completed ("not yet" while it downloads);
- an error or a tracker problem the daemon reports for the torrent, first, in red or yellow.

The rest comes from the torrent's details, which 4x4 reads only while the panel is open on one
torrent and keeps only while it shows them, and says "—" until they have come: how many pieces there
are and their size, the data downloaded and uploaded over the torrent's life, who created the
torrent and when, its source (a private tracker's tag, shown only when there is one), its comment,
whether it is private, and how its trackers are doing: how many work, how many fail and how many
the daemon has not announced to since it started (all of them for a torrent that has not run since
then, and backup trackers), with the worst problem among them in plain words, such as "Tracker
tracker.example.org: connection timed out". Dates are in
the local time zone, such as "2026-10-29 14:05". The buttons next to the path and the hash copy them
at once, also while the daemon is not connected, and give the keys back to the table.

The Files tab shows the torrent's files as a tree: the torrent's own folder first, open, and in each
folder the folders before the files, by name, with numbers in their natural order ("E2" before
"E10"). Names that agree in their first 32 runs of digits or of other characters, in their first 256
bytes, or in their first 64 characters that UTF-8 writes in three bytes or more (those of Chinese
and Japanese among them), which real names do not, are ordered by the rest character by character.
Padding files, which some torrents carry only to line their pieces up, are hidden. Each row
has the arrow that opens and closes a folder, a checkbox for whether it is downloaded (a dash for a
folder where only some of its files are), the name, the size, the progress, and the priority, Low,
Normal or High ("Mixed" for a folder whose files differ), which a click on it changes. A change to a
folder applies to every file in it, the hidden padding files included. Each row is one line. After
the name of a file that is downloading, once its first bytes have come, muted text such as "Playable
from start: 12.4 GiB (34%)" says how much of its beginning has come without a gap, which a player
can already read; a long name is cut short before it is. It is counted in the pieces the daemon has,
from the file's first byte up to the first piece still missing, so a file that downloads in no
particular order can be half done and still play only its first seconds. Above the list, the switch
"Download in order (sequential)" has the daemon download the torrent's pieces from the start, so
that a file can be played while it downloads. A change shows once the daemon's next answer does, a
moment later, not before. While the daemon is not connected, the checkboxes, the priorities and the
switch are disabled. Until the details have come, the tab says "—" under the switch, and so it does
while the files of a torrent of more than 1000 are sorted into their tree when the tab first shows
them, which is done off the window's own thread, so that the window keeps answering meanwhile (some
20 ms for 50 000 files). A magnet link has no files until its metadata has come; the tab says so,
and the switch can be set already. A file list that would take too much memory to show, such as one
of more than 50 000 files or of files nested hundreds of folders deep, is not drawn: the tab says
the list is too large to show.

A click anywhere in the file list gives it the keys, and so does Tab: Up, Down, Page Up, Page Down,
Home and End move between the rows, Left closes a folder or goes to the folder around the row, Right
opens a folder or goes into it, Space downloads or skips the row, and `+` and `-` raise or lower its
priority by a step (a folder of mixed priority goes to Normal). Holding Space, `+` or `-` acts once.
With no row to start from, Up, Down, Page Up, Page Down and Home go to the first row, and End to the
last, as in the table. These keys never change the selected torrents, and the table's keys never
move in the file list. While a priority menu is open, it keeps the keys. A change that fails shows a
notification, one for downloading or skipping files, one for their priorities and one for the
switch. The open folders and the place in the list stay while the Files tab shows the same torrent,
also after a visit to the General tab; when it shows another torrent, the list starts afresh, with
only the torrent's own folder open. They are not remembered between runs.

The Peers tab shows who the torrent is connected to, one line each. The line above the list counts
the peers, then how many of them were found in each way, the most first (from trackers, DHT, PEX,
LPD, incoming, LTEP, or the resume file of an earlier run; a way that found none is left out), then
how many send data to us and how many receive data from us right now, as their speeds tell; on a
narrow panel the line wraps between its parts, never inside one. Each row has the peer's country
flag and two-letter code (a grey globe when its address has no known country, such as a private
address, or when the GeoIP files are missing), its address (screen readers hear its port too), the
client it says it runs ("—" when it says none), how much of the torrent it has, as a bar in the
torrent's state color and in percent, the speeds from it and to it, in the state's color and bold
while data moves that way, and the icons of its connection: a lock when it is encrypted, a "µTP"
badge when it runs over µTP rather than TCP, and an arrow when the peer connected to us, each named
in its tooltip. The busiest peers come first: by the speed they send to us, then the speed we send
to them (both counted in whole KiB/s, so that rows do not trade places for a small wobble), then the
most complete, so that idle rows stay put. A click on the header of the country, the address, the
client, the progress or a speed sorts by it (the progress and the speeds largest first, a speed by
its exact value, not in whole KiB/s), and a second click reverses the order. Until the first read
has come, the tab says "—"; for a paused torrent it says "The torrent is paused", and while no peer
is connected, as for a magnet link still looking for its metadata, "No peers connected". While the
daemon does not answer, the rows are dimmed, as the table is. The list takes no keys; the wheel
scrolls it. 4x4 reads the peers every 3 seconds, only while the tab is open on one torrent and the
window is shown. Left, or with the panel hidden, the tab keeps nothing, and its order goes back to
the busiest first; the order is not remembered between runs.

The Trackers tab shows each tracker of the torrent on a row of two lines, in the order the daemon
keeps them, tier by tier; it is not sortable, because the order of the tiers means something. The
dot and the first line give the tracker's address (without the port when that is 80 or 443) and,
muted beside it, its site name when the address does not say it. The second line says what the
tracker is doing, in the dot's color: "Working" (green), "Announcing" (the theme's info color, cyan
in the default themes), "Not contacted yet" and "Backup" (grey), "Timed out" (yellow), or, when an
announce failed, the text the tracker or the daemon sent, in red, as sent but with control
characters taken out and cut to the line (the whole text is in its tooltip; when the daemon sent
none, "Announce failed"). After it come how many peers the last announce brought and how long ago it
was; a failed scrape, which many trackers never answer, is a muted note "No peer counts" after them
when the tracker reports no seeders or leechers, and the dot stays green. The columns after the
tracker are the tier (from 1), the last announce ("3 m ago") and the next one ("in 27 m", "now",
"queued" while it waits for a free slot, "—" when none is scheduled, as for a paused torrent), and
the seeders, leechers and downloads the tracker reports ("—" when it reports none). A time is
rounded down to whole minutes above a minute, and while the window is shown and active the tab draws
itself again every 5 seconds, so that the times move on an idle torrent too. A torrent without
trackers says "This torrent has no trackers (DHT and PEX only)", or, when it is private, that it
cannot use DHT or PEX either; until the first read has come the tab says "—". The daemon keeps the
announce data of a tier, not of a tracker, and shows it on the tier's current tracker: after an
error it makes that tracker a backup and shows the error on the next one of the tier, so a row tells
how its tier is doing. While the daemon does not answer, the rows are dimmed, as the table is. The
list takes no keys; the wheel scrolls it. 4x4 reads the trackers every 5 seconds, only while the tab
is open on one torrent and the window is shown, and again right after a reannounce of the torrent.
Left, or with the panel hidden, the tab keeps nothing.

4x4 reads a torrent's details only while the window is shown and active, the panel is open and it
shows one torrent, so that a small server is asked for no more than the panel needs. It reads them
in full once the selection has stayed on the torrent for a quarter of a second, so that moving
through the list with the arrow keys reads nothing on the way; again after connecting; and when the
torrent's files change, as when a magnet link's metadata comes. In between it reads only what
changes: each file's progress, priority and switch, the pieces, and the data moved over the
torrent's life. That is every 3 seconds while the torrent downloads or is verified, and otherwise
once a minute, with the whole list, when the trackers are read too; also once right after an action
on the torrent, such as a change made in the Files tab or a pause, and when the window shows again.
On a torrent of 2000 files that downloads, the panel adds about one request every 3 seconds and 11
KiB per second; on one that seeds, one request of 3 KiB a minute (see [Performance](#performance)).

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

- a dot for the connection: green when connected, grey while connecting or when no server is set,
  yellow while reconnecting and red when the daemon is too old (4x4 needs Transmission 4.1 or
  newer). Next to it, "Connecting…", "No server set" or "Reconnecting (attempt N)… timed out" and
  the like; the tooltip names the server once connected ("Connected to nas:9091") and says more
  about why the daemon could not be reached. A click on the dot or its text opens the Connection
  dialog;
- the turtle, which switches the daemon's alternative speed limits on and off (`Ctrl+T`). It is
  highlighted, in yellow, while they apply, and its tooltip gives them, such as "Alternative speed
  limits: ↓ 50 KiB/s ↑ 50 KiB/s";
- the global speed limits, such as "↓ 500 KiB/s · ↑ no limit". A click on them, or `Ctrl+L`, opens
  a popover that sets them (see below);
- the free space where new torrents are saved;
- the total download and upload speeds, ↓ and ↑.

The speed limits popover has, for the download and the upload limit each, a switch that turns the
limit on and a field for it in KiB/s (1024 bytes per second), as every speed in 4x4 is shown. The
daemon keeps its limits in kB/s (1000 bytes per second), and 4x4 converts them both ways, to the
nearest whole number: 500 KiB/s typed here is stored as 512 kB/s, and a limit of 100 kB/s set in
another client shows here as 98 KiB/s. Each field starts with the daemon's value, also while that
limit is off. Typing a speed into a field switches its limit on, and clearing the field switches it
off. `Enter` in a field, or Apply, sends the limits that changed, and only those, and closes the
popover; `Escape`, a click beside it, or moving the focus out of it (`Tab` past Apply, `Shift+Tab`,
`Ctrl+F`) closes it and changes nothing. So does `/` pressed on a switch or on Apply, where it types
nothing: it goes to the search box, as anywhere else. `Ctrl+T` switches the turtle and leaves the
popover open. Switching to another app leaves it open, with what was typed, until you come back. A
click on blank space in the popover leaves the cursor in the field. A limit is a whole number from 1
to 1,000,000 KiB/s: anything else is refused with a note under its field, and nothing is sent; a
limit switched on with an empty field asks for a speed. A key held down to open the popover does not
type into it or apply it. While a menu is open, the context menu or one of the toolbar, `Ctrl+T` and
`Ctrl+L` do nothing. The turtle and the limits are disabled until the daemon has sent its settings,
and while it is not connected; an open popover then keeps what was typed, and Apply waits for the
connection. A change that fails shows a notification, as an action does, one for the turtle and one
for each limit.

Torrents to add come from the toolbar, the keys, the clipboard and drops, and each of them only
offers the torrents in the add dialog:

- Add file (the file with a plus), or `Ctrl+O` from anywhere in the window, opens the desktop's
  file dialog over the window, which shows only torrent files ("Torrent files") and lets you choose
  several at once; each file chosen is offered in turn. Cancelling it adds nothing. Files it names
  that are not on this computer, such as those of a network place, are left out, with one
  notification that counts them. If the desktop has no file dialog, a notification says so;
- Add magnet (the magnet), or `Ctrl+U`, offers each magnet link on the clipboard, one per line, as
  4x4's own "Copy magnet link" writes them; with none there, it opens the form with an empty link
  field, to paste one into;
- `Ctrl+V` in the table offers the magnet links on the clipboard too, and with none there a
  notification says "No magnet link on the clipboard". In the search box `Ctrl+V` pastes text;
- `.torrent` files dropped anywhere on the window, or on the open add dialog, join the torrents to
  add. The window, or the dialog, is tinted and framed while they hover, and nothing in it moves.
  Other files are left out, with one notification that counts them. Torrents dropped from another
  app bring the window to the front, so that the keys go to the add dialog.

4x4 reads the clipboard only when you ask it to, and no more magnet links of it than can wait to be
added (see below): the lines that look like magnet links after the 100th link taken, or after the
200th such line read (copies of one link and broken links count), are left out and counted. While a
menu, the speed limits or a dialog is open, `Ctrl+O`, `Ctrl+U` and `Ctrl+V` do nothing, and holding
one of them down acts once. One file dialog is open at a time, for as long as it stays open: Add
file or `Ctrl+O` meanwhile says "The file dialog is open already".

The add dialog shows the torrents to add, one at a time; nothing is added until you press Add. A
`.torrent` file is read only when its turn comes, and one that cannot be offered is skipped with a
notification that names it and says why, and is not counted among the torrents to add: it does not
exist, 4x4 may not read it, it is not a regular file or is larger than 16 MiB, it is not a torrent
file or holds no torrent, it is a BitTorrent v2 torrent only, which Transmission 4.1 cannot add, or
the daemon would refuse it as broken (as when it has no files, two files of the same path, or piece
hashes that do not match its size). A file that takes longer than a moment to read, as on a network
folder that hangs, opens the dialog with "Reading the file…" and Skip, which moves on to the next
torrent. At most 100 torrents wait; a notification says how many more were left out. The title says
which torrent of how many it shows, such as "Add torrent (2 of 5)"; under it come the torrent's name
and size, and for a magnet link its field, checked as you type, with the link's name or its hash. A
magnet link must start with `magnet:?` and name one BitTorrent info hash (40 hexadecimal or 32
base32 characters), without a control character such as a line break, and in at most 64 KiB; a
bare hash, a link with several hashes and one with only a BitTorrent v2 hash, which Transmission
4.1 cannot add, are refused with the reason under the form.

The folder on the server starts as the daemon's download folder. The button beside it, or
`Alt+Down`, offers that folder, the last 10 folders torrents were added to (remembered between runs;
a folder counts once a torrent was added there, not when the daemon had the torrent already) and the
folders of the torrents in the list. Half a second after you stop typing, 4x4 asks the daemon how
much space is free in the folder, once per folder while the dialog is open: the line under the field
gives the free space, says "New folder" with the free space of the nearest folder above it that
exists, or says that the free space is unknown. A new folder is fine: the daemon makes it once the
torrent starts and writes there, and a torrent added paused makes nothing until then.

A `.torrent` lists its files as a tree with checkboxes, every file wanted at first, which the keys
of the Files tab move through (without the priorities); padding files are hidden and go with their
folder. Some `.torrent` files offer no choice, and all their files are added, with the dialog saying
why. A `.torrent` whose list of files could be read in more than one way offers none: its
dictionary keys are out of order or given twice, it has two `info` dictionaries, or an entry of its
file list is not a plain one. The daemon could then number the files otherwise than 4x4 reads them,
so leaving one out could leave out another. So does one with too many files to choose from: more
than 50 000, or files nested so deep that their tree would take too much memory. A magnet link has
no choice of files either, as its files are known only once its metadata has come. "Start
immediately" starts as the daemon's setting for new torrents; without it, the torrent is added
paused.

Add stays disabled, with the reason beside the buttons, until a file is wanted, the folder is a
full path on the server without `.` or `..`, the magnet link is valid and the daemon is connected;
for a torrent in the list already, Show selects it instead. The dialog keeps its size while you
choose files, type a folder or paste a link: the reason, the free space and the name take room
that is already there. Add sends the torrent once and says "Adding…"
until the daemon answers. The dialog then shows the next torrent or closes, and the new torrent is
selected as soon as it is in the list, with the filter and the search cleared if they would hide it.
A torrent the daemon has already is selected, with a notification; one whose add the daemon does not
confirm in time is selected if it shows up within two minutes; a failure shows a notification and
keeps the form, to try again; if the part of 4x4 that talks to the daemon stops before the daemon
answers, a notification says so, as the torrent may have been added. A new torrent is not selected
once you click or move in the list (even on the row selected already), or change the filter, the
search or the sort order, from the moment you press Add. The next torrent keeps the folder and
"Start immediately" you chose. `Enter` in a field adds, `Cancel` or `Escape` skips the torrent, a
click beside the dialog does nothing, and `.torrent` files dropped anywhere on the dialog join the
queue. While the dialog that removes torrents is open, the torrents to add wait for it to close.

4x4 also offers the `.torrent` files and magnet links it is started with, such as
`4x4 Show.torrent 'magnet:?xt=urn:btih:…'`, or that a file manager or a browser opens with it, in
the add dialog, in their order. A relative path is taken from the folder 4x4 was started in, and a
`file://` address must name a file on this computer, without a query. Web addresses (`http://`,
`https://` and the like), bare info hashes, options (anything starting with `-`, also after a `--`)
and anything else are refused, with one notification that counts them and says why the first one
was; a first `--`, which a shell or `cargo run` may pass, is skipped. At most 100 are taken, as many
as wait to be added; the notification that some were left out counts the others, which are not
looked at. From the source tree, `just run 'magnet:?xt=…'` passes them on; quote a magnet link, as
its `&` means something to the shell.

One 4x4 runs at a time for each user. Starting it again, with or without files and links, hands
them to the 4x4 that runs already and exits at once: that one offers them in its add dialog, after
any it is showing, and brings its window to the front. Where the desktop does not let it come to the
front, the window asks for your attention instead (a highlighted task bar entry). If the 4x4 that
runs does not answer within 2 seconds, because it hangs, the new start opens a window of its own
with them, so a click on a file is never lost; the new start takes the place of a 4x4 that is
quitting, also one that quits while the new start waits for it. Debug builds (`just run`) and
release builds never hand over to each other, and a 4x4 in [`just virtual`](#windows-off-screen)
never hands over to the one on your desktop. Without `$XDG_RUNTIME_DIR`, every start opens a window
of its own and the log says so.

The desktop entry in `packaging/linux/`,
[`io.github.lexy898.FourByFour.desktop`](packaging/linux/io.github.lexy898.FourByFour.desktop),
offers 4x4 as a handler for `.torrent` files and `magnet:` links (`Exec=4x4 %U`, one process for all
the files and links of a launch). The package installs it (see [Install](#install)), so file
managers and browsers list 4x4 among the apps for them; `just default-handler` makes it their
default and `just default-handler --undo` gives them back. 4x4 never makes itself the default
handler without asking. From the source tree, without the package, start it with them or drop them
on its window.

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
| `Up`, `Down`, `Enter`                               | in a menu                 | move to an item and choose it                |
| `Escape`                                            | in a menu                 | closes it                                    |
| `Enter`, `Space`                                    | in a removal dialog       | press the button that has the focus          |
| `Escape`                                            | in a removal dialog       | closes it and removes nothing                |
| `Ctrl+T`                                            | anywhere in the window    | switches the alternative speed limits        |
| `Ctrl+L`                                            | anywhere in the window    | opens the speed limits                       |
| `Alt+Enter`                                         | anywhere in the window    | shows or hides the details panel             |
| `Alt+1`, `Alt+2`, `Alt+3`, `Alt+4`                  | anywhere in the window    | show the panel's tabs in the bar's order     |
| `Ctrl+O`                                            | anywhere in the window    | opens the file dialog to add .torrent files  |
| `Ctrl+U`                                            | anywhere in the window    | adds the clipboard's magnet links, or a form |
| `Ctrl+,`                                            | anywhere in the window    | opens the Connection dialog                  |
| `Ctrl+V`                                            | in the table              | adds the magnet links on the clipboard       |
| `Up`, `Down`, `Page Up`, `Page Down`, `Home`, `End` | in the file list          | move between files and folders               |
| `Left`, `Right`                                     | in the file list          | close or open a folder, or go up or in       |
| `Space`                                             | in the file list          | downloads or skips the file or folder        |
| `+`, `-`                                            | in the file list          | raise or lower its priority                  |
| `Enter`                                             | in the speed limits       | applies them                                 |
| `Escape`                                            | in the speed limits       | closes them and changes nothing              |
| `Enter`                                             | in the add dialog         | adds it from a field, or presses the button  |
| `Escape`                                            | in the add dialog         | skips the torrent shown; adds nothing        |
| `Enter`                                             | in the Connection dialog  | saves from a field, or presses the button    |
| `Escape`                                            | in the Connection dialog  | closes it and changes nothing                |
| `Alt+Down`                                          | in the folder field       | opens the folders used lately                |
| `Tab`, `Shift+Tab`                                  | anywhere                  | move to the next or the previous control     |
| `Ctrl+Q`                                            | anywhere                  | quits 4x4                                    |

In the search box, `Ctrl+A` selects the text, not the torrents, `Ctrl+C` copies the selected text,
`Ctrl+V` pastes text, `Space` types a space, and `Delete` deletes a character, never a torrent.
`Ctrl+F` works on any keyboard layout. `/` is matched by the character the key types, not by its
place on the keyboard: on the Russian layout it is the key that types `/` there, not the key marked
`/`, which types a period. Verify is `Ctrl+Shift+R` rather than Transmission's `Ctrl+V`, which
adds magnet links here.

## What 4x4 remembers

Between runs 4x4 remembers, in `state.toml`:

- the columns: their order, which are shown and their widths;
- the column the list is sorted by, and the direction;
- the filter chosen in the sidebar;
- the theme chosen with the toolbar button (system, light or dark);
- the size of the window and whether it is maximized. Not its position: on Wayland the desktop
  places windows, and an app can neither read nor set where its window is;
- the details panel: whether it is shown, its height and the tab open in it;
- the last 10 folders torrents were added to, which the add dialog offers.

The search text, the selected torrent, the scroll position, the open folders of the Files tab, the
order of the Peers tab and the torrents waiting in the add dialog are not remembered.

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
| `~/.config/4x4/config.toml`         | The server. The Connection dialog writes it, readable by you only.      |
| `~/.local/state/4x4/state.toml`     | What 4x4 remembers between runs. It is safe to delete.                  |
| `~/.local/state/4x4/4x4.<date>.log` | The log, one file per day (UTC date); the last 7 daily files are kept.  |

While it runs, the first 4x4 also keeps two files in `$XDG_RUNTIME_DIR` (`/run/user/<uid>`), named
after its app id (with `.Devel` in debug builds): `io.github.lexy898.FourByFour.lock`, which says
that it runs, and `io.github.lexy898.FourByFour.sock`, the socket later starts hand their files and
links to. Only you can use them. The socket is removed when 4x4 quits, and one left by a 4x4 that
crashed is replaced by the next one; the lock file stays and is safe to delete while 4x4 is not
running.

`config.toml` holds the server, which the Connection dialog sets (see [Connection](#connection));
it can be written by hand too. Only plain `http://` is supported, and `username` and `password` are
only for a daemon that asks for a login (either one alone counts the other as empty):

```toml
[connection]
url = "http://192.168.1.10:9091/transmission/rpc"
username = "alice"
password = "secret"
```

4x4 reads it at startup and connects to that address. There is no built-in server: without the
file, or without `url` in it, no server is set, and the Connection dialog opens. The address may not
hold the user name or the password (`http://alice:secret@…`), which belong in their own keys, nor a
query or a fragment; without a port it means port 80. A file 4x4 cannot use, such as one that is not
valid TOML, has a value of the wrong type or an `https://` address, is reported in the log and in
the status bar ("Config file not used"; the tooltip says why, and "Dismiss" hides it); no server is
set then either, and the dialog opens. Neither ever shows the password.

Save in the Connection dialog writes only the `[connection]` table's `url` (always with its port),
`username` and `password`, and removes `username` and `password` when there is no login; the
comments and every other key and table of the file stay as they were. It writes the file atomically,
as `state.toml`, with mode 0600, so that only you can read it, because the password is in it as
plain text. A file that is not TOML at all cannot be edited: Save moves it to `config.toml.bad`
(replacing an older one; only you can read it either, as it may hold the password), writes a new
one, and the log says so. If `config.toml` is a symbolic link (dotfiles kept in a repository), Save
writes through it: the file the link names gets the new text, and the link stays a link.

4x4 writes `state.toml` atomically, so a crash cannot leave half a file. If the file cannot be
parsed, 4x4 moves it to `state.toml.bad` (replacing an older one) and starts from the defaults. A
file written by a newer version of 4x4, one that cannot be read, or a corrupt one that cannot be
moved aside is left alone: 4x4 starts from the defaults and saves nothing in that session.

For the countries of peers, 4x4 reads, and never writes, the GeoIP country files of the
`geoip-database` package: `GeoIP.dat` (IPv4) and `GeoIPv6.dat` (IPv6). Each is taken from the
first of these folders that has it: `GeoIP` in `$XDG_DATA_HOME` (`~/.local/share`), `GeoIP` in each
folder of `$XDG_DATA_DIRS` (`/usr/local/share:/usr/share`), `/usr/share/GeoIP` and
`/var/lib/GeoIP`. 4x4 opens them the first time it reads the peers of a torrent, not at startup,
and reads a few bytes per address, never a whole file. Without them the peers show no country: the
log says once which files are missing and where it looked, and names any file it found but cannot
use, with the reason.

The log lines also go to stderr. `RUST_LOG` sets the level for both (for example
`RUST_LOG=x4_app=debug`); without it 4x4 logs its own lifecycle events and the problems of its
libraries. If the log directory cannot be used, 4x4 logs to stderr only and still starts.

Debug builds (`just run`) give the window the Wayland app id `io.github.lexy898.FourByFour.Devel`
instead of `io.github.lexy898.FourByFour`, so window rules made for test windows (for example,
open on another screen without taking focus) never catch the window you actually use, and a debug
build never hands its files and links to the 4x4 you use. Set `X4_DEVEL=1` to give a release build
the same id; `just measure` does.

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
`state.toml` and its log go there and are removed afterwards; it still reads `config.toml`, unless
`--offline`), waits 5 seconds and prints one line: the time to the first rendered frame, then the
memory as proportional (`pss_mib`), unique (`uss_mib`), anonymous (`rss_anon_mib`) and resident
(`rss_mib`) size, the thread count, whether Mesa's OpenGL stack is loaded (`gl_stack_mapped`) and
the binary size. It adds a `WARN:` line on stderr when a figure is above one of its own reference
limits (16 MiB private memory, 55 MiB PSS, 120 MiB RSS) or when the OpenGL stack is mapped, which it
should not be where a Vulkan driver is installed. The exit code stays 0 unless the measurement
itself failed. `--cpu <seconds>` adds a second line, the CPU time of all its threads over that many
seconds after the memory figures, as a percentage of one core (`cpu_percent`); `--synthetic`
measures the release build of the [synthetic demo](#synthetic-demo) instead of the app, and
arguments after `--` go to the program, as in `just measure --synthetic --cpu 30 -- --static`.
`--binary <path>` measures that file and builds nothing, such as the installed `/usr/bin/4x4` or one
taken out of a package (not with `--synthetic`, which builds the demo); a relative path is taken
from the repository's directory. `--offline` gives the program a temporary config directory of its
own, removed afterwards, whose `config.toml` names a closed port on this machine
(`http://127.0.0.1:1/transmission/rpc`), so the app reaches no daemon and keeps reconnecting; it is
refused with `--synthetic`, as the demo reads no config and reaches no network anyway.
Without it the app reads your `config.toml`; inside `just virtual`, whose config directory is
empty, no server is set, so the app polls nothing and shows the Connection dialog over the window,
which is not how it runs in use. For example,
`just virtual just measure --binary /usr/bin/4x4 --offline --cpu 60` measures the installed 4x4 off
screen and off the network. Measure the app with no development 4x4 running in the same session, or
inside `just virtual`: otherwise the measured start hands over to it and exits. (`just measure`
gives the program the development app id, so it never hands over to an installed 4x4.)

The honest memory metrics are the private (anonymous, `RssAnon`) memory and the PSS, not the RSS.
RSS counts every resident page the app maps, including shared driver libraries (`libLLVM`,
`libgallium`) that the desktop shell and the browser already keep in memory anyway, so it overstates
what 4x4 costs and swings with the page cache. PSS divides each shared page among the processes that
map it, which is the fair share, though it depends on what else is running. Private memory is what
4x4 alone holds and what only swap can take back. USS (private clean and dirty pages) adds the file
pages only 4x4 maps, such as its own code, which the kernel can drop without swap. The RSS stays in
the report as a reference.

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

### The details panel (stage 4)

Measured on 2026-11-02 on the same laptop, inside the headless KWin of `just virtual`, with the
release build of the [synthetic demo](#synthetic-demo), its 2000 torrents busy. The torrent of 2000
files is "Synthetic torrent 0005" (`--select 4`), which downloads; "Synthetic torrent 0014"
(`--select 13`) seeds. The requests and their bytes were counted in the demo's debug log
(`RUST_LOG=warn,x4_synthetic=debug,x4_core::service=debug`) over 60 seconds, from 15 seconds after
the start; the bytes are those of the bodies, without the HTTP headers. The stage 4 targets are at
most 1.75 requests per second with the panel open on a torrent that downloads, and 1.45 on one that
seeds.

| Details panel                       | Requests per second   | Details reads a minute | Details answers |
| ----------------------------------- | --------------------- | ---------------------- | --------------- |
| Open on the 2000 files, downloading | 1.733 (target ≤ 1.75) | 21                     | 11.1 KiB/s      |
| Open on a seeding torrent           | 1.400 (target ≤ 1.45) | 1                      | 0.05 KiB/s      |
| Closed                              | 1.383                 | 0                      | 0               |

The rest is the same in all three: every 1.5 seconds the recently active torrents and the session
statistics, the free space every 30 seconds, and the whole list and the session settings every
minute, 83 requests in the 60 seconds. With the demo's 2000 busy torrents their answers carry about
216 KiB/s, nearly all of it the 513 torrents that change on every poll; a real daemon with a few
torrents sends a small part of that. The full read of the 2000 files, once when the torrent is
selected, is 298 KiB; each of their progress reads is about 32 KiB, and the one with the trackers
about the same; a seeding torrent's read with its trackers is 3 KiB. The 4x4 requests themselves are
under 0.5 KiB/s.

Memory and CPU, medians of three runs, each the first start of its own session
(`X4_MEASURE_SETTLE_SECONDS=15 just virtual just measure --synthetic --cpu 30`, with `-- --select 4`
for the General tab and `-- --select 4 --press alt-2` for the Files tab); "not met" marks every cell
over its target:

| Metric                     | Target              | Nothing selected    | 2000 files, General | 2000 files, Files   |
| -------------------------- | ------------------- | ------------------- | ------------------- | ------------------- |
| First frame                | ≤ 500 ms            | 234 ms              | 229 ms              | 226 ms              |
| Resident memory (RSS)      | ≤ 100 MiB           | 165.0 MiB (not met) | 166.3 MiB (not met) | 169.4 MiB (not met) |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 78.1 MiB (not met)  | 79.2 MiB (not met)  | 82.4 MiB (not met)  |
| Unique memory (USS)        | —                   | 66.7 MiB            | 67.8 MiB            | 71.0 MiB            |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 37.3 MiB (not met)  | 38.4 MiB (not met)  | 41.6 MiB (not met)  |
| Threads                    | —                   | 33                  | 33                  | 33                  |
| CPU over 30 s, one core    | —                   | 1.30 %              | 1.53 %              | 1.83 %              |
| Binary size                | —                   | 37.4 MiB            | 37.4 MiB            | 37.4 MiB            |

- The details of the 2000 files add 1.1 MiB of private memory with the General tab open, and 4.3 MiB
  with the Files tab, which keeps the tree of the files and its rows; leaving the tab lets the tree
  go. Reading their progress every 3 seconds, merging it and, on the Files tab, making the tree
  again, costs 0.2 and 0.5 points of one core.
- The PSS and the private memory are close to those of the busy demo in stage 2 (81.6 and 38.6 MiB),
  while the RSS is 65 MiB higher: it counts the shared driver pages that were resident in these
  sessions (see above), so compare the columns of this table with each other.

### Adding (stage 5)

Measured on 2026-11-03 on the same laptop, inside the headless KWin of `just virtual`, with release
builds; each figure is the median of three runs, each the first start of its own session. The app
had a `config.toml` in the session's own config directory that names a closed port on this machine
(`http://127.0.0.1:1/transmission/rpc`), so it reached no daemon and kept reconnecting; the
synthetic demo answered from memory (`--static`). Nothing was added: the add dialog was opened and,
where the table says so, skipped with Escape (the demo's `--press escape`). The `.torrent` was made
for the measurement: 2000 files of 1 MiB in 40 folders, 168 KB.

Startup (`just measure`), with the single-instance claim and without it, as when there is no
`$XDG_RUNTIME_DIR` (KWin's socket was then named by its full path); "not met" marks every cell over
its target:

| Metric                     | Target              | With the claim      | Without the claim   |
| -------------------------- | ------------------- | ------------------- | ------------------- |
| First frame                | ≤ 500 ms            | 219 ms              | 217 ms              |
| Resident memory (RSS)      | ≤ 100 MiB           | 145.1 MiB (not met) | 145.0 MiB (not met) |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 60.9 MiB (not met)  | 60.9 MiB (not met)  |
| Unique memory (USS)        | —                   | 49.3 MiB            | 49.3 MiB            |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 16.0 MiB (not met)  | 16.0 MiB (not met)  |
| Threads                    | —                   | 35                  | 34                  |
| Binary size                | —                   | 39.5 MiB            | 39.5 MiB            |

- The claim, which takes the lock and binds the socket before the window opens, took 80 to 92 µs in
  the app's own log (from "claiming the single instance" to "this is the first 4x4", three runs),
  and 113 to 118 µs when it first removed the socket of a 4x4 that had been killed: well under the
  10 ms it may take. The first frames with and without it differ by less than the runs among
  themselves. The thread that answers later launches is the one more thread. The private memory is
  just over 16 MiB (by less than 0.1 MiB) either way.
- A second launch with a magnet link, while the first 4x4 ran, took 6.3 ms from its start to its
  exit (median of 10 in one session, 5.1 to 7.2 ms, and 27.3 ms for the first of them, 5 seconds
  after the first 4x4 started). Of that, the hand-over itself, from connecting to the answer, took
  0.24 ms (22 ms for the first), and the first 4x4 had queued the link 0.26 ms after the second
  began to claim. The rest is starting the process and its log.

The add dialog on the 2000 files, measured again once each file was read on a thread of its own;
the demo was measured 15 seconds after its start, 28 seconds for the five files, and the app after
5 seconds:

| Metric                     | Demo, no dialog | Demo, dialog open | Demo, closed | Demo, 5 files opened and skipped | App, no dialog | App, dialog open |
| -------------------------- | --------------- | ----------------- | ------------ | -------------------------------- | -------------- | ---------------- |
| First frame                | 299 ms          | 287 ms            | 260 ms       | 237 ms                           | 223 ms         | 219 ms           |
| Resident memory (RSS)      | 166.0 MiB       | 168.2 MiB         | 168.3 MiB    | 171.9 MiB                        | 146.3 MiB      | 149.9 MiB        |
| Proportional memory (PSS)  | 81.9 MiB        | 83.9 MiB          | 84.0 MiB     | 87.6 MiB                         | 62.0 MiB       | 65.6 MiB         |
| Unique memory (USS)        | 70.4 MiB        | 72.4 MiB          | 72.6 MiB     | 76.1 MiB                         | 50.6 MiB       | 54.2 MiB         |
| Private memory (`RssAnon`) | 36.4 MiB        | 40.1 MiB          | 40.1 MiB     | 41.9 MiB                         | 16.0 MiB       | 19.3 MiB         |
| Threads                    | 33              | 33                | 33           | 33                               | 35             | 35               |

- The dialog on 2000 files adds 3.3 to 3.7 MiB of private memory: the parse, the tree of the files
  and the rows it draws; the file itself is 168 KB.
- Closing the dialog gives none of it back to the system, since glibc keeps freed memory for its
  next use, but nothing builds up per file: five such files opened and skipped in turn hold 1.8 MiB
  more than one. glibc keeps an arena of freed memory for each thread that allocates. Each file is
  read and parsed on a thread of its own, which ends with the read, and the next reader takes over
  the arena the last one left, so the reads reuse the same memory; now and then another thread
  takes that arena first, and the next reader makes one more. Read on GPUI's background pool, whose
  threads never end, each file had kept an arena of its own, and five held 5.5 MiB more than one.
  With a single arena for the whole app (`MALLOC_ARENA_MAX=1`, two runs each, measured then) the
  demo held 29.0 MiB with no dialog, 31.9 MiB after one file and 31.5 MiB after five.
- After a large add, which none of these runs made, glibc may keep some freed memory resident as
  well: the service thread builds the request, the `.torrent` in base64 inside JSON, and its arena
  keeps the freed buffers for the next request (a separate test of the same pattern kept 6.3 MiB
  after a 16 MiB file); the memory is reused, not lost. Stage 6 below returns it.

The Files tab keeps its tree from one read of the details to the next while the torrent's files
stay the same, which they do while it downloads: each read every 3 seconds only takes in the
progress, the priorities and which files are wanted, and the names are sorted once, when the tab
opens on the torrent. Measured on the same laptop in release builds of x4-core, the best of three,
making the tree and its rows against taking a read into the tree kept:

| Files of the torrent                            | Tree made at each read | Tree kept |
| ----------------------------------------------- | ---------------------- | --------- |
| 5000, four folders deep, long names             | 1.95 ms                | 0.05 ms   |
| 50 000, four folders deep, long names           | 19.6 ms                | 1.0 ms    |
| 50 000 in one folder, open                      | 14.0 ms                | 3.1 ms    |
| 36 000 names crafted to compare slowly, open    | 667 ms                 | 14 ms     |

Since stage 7 the first tree of more than 1000 files is made on a thread of its own, so the window
keeps drawing while the demo's 50 000 files take 18 ms (1000 names crafted to compare slowly take at
most 13.3 ms, on the window's thread); private memory with the Files tab open, medians of three
(`X4_MEASURE_SETTLE_SECONDS=15 just virtual just measure --synthetic -- --select 4 --press alt-2`
for the 2000 files, `--select 5` for the 50 000), is unchanged at 40.0 and 146.2 MiB, against 39.8
and 146.1 when the window's thread made the tree; the 146.2 carries the demo's own answer of 50 000
files: selecting that torrent adds about 106 MiB, mostly the made-up daemon's JSON, not 4x4's.

### The allocator (stage 6)

4x4 fixes glibc's mmap threshold at 128 KiB (`mallopt(M_MMAP_THRESHOLD)`, the first thing `main`
does, in `4x4` and in the synthetic demo alike), so that every freed buffer of that size or more
goes back to the system at once; the call is the only `unsafe` in the project, approved by the
owner, in `crates/x4-app/src/allocator.rs`, and does nothing off Linux with glibc. By default glibc
raises the threshold to the size of each large block it sees freed, up to 32 MiB, and the next
allocations of that size come from the arena of the thread that makes them, where `free` keeps
them; adding a big `.torrent` makes three such buffers: the file, read on a short-lived reader
thread, then its base64 form and the JSON body of the request on the long-lived service thread.
Through the real add path to a local sink, a 16 MiB and then an 8 MiB file left 37.6 to 39.6 MiB of
private memory resident (five runs) by default, and 0.30 MiB with the threshold fixed;
`malloc_trim(0)` alone was not reliable and adds nothing once it is fixed. `M_ARENA_MAX=1` would
save about 8 MiB more private memory, but it costs CPU in the demo (about 0.7 points while it polls
a busy daemon, about 17 while it scrolls), so it is not set. The test
`freed_large_buffers_go_back_to_the_system` pins the effect. In release builds inside
`just virtual`, medians of three runs of the synthetic demo (2000 torrents) without and with the
pin did not differ beyond the noise of the runs: first frame (median of nine starts) 236 and 237 ms,
private memory 36.3 and 36.2 MiB, CPU while idle 0.00 and 0.03 %, while 513 torrents change on
every poll 1.20 and 1.23 %, and while scrolling a row per frame 34.3 and 32.6 % (the runs without
the pin alone swung from 31 to 44 %), frame interval p99 17.62 and 17.38 ms with none over 33 ms;
the real app over a closed port (four sessions each) had a first frame of 237.5 and 226.5 ms and
16.0 and 15.8 MiB of private memory. The one visible cost is about 125 more minor page faults a
second (7 and 131) while the demo polls a busy daemon, because its answers of a few hundred KiB are
now fresh mappings each time.

### Installing (stage 6)

Measured on 2026-11-03 on the same laptop, inside the headless KWin of `just virtual`, with release
builds; each figure is the median of three runs, each the first start of its own session, with the
runs of the app and of the package taken in turn. The app (`just virtual just measure --offline
--cpu 60`) and the binary of the package that `just package` builds from the same commit (see
[Install](#install)), taken out of the package file with `bsdtar` instead of installed
(`just virtual just measure --binary <dir>/usr/bin/4x4 --offline --cpu 60`), reached no daemon and
kept reconnecting to the closed port. The synthetic demo answered from memory with nothing changing
(`X4_MEASURE_SETTLE_SECONDS=10 just virtual just measure --synthetic --cpu 60 -- --static`); as in
stage 2, the targets are those of the app with a real daemon, and the demo's column also carries
the synthetic daemon. The CPU is that of the 60 seconds after the memory figures; "not met" marks
every cell over its target:

| Metric                     | Target              | App (`target/release`) | Installed build (package) | Demo, 2000 torrents, idle |
| -------------------------- | ------------------- | ---------------------- | ------------------------- | ------------------------- |
| First frame                | ≤ 500 ms            | 220 ms                 | 229 ms                    | 239 ms                    |
| Resident memory (RSS)      | ≤ 100 MiB           | 144.8 MiB (not met)    | 144.4 MiB (not met)       | 164.8 MiB (not met)       |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 62.0 MiB (not met)     | 61.7 MiB (not met)        | 82.2 MiB (not met)        |
| Unique memory (USS)        | —                   | 51.4 MiB               | 51.2 MiB                  | 71.6 MiB                  |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 15.8 MiB               | 15.8 MiB                  | 36.2 MiB (not met)        |
| Threads                    | —                   | 35                     | 35                        | 33                        |
| CPU over 60 s, one core    | near 0 % when idle  | 0.03 %                 | 0.02 %                    | 0.08 %                    |
| Binary size                | —                   | 39.6 MiB               | 38.7 MiB                  | 39.1 MiB                  |

- The installed build and the one in `target/release` differ by less than the runs among
  themselves: the package's first frames were 227 to 317 ms (the 317 ms start is an outlier of
  unknown cause), those of `target/release` 218 to 229 ms. The package's binary is 0.9 MiB smaller,
  mostly in its unwind tables (`.eh_frame`) and its code; makepkg builds it with Arch's Rust flags
  and `--remap-path-prefix`, and which of them makes the difference was not isolated.
- Against stage 5 the app has 0.2 MiB less private memory (16.0 then), as the measurement of the
  allocator's pin found, and the same first frame (219 and 223 ms then); its PSS and USS are within
  about 1 MiB of stage 5's two measurements of it (60.9 to 62.0 and 49.3 to 50.6 MiB).
- With nothing to do, the app takes 0.03 % of one core even while it keeps trying the closed port.
  The demo's 60 seconds always hold one read of the whole list of 2000 torrents, which comes once a
  minute; its stage 2 and allocator figures (0.03 %, and 0.00 and 0.03 %) came from 30 second
  windows, which may miss it.
- Scrolling 2000 torrents one row per frame (three sessions of `--scroll` with
  `RUST_LOG=warn,x4_synthetic=info`, 600 intervals each) gave a median interval of 16.67 ms, a 99th
  percentile of 17.74 ms (17.61 to 17.85 ms) and a longest of 18.57 ms, none over 33 ms on the
  virtual output's 60 Hz; 43 % are just over 16.7 ms only because a 60 Hz pace jitters around
  16.67 ms. The demo took 31.9 % of one core over seconds 4 to 12 of the scroll
  (`X4_MEASURE_SETTLE_SECONDS=4 just virtual just measure --synthetic --cpu 8 -- --scroll`, three
  sessions, 31.7 to 32.0 %).

Against the goals of the design, in the virtual session: the cold start (at most 0.5 s), the idle
CPU (near 0) and the smooth scroll of 2000 torrents are met; of the memory targets, only the
proposed 16 MiB of private memory is met, by 0.2 MiB, while RSS (100 MiB) and the proposed PSS (55
MiB) are not, as in stages 4 and 5. The release profile is the one asked for: LTO (thin),
`codegen-units = 1` and `strip` (`[profile.release]` in `Cargo.toml`), and the package installs the
one binary, `/usr/bin/4x4`. The load on the daemon, at most one or two requests a second with 6
torrents and the window open, was measured in stage 2 against the owner's daemon (1.4 a second);
the traffic of the polls on a real daemon has not been measured.

What only the real desktop can show, with 4x4 installed and its real daemon (quit 4x4 before each
command that starts one, or the new start hands over to it): the start on the real screen, the
memory and CPU against the real daemon, the requests a second to it, and the scrolling at the real
screen's pace. Each command says whether it contacts your daemon:

```bash
# First frame from the application menu: start 4x4 there, then read its log. Local, no network
grep 'first frame rendered' ~/.local/state/4x4/4x4.$(date -u +%F).log | tail -n 1
# First frame, memory and idle CPU of the installed 4x4 on the real screen. Contacts your daemon:
# it polls it for about a minute, as 4x4 does
just measure --binary /usr/bin/4x4 --cpu 60
# Requests over 2 minutes with the window focused; divide the count by 120. Contacts your daemon
# for 2 minutes, with your real window and state
RUST_LOG=warn,x4_app=info,x4_core::service=debug timeout 120 4x4 2>&1 | grep -c 'request answered'
# Scrolling 2000 made-up torrents (the package has no demo, so it is built here, from the checkout;
# a window for ~13 s). No network
RUST_LOG=warn,x4_synthetic=info cargo run --release --locked -p x4-app --features synthetic \
    --bin x4-synthetic -- --scroll 2>&1 | grep 'scrolled the table'
# Private memory before and after adding a large .torrent (several MiB): it should come back to
# within a few MiB of the first figure. Run it with no debug build (just run) going: that build
# has the same process name, 4x4, so pgrep -x 4x4 (every process named exactly that) would print
# two pids and the command would fail. Adding contacts your daemon and adds that torrent to it for
# real: untick "Start immediately" in the add dialog, and afterwards remove it with "Remove from
# list…", which keeps files (there are none)
grep RssAnon /proc/$(pgrep -x 4x4)/status
```

### Peers and trackers (stage 7)

Measured on 2026-11-04 on the same laptop, inside the headless KWin of `just virtual`, with release
builds; each figure is the median of three runs, each the first start of its own session. The
[synthetic demo](#synthetic-demo) had its 2000 torrents busy and "Synthetic torrent 0016"
(`--select 15`) selected, which downloads from 150 peers and has twelve trackers, with its General
tab, its Peers tab (`--press alt-3`) or its Trackers tab (`--press alt-4`) open
(`X4_MEASURE_SETTLE_SECONDS=15 just virtual just measure --synthetic --cpu 30 -- --select 15
--press alt-3`); "not met" marks every cell over its target:

| Metric                     | Target              | General             | Peers, 150 peers    | Trackers, 12        |
| -------------------------- | ------------------- | ------------------- | ------------------- | ------------------- |
| First frame                | ≤ 500 ms            | 235 ms              | 230 ms              | 232 ms              |
| Resident memory (RSS)      | ≤ 100 MiB           | 145.8 MiB (not met) | 147.7 MiB (not met) | 146.6 MiB (not met) |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 83.9 MiB (not met)  | 85.8 MiB (not met)  | 84.8 MiB (not met)  |
| Unique memory (USS)        | —                   | 73.8 MiB            | 75.6 MiB            | 74.6 MiB            |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 37.6 MiB (not met)  | 39.4 MiB (not met)  | 38.4 MiB (not met)  |
| Threads                    | —                   | 33                  | 33                  | 33                  |
| CPU over 30 s, one core    | —                   | 1.43 %              | 2.00 %              | 1.63 %              |
| Binary size                | —                   | 39.7 MiB            | 39.7 MiB            | 39.7 MiB            |

- The Peers tab of 150 peers adds 1.8 MiB of private memory to the General tab of the same torrent,
  and 0.6 points of one core: every 3 seconds the made-up daemon writes the 150 peers (about 65 KiB
  of JSON), the service parses them and looks up their countries, and the tab orders and draws the
  rows it shows. The synthetic daemon's own part counts here too. The Trackers tab adds 0.8 MiB and
  0.2 points. The runs of each tab differed by at most 0.2 MiB of private memory.
- The requests were counted as in stage 4, in the demo's debug log
  (`RUST_LOG=warn,x4_synthetic=debug,x4_core::service=debug`) over 60 seconds from 15 seconds after
  the start, one run each. With the Peers tab open, 124 requests (2.07 a second), 20 of them reads
  of the peers: one every 3 seconds, as asked, each answer about 65 KiB for 150 peers (about 22
  KiB/s). With the Trackers tab, 116 (1.93 a second), 12 of them reads of the trackers, one every
  5 seconds, of 8.8 KiB each. With the General tab, 104 (1.73 a second): the recently active
  torrents and the session statistics every 1.5 seconds, the progress of the download every 3
  seconds, the free space every 30 seconds, and the whole list and the session settings every
  minute; the other two tabs add their reads to these.

The app itself, which reached no daemon and kept reconnecting to a closed port
(`just virtual just measure --offline --cpu 60`), against stage 6:

| Metric                     | Target              | App, stage 6        | App, stage 7                  |
| -------------------------- | ------------------- | ------------------- | ----------------------------- |
| First frame                | ≤ 500 ms            | 220 ms              | 225 ms                        |
| Resident memory (RSS)      | ≤ 100 MiB           | 144.8 MiB (not met) | 124.7 MiB (not met)           |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 62.0 MiB (not met)  | 62.6 MiB (not met)            |
| Unique memory (USS)        | —                   | 51.4 MiB            | 52.4 MiB                      |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 15.8 MiB            | 16.2 MiB (not met; see below) |
| Threads                    | —                   | 35                  | 35                            |
| CPU over 60 s, one core    | near 0 % when idle  | 0.03 %              | 0.02 %                        |
| Binary size                | —                   | 39.6 MiB            | 40.0 MiB                      |

- The first frame and the idle CPU are unchanged. The private memory of the table comes from two
  sessions: stage 6's 15.8 MiB from another one (the plan's baseline was 15.9), so the 0.4 MiB
  between them (stage 7 had 16.2 MiB in two runs, 16.4 in the third) is mostly the sessions', not
  stage 7's. Measured in one session of `just virtual`, the release builds of the two stages taken
  in turn, three runs each (`X4_MEASURE_SETTLE_SECONDS=10 just measure --offline --cpu 20 --binary
  <file>`), stage 6 had 15.2 MiB and stage 7 15.3 (medians), both under the proposed 16 MiB: stage 7
  adds about 0.1 MiB at startup. The app opens no country database until it shows the first peers,
  so none of it is the GeoIP files. The binary grew by 0.4 MiB, 0.17 MiB of which are the 265
  embedded flags. The RSS is 20 MiB lower probably because fewer shared driver pages were resident
  in these sessions, as the PSS and USS figures suggest (see above).

### The connection (stage 8)

Measured on 2026-11-05 on the same laptop, in one session of `just virtual`, with release builds:
the app of `master` (stage 7, built in a separate clone with `cargo build --release --locked`) and
the app of this stage, taken in turn, three runs each
(`X4_MEASURE_SETTLE_SECONDS=10 just measure --offline --cpu 20 --binary <file>`); each figure is
the median of the three. Both reached no daemon and kept reconnecting to the closed port that
`--offline` names, so a server was set and the Connection dialog stayed closed. The CPU is that of
the 20 seconds after the memory figures; "not met" marks every cell over its target:

| Metric                     | Target              | App, stage 7        | App, stage 8        |
| -------------------------- | ------------------- | ------------------- | ------------------- |
| First frame                | ≤ 500 ms            | 196 ms              | 190 ms              |
| Resident memory (RSS)      | ≤ 100 MiB           | 140.6 MiB (not met) | 140.9 MiB (not met) |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 57.5 MiB (not met)  | 57.8 MiB (not met)  |
| Unique memory (USS)        | —                   | 47.1 MiB            | 47.4 MiB            |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 15.3 MiB            | 15.4 MiB            |
| Threads                    | —                   | 29                  | 29                  |
| CPU over 20 s, one core    | near 0 % when idle  | 0.05 %              | 0.00 %              |
| Binary size                | —                   | 40.1 MiB            | 40.4 MiB            |

- Stage 8 adds 0.1 MiB of private memory at startup and 0.3 MiB of PSS and USS, about what the
  runs differ by among themselves (stage 8's private memory was 15.4 to 15.5 MiB); the first frame
  and the idle CPU, 0.00 to 0.05 % in either build, are unchanged. The binary grew by 0.35 MiB:
  the Connection dialog, the credentials sent with each request, and the writer that edits
  `config.toml` (`toml_edit`, whose code the binary did not hold before: only the macros of
  gpui-component used it, while building). How much of it each takes was not measured.
- The first run of the session, which was `master`'s, had 35 threads, 16.0 MiB of private memory,
  63.7 MiB of PSS and a first frame of 275 ms; every later run, of either build, had 29 threads.
  The app's figures of stages 6 and 7 above, each from first starts of sessions, show 35 threads
  too, so the figures here compare with each other, not with those. That run sways none of the
  medians.

### The gpui-kit 0.9 bump (stage 9)

Measured on 2026-11-05 on the same laptop, in two sessions of `just virtual`, with release builds:
the app of this stage (gpui-kit 0.9.0, which brings `gpui-pre-wgpu` 0.3.11, with the Vulkan-first
patch on it) and the app of `master` at stage 8 (gpui-kit 0.8.1, `gpui-pre-wgpu` 0.3.10), built
again in a separate clone with `cargo build --release --locked`, so that both ran in the same
sessions. Each session ran the two apps in turn, three runs each, one session starting with each
(`X4_MEASURE_SETTLE_SECONDS=10 just measure --offline --cpu 20 --binary <file>`); each figure is the
median of the six runs of a build. Both reached no daemon and kept reconnecting to the closed port
that `--offline` names. "Stage 8 (above)" is the table of the previous subsection, taken in another
session; "not met" marks every cell over its target:

| Metric                     | Target              | Stage 8 (above)     | Stage 8 (rebuilt)   | Stage 9             |
| -------------------------- | ------------------- | ------------------- | ------------------- | ------------------- |
| First frame                | ≤ 500 ms            | 190 ms              | 193 ms              | 198 ms              |
| Resident memory (RSS)      | ≤ 100 MiB           | 140.9 MiB (not met) | 141.6 MiB (not met) | 140.9 MiB (not met) |
| Proportional memory (PSS)  | ≤ 55 MiB (proposed) | 57.8 MiB (not met)  | 55.0 MiB (not met)  | 54.4 MiB            |
| Unique memory (USS)        | —                   | 47.4 MiB            | 46.1 MiB            | 45.4 MiB            |
| Private memory (`RssAnon`) | ≤ 16 MiB (proposed) | 15.4 MiB            | 15.4 MiB            | 15.6 MiB            |
| Threads                    | —                   | 29                  | 29                  | 31                  |
| CPU over 20 s, one core    | near 0 % when idle  | 0.00 %              | 0.00 to 0.05 %      | 0.00 to 0.10 %      |
| Binary size                | —                   | 40.4 MiB            | 40.4 MiB            | 40.6 MiB            |

- Mesa's OpenGL stack was not mapped (`gl_stack_mapped=no`) in any of the 18 runs, the 12 of this
  stage's app (6 above, 4 of plain `just measure --offline`, three runs then one with `--cpu 30`,
  and 2 more first starts) and the 6 of the stage 8 app, and no `WARN:` line named it: the patch
  still keeps the OpenGL driver out of a process that Vulkan draws, so the numbers in
  [`PATCHED.md`](vendor/gpui-pre-wgpu/PATCHED.md) stand. The only `WARN:` lines were the usual ones
  on the memory figures: PSS in 2 of the 6 runs above (4 of the 6 of the stage 8 app, whose 55.0 MiB
  is a hair over the proposed 55) and private memory in the first run of a session.
- The bump changes the memory figures by about what the runs differ among themselves: PSS 0.6 MiB
  and USS 0.7 MiB lower, private memory 0.2 MiB higher, and the binary 0.2 MiB bigger. The first
  frame (193 and 198 ms) and the idle CPU (a tick of the 20 seconds is 0.05 %, so these are 0 to 2
  ticks) did not change.
- Stage 9 has two threads more. One is a third `zbus` D-Bus connection thread, always there; GPUI's
  listener for the system's power profile is the likely source. The other is a `blocking-2` pool
  thread, there in most runs: 31 threads in four of the six, 30 in one, 37 in the first run of a
  session.
- As in stage 8, the first run of a session is higher in either build: 35 threads, 16.3 MiB of
  private memory, 59.6 MiB of PSS and 267 ms for the stage 8 app; 37 threads, 16.2 to 16.5 MiB,
  58.9 to 60.1 MiB and 223 to 288 ms for the stage 9 app (the first runs of four sessions, apart
  from one first frame: the very first run of the day, right after the release build, took 928 ms
  and did not come back in the other three). The medians above are not moved by them.
- The OpenGL fallback works: started offline with `VK_DRIVER_FILES=/nonexistent/icd.json`, the
  release build logs `No usable Backends(VULKAN) adapter (hardware only: true)`, draws its window
  through OpenGL (`libgallium` and `libEGL_mesa` mapped; first frame 309 ms) and keeps reconnecting.
- Spot checks with the [synthetic demo](#synthetic-demo) (a debug build) in `just virtual`, each
  next to the stage 8 app's picture of the same scene, taken the same way. The main window with
  row 4 selected and General open (`--select 4`) is identical to the pixel, dark and light. The add
  dialog over a made-up magnet link with a 120-character name of dots, dashes and slashes breaks
  the line after a slash ("`…/Extras/`" over "`Behind-The-Scenes…`") where the stage 8 app broke it
  before one: GPUI now wraps by Unicode line breaking, and a narrow General tab does the same with
  the path ("`/data/torrents/`" over "`Synthetic torrent`"); nothing else wraps differently, and
  the hash still breaks at any character. In the torrent's context menu
  (`--select 4 --right-click --press down`) the first Down lands on Pause, the first item that can
  be clicked, as the torrent is downloading and Start is dimmed; the stage 8 app showed no
  highlight there. The Connection dialog (`--connection`) is the same as before. A window resized
  0.5 to 1.2 seconds after start (KWin scripting through D-Bus), to 900x600, 1300x820 and 760x520
  in turn, is redrawn whole at 1300x820 and at 760x520, the two sizes looked at, with no stale or
  black area.

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
`spectacle -b -n -f -o /tmp/shot.png` inside it captures the virtual screen. `HOME` is the
session's own as well, because some programs write by fixed paths under it rather than through the
XDG directories (KDE's update scripts, run when Plasma first starts in a session, do); cargo and
rustup keep your `~/.cargo` and `~/.rustup`. Whatever the session started is stopped when it ends.
For example, `just virtual just measure --offline` measures the app without a window on your
screens and without reaching your daemon. It needs KDE Plasma (`kwin_wayland` and
`dbus-run-session`) and works nowhere else.

### Synthetic demo

`just demo-synthetic` opens the main window over a synthetic daemon of 2000 torrents in every state,
made up in memory: nothing goes over the network, and none of 4x4's files is read or written (the
log goes to stderr only). The rates of the active torrents change on every poll. `--static` keeps
every rate at 0, an idle daemon; `--scroll` scrolls the table by one row per frame for 10 seconds,
logs the intervals between the frames and quits; `--select N` selects the row at index `N` (from 0)
once the list is there, and `--extend K` then extends the selection `K` rows down, as Shift+Down
would. The made-up daemon carries out the actions on its own torrents: pausing, starting (a torrent
that lacks data downloads, a complete one seeds), starting ahead of the queue, verifying (the
torrent checks for three updates, then goes back to what it was), removing, the speed settings, and
what the Files tab changes: which files download, their priorities and downloading in order. It
also adds torrents, from the content of a `.torrent` file (with the files it lists, in its order) or
from a magnet link (which stays without its metadata), honouring `paused`, the folder and the files
left out; it reports a torrent it already has as a duplicate and changes nothing; and its free space
answers for the session's folder and the folders of its torrents, and their parent folders, with
Transmission's error 5, "No such file or directory", for any other. Its torrents have files, most
one, some a folder of several, a few dozens, "Synthetic torrent 0005" (`--select 4`) 2000 in
folders, and "Synthetic torrent 0006" (`--select 5`, Down from it), a seed, 50 000 in folders, the
most the Files tab shows; their pieces follow the download, from the start when it is in order. The running
torrents have peers, at addresses reserved for documentation (192.0.2.0/24, 198.51.100.0/24,
203.0.113.0/24 and 2001:db8::/32), which stay from one read to the next while the torrent's rates
are shared out among them; a country database the demo makes in memory puts them in countries
(Japan, South Korea and Finland among them, whose flags are mostly white), in the region EU or in
none, and the installed GeoIP files are not read. Their trackers, none, one or twelve in tiers
with a backup in the first, announce every 30 minutes counted from the clock, and a few wait for a
slot or announce right now; some fail, time out or fail only their scrape, and those of a stopped
torrent that has not run since the made-up daemon started were never contacted. "Synthetic torrent
0016" (`--select 15`; with `--static`, "Synthetic torrent 0011", `--select 10`) is busy: it
downloads from 150 peers and has a tracker of every status, so that `just virtual just
demo-synthetic --select 15 --press alt-3` shows a long Peers tab and `--press alt-4` the Trackers
tab with each kind of row. `--press KEY`, which can be given several times, presses each key in turn
once the list is there and selected, one every three seconds, as on the keyboard: for example `just
virtual just demo-synthetic --static --select 5 --press ctrl-p --press space` pauses the sixth
torrent and starts it again, `--press shift-delete` opens the dialog that would delete its files,
and `--press ctrl-o` opens the desktop's file dialog, with its filter for torrent files. `--add
PATH_OR_MAGNET`, which can also be given several times, asks to add each `.torrent` file or magnet
link at the start, as if 4x4 had been opened with them, which opens the add dialog; what it does not
open makes its notification. The window is set to a made-up server, `synthetic.invalid:9091`, and
`--connection` opens the Connection dialog after the first frame, as `Ctrl+,` would. Its Test
answers for any server with the made-up daemon's version ("Connected: Transmission 4.1.3
(synthetic)"), and its Save switches to a new made-up daemon, whatever other server the fields name,
as a switch in 4x4 does; it writes no `config.toml`. `just virtual just demo-synthetic --scroll`
runs it off screen. With `RUST_LOG=x4_synthetic=debug` the made-up daemon logs each request it
answers with the bytes of the request and of the answer, which is how the requests and the bytes the
app asks of a daemon are counted (see [Performance](#performance)). The demo is the `x4-synthetic`
binary of the `synthetic` feature, so the `4x4` build does not contain it.

### Live tests

`just test-live` runs the tests in `crates/x4-rpc/tests/live.rs`, `crates/x4-core/tests/live.rs`
and `crates/x4-app/tests/live.rs` against a real Transmission 4.1+ daemon; `just test` and
`just check` skip them. They never touch existing torrents or the daemon's settings. Two tests only
read the server version: x4-rpc's through the client, and x4-app's as the Connection dialog's Test
does, with the one `session_get` that Test sends, whose answer must name a Transmission 4. Each of
the others makes a torrent of its own, a random 3 MiB file or a folder of a few random files of a
few KiB, adds it as a trackerless torrent into a test directory, works only on that torrent (stop,
start, start now, verify, reannounce, sequential download on and off, which files are downloaded and
their priorities, a file index past the last file, which the daemon refuses, adding it twice), and
removes it together with its data. Most write the data into the test directory first. Some do not:
they add their torrent paused, or by a magnet link of it, and the daemon then has none of its data.
One removes its torrent from the list only, checks that the daemon kept the file, and then deletes
the file itself. The tests of x4-core read the details of their torrent as the app does, in full and
then only their progress, with and without the trackers, and check that the app understands every
answer; and they add a torrent as the add dialog does, with a file unchecked that comes after a
padding file and before an empty file, and check that the daemon numbers the files, those two
included, in the order 4x4 read them from the `.torrent`, which is not the order of their names,
with only that file left out. The tests of adding check that a torrent added paused stays
stopped, though all its data is there; that adding it again, started, into another
folder and with another file left out, is a duplicate that changes nothing; that a folder that does
not exist yet, `4x4-live-new-<hex>` in the test directory, is kept as it was sent and made only
when the torrent starts; and that a magnet link of a torrent of the test's own is added and listed,
paused and without its metadata. One test of x4-core changes nothing at all: it reads the peers and
the trackers of the daemon's torrent with the most peers as the Peers and the Trackers tab do, and
checks what they become, printing only counts. Every other test checks that every other torrent
kept its status, sequential mode, file priorities and wanted files, and prints how many torrents
the daemon lists when it starts and once it has removed its own. The torrent and its data are removed even when a
test fails, and only from the folder it was added to: the test directory, or the folder of its own
that the daemon made directly in it. The folder the daemon reports must be that one once both are
normalized, so a doubled or a trailing `/` makes no difference, and a folder with any control
character in its name is never it.

An interrupted run (Ctrl-C, a killed process) cannot clean up, so each test prints the hash and the
data of its torrent as soon as the daemon has added it (`live test: added torrent <hash> with its
data at <path>`). To recover, remove the torrent with that hash in any Transmission client, choosing
to delete its local data. If no such line was printed, the run most likely stopped before the
torrent was added: delete the `4x4-live-<hex>.bin` file or the `4x4-live-<hex>` folder from the test
directory instead. A `4x4-live-new-<hex>` folder left there is a test's own, and can be deleted once
no torrent downloads into it. The next run refuses to start while any torrent downloads into the
test directory or a folder in it, so a leftover torrent cannot go unnoticed; remove it the same way.
The test that removes its torrent from the list only, keeping the file, may be interrupted after the
removal: then the daemon no longer has the torrent, and the file stays. Delete by hand the file
named on its `live test: added torrent …` line.

The test directory is one directory seen from both sides: the daemon reads it under one path and
this machine writes to it under another (for example over NFS), so both must be able to write
there. A test creates it if it is missing (its parent must exist) and then removes it again; a
directory that already existed is left in place. No torrent may already download into it.

`just test-live` also runs `crates/x4-core/tests/geoip_live.rs`, which needs no daemon: it opens the
installed GeoIP country files `GeoIP.dat` and `GeoIPv6.dat`, read-only, and checks only what does
not change from one build of them to the next: the countries of two public DNS resolvers, whose
addresses are each registered to one country, and that loopback addresses have none.

| Variable             | Meaning                                  | Default                                    |
| -------------------- | ---------------------------------------- | ------------------------------------------ |
| `X4_LIVE_URL`        | the daemon's RPC endpoint                | `http://192.168.1.5:9091/transmission/rpc` |
| `X4_LIVE_REMOTE_DIR` | the test directory as the daemon sees it | `/mnt/hdd/torrents/4x4-test`               |
| `X4_LIVE_LOCAL_DIR`  | the same directory on this machine       | `/mnt/pi-hdd/torrents/4x4-test`            |
| `X4_GEOIP_DIR`       | the folder of the GeoIP country files    | `/usr/share/GeoIP`                         |

```bash
X4_LIVE_URL=http://my-daemon:9091/transmission/rpc \
X4_LIVE_REMOTE_DIR=/srv/torrents/4x4-test X4_LIVE_LOCAL_DIR=/mnt/daemon/4x4-test just test-live
```

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

The country flags are the `3x2` pictures of
[country-flag-icons](https://gitlab.com/catamphetamine/country-flag-icons) 1.6.20, embedded as
published. They are MIT licensed, copyright (c) 2020 catamphetamine; their notice is in
[`crates/x4-app/assets/flags/LICENSE`](crates/x4-app/assets/flags/LICENSE).

This product includes GeoLite2 Data created by MaxMind, available from
<https://www.maxmind.com/>. (4x4 reads the country files of the `geoip-database` package to find
the countries of peers; it does not ship them.)
