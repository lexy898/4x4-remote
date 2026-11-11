# User guide

The full reference of 4x4 Remote, part by part. The [README](../README.md) has the short
version, the [keys](../README.md#keys) and the files 4x4 keeps.

This guide names the keys as they are on Linux and Windows. On macOS, ⌘ (Command) takes the place
of Ctrl everywhere (⌘F for `Ctrl+F`, and a ⌘-click toggles a row); ⌘⌫ removes the selected torrents
and ⌥⌘⌫ removes them with their files, as `Delete` and `Shift+Delete` do. The tooltips and the
search box name the key of the system.

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

Test asks the server for its version once (one `session_get`, and a second one, in the older
protocol, for a daemon older than Transmission 4.1; each given up after 5 seconds without an
answer), off the window's thread, so that the window never waits. The button says "Testing…" until
the answer comes; a line under the fields then says, in green, "Connected: Transmission 4.1.3
(…)", or in red why not: "The server cannot be reached", "The server did not answer in time", "The
server rejected the credentials", "The server does not accept requests from this computer" (the
daemon's whitelist), "Transmission 3.00 or newer required (server RPC 15)", and the like. Any edit
takes the line away, and drops the answer of a test still running, so that the line always speaks
of the fields as they are. Save needs no test.

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

## The parts of the window

The window has five parts: a toolbar along the top, with the search box, the Add file and Add
magnet buttons, the Start and Pause buttons and the Remove menu on the left and the Connection
button, the theme button and the "Columns" menu on the right;
the filter sidebar down the left side; the torrent table, which takes the rest; the details panel
under the table; and the status bar along the bottom.

## The torrent table

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

## Sorting and columns

Click a column header to sort by it; click it again to reverse the order. ▲ or ▼ after the header
shows the order. Size, progress, speeds, ratio and peers sort largest first at the first click;
names sort from A to Z, the time left soonest first (then an unknown time, then "—"), and the state
with errors first and paused torrents last. Drag a header to move the column, and drag the edge of a
header to resize it. "Columns" in the toolbar shows and hides columns; the name column always stays.
The sort order and the columns are remembered between runs.

## Selecting torrents

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

## Actions

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

## The context menu

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

## The clipboard

Two notes on the clipboard:

- the magnet links reach the clipboard only if 4x4 still has the focus when the daemon's answer
  comes, a moment after `Ctrl+C`: 4x4 does not set the clipboard while another app has the focus
  (on Wayland it could not). If you have switched to another app by then, nothing is copied and a
  notification asks you to copy them again. The hashes are copied at once, so this never happens to
  them;
- 4x4 itself holds the copied text and hands it to the app that pastes it: when 4x4 quits, the
  copied text is lost unless a clipboard manager keeps it.

## Removing torrents

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

## When an action works

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

## The details panel

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

## The General tab

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

## The Files tab

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
that a file can be played while it downloads. It needs Transmission 4.1 or newer on the server: on
an older one the switch is disabled and the line under it says why. A change shows once the
daemon's next answer does, a moment later, not before. While the daemon is not connected, the
checkboxes, the priorities and the switch are disabled. Until the details have come, the tab says
"—" under the switch, and so it does while the files of a torrent of more than 1000 are sorted into
their tree when the tab first shows them, which is done off the window's own thread, so that the
window keeps answering meanwhile (some 20 ms for 50 000 files). A magnet link has no files until its
metadata has come; the tab says so, and the switch can be set already. A file list that would take
too much memory to show, such as one of more than 50 000 files or of files nested hundreds of
folders deep, is not drawn: the tab says the list is too large to show.

## Keys of the file list

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

## The Peers tab

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

## The Trackers tab

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

## How often the details are read

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
KiB per second; on one that seeds, one request of 3 KiB a minute (see
[Performance](performance.md)).

## The sidebar

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

## Search

The search box in the toolbar narrows the list, as you type, to the torrents whose names contain
every word typed, in any order and ignoring case. It works together with the filter. `Ctrl+F` moves
to the search box from anywhere in the window, and `/` from anywhere but a text field (see
[Keys](../README.md#keys) for keyboard layouts); `Escape` in the box empties it and goes back to the
table. The search is not remembered between runs.

## The status bar

The status bar at the bottom shows:

- a dot for the connection: green when connected, grey while connecting or when no server is set,
  yellow while reconnecting and red when the daemon is too old (4x4 needs Transmission 3.00 or
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

## Speed limits

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

## Adding torrents

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

## The add dialog

The add dialog shows the torrents to add, one at a time; nothing is added until you press Add. A
`.torrent` file is read only when its turn comes, and one that cannot be offered is skipped with a
notification that names it and says why, and is not counted among the torrents to add: it does not
exist, 4x4 may not read it, it is not a regular file or is larger than 16 MiB, it is not a torrent
file or holds no torrent, it is a BitTorrent v2 torrent only, which Transmission cannot add, or
the daemon would refuse it as broken (as when it has no files, two files of the same path, or piece
hashes that do not match its size). A file that takes longer than a moment to read, as on a network
folder that hangs, opens the dialog with "Reading the file…" and Skip, which moves on to the next
torrent. At most 100 torrents wait; a notification says how many more were left out. The title says
which torrent of how many it shows, such as "Add torrent (2 of 5)"; under it come the torrent's name
and size, and for a magnet link its field, checked as you type, with the link's name or its hash. A
magnet link must start with `magnet:?` and name one BitTorrent info hash (40 hexadecimal or 32
base32 characters), without a control character such as a line break, and in at most 64 KiB; a
bare hash, a link with several hashes and one with only a BitTorrent v2 hash, which Transmission
cannot add, are refused with the reason under the form.

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
than 50 000, or files nested so deep that their tree would take too much memory. On Transmission
4.0, a torrent with empty files is added with all its files, because 4.0 leaves empty files out and
numbers the rest differently. A dialog that opens before 4x4 has reached the server offers the
choice until it learns that the server is 4.0, and then drops it; until 4x4 knows the server's
version, Add stays disabled while a file of such a torrent is unchecked. A magnet link has no choice
of files either, as its files are known only once its metadata has come. "Start immediately" starts
as the daemon's setting for new torrents; without it, the torrent is added paused.

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

## Launch arguments

4x4 also offers the `.torrent` files and magnet links it is started with, such as
`4x4 Show.torrent 'magnet:?xt=urn:btih:…'`, or that a file manager or a browser opens with it, in
the add dialog, in their order. A relative path is taken from the folder 4x4 was started in, and a
`file://` address must name a file on this computer, without a query. Web addresses (`http://`,
`https://` and the like), bare info hashes, options (anything starting with `-`, also after a `--`,
except `--version` and `-V` before it) and anything else are refused, with one notification that
counts them and says why the first one was; a first `--`, which a shell or `cargo run` may pass, is
skipped. At most 100 are taken, as many as wait to be added; the notification that some were left
out counts the others, which are not looked at.

`4x4 --version`, or `4x4 -V`, prints the name and the version of the build, such as
`4x4 Remote 1.0.0`, on one line and exits. It opens no window, writes no log and does not reach the
4x4 that runs, so it answers at any time; it is understood next to files too, but not after a `--`.
It is the only option 4x4 knows: every other option is refused, as above.

## One 4x4 at a time

One 4x4 runs at a time for each user. Starting it again, with or without files and links, hands
them to the 4x4 that runs already and exits at once: that one offers them in its add dialog, after
any it is showing, and brings its window to the front. Where the desktop does not let it come to the
front, the window asks for your attention instead (a highlighted task bar entry). If the 4x4 that
runs does not answer within 2 seconds, because it hangs, the new start opens a window of its own
with them, so a click on a file is never lost; the new start takes the place of a 4x4 that is
quitting, also one that quits while the new start waits for it. Without `$XDG_RUNTIME_DIR`, every
start opens a window of its own and the log says so.

## The desktop entry

The desktop entry in `packaging/linux/`,
[`io.github.lexy898.FourByFour.desktop`](../packaging/linux/io.github.lexy898.FourByFour.desktop),
offers 4x4 as a handler for `.torrent` files and `magnet:` links (`Exec=4x4 %U`, one process for all
the files and links of a launch). The package installs it (see [Install](../README.md#install)), so
file managers and browsers list 4x4 among the apps for them; Install also says how to make 4x4
their default. 4x4 never makes itself the default handler without asking.

## When the daemon stops answering

When the daemon stops answering, the last data stays on screen, with the list, the counts in the
sidebar and the numbers in the status bar dimmed, until it answers again. If the part of 4x4 that
talks to the daemon stops or cannot start, the status bar says "Service stopped" or "Service could
not start", the data is dimmed for good, and the reason is in the log.

## Theme

The button in the toolbar switches the theme: the system's (light or dark, following the desktop),
light, or dark. The choice is remembered between runs.

## Polling in the background

While the window is minimized or does not have the focus, 4x4 asks the daemon for updates every
10 seconds instead of every 1.5 seconds, to go easy on a small server.

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

The README's [Files and logs](../README.md#files-and-logs) lists the files 4x4 keeps in the XDG
directories, `config.toml`, `state.toml` and the daily log, with an example `config.toml`.

While it runs, the first 4x4 also keeps two files in `$XDG_RUNTIME_DIR` (`/run/user/<uid>`), named
after its app id (with `.Devel` in debug builds): `io.github.lexy898.FourByFour.lock`, which says
that it runs, and `io.github.lexy898.FourByFour.sock`, the socket later starts hand their files and
links to. Only you can use them. The socket is removed when 4x4 quits, and one left by a 4x4 that
crashed is replaced by the next one; the lock file stays and is safe to delete while 4x4 is not
running.

4x4 reads `config.toml` at startup and connects to the address of its `url`. There is no built-in
server: without the file, or without `url` in it, no server is set, and the Connection dialog opens.
The address may not hold the user name or the password (`http://alice:secret@…`), which belong in
their own keys, nor a query or a fragment; without a port it means port 80. A file 4x4 cannot use,
such as one that is not valid TOML, has a value of the wrong type or an `https://` address, is
reported in the log and in the status bar ("Config file not used"; the tooltip says why, and
"Dismiss" hides it); no server is set then either, and the dialog opens. Neither ever shows the
password.

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
