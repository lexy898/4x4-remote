# Development

Read [`AGENTS.md`](../AGENTS.md) before changing anything: it holds the commands, the architecture
and the rules every change must follow.

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

## Windows and macOS

4x4 is developed on Linux, and CI also builds it and runs its tests on Windows and macOS. Next to
`check`, which runs the whole gate on Ubuntu, two jobs of [`ci.yml`](../.github/workflows/ci.yml)
run clippy and the tests on the other systems: `windows` on `windows-2025` and `macos` on
`macos-15`. They take the toolchain of `check` (the MSRV on pushes and pull requests, the newest
stable in the weekly run) and run:

```bash
cargo clippy --workspace --all-targets --features x4-app/synthetic --locked -- -D warnings
cargo nextest run --workspace --features x4-app/synthetic --locked --no-tests=warn
```

The rest of the gate (formatting, spelling, the docs, the lint policy and the dependency checks)
does not depend on the system, so only `check` runs it. The `windows` job turns off git's
`core.autocrlf` before the checkout: Git for Windows would check text files out with CRLF line
endings, and the tests that read committed text files, such as the README's table of keys, expect
them as committed. A test of something only one kind of system has (a Unix socket, a FIFO, the XDG
directories) is compiled for that kind alone; a test that gives its subject Unix paths has a twin
for Windows with the same cases in Windows paths.

CI runs on pull requests and on pushes to `master`. To run it on a branch before its pull request:
`gh workflow run ci.yml --ref <branch>`.

### The Windows quick check

A CI round takes long, so check code for Windows on Linux before pushing it, above all code under
`cfg(windows)`, which a Linux build never compiles. Clippy for the Windows target compiles and lints
the workspace without linking it, so it needs neither Windows nor MSVC, only a rustup toolchain
with the `x86_64-pc-windows-msvc` target. Arch's `rust` package has no rustup, so keep one in a
folder of its own, apart from the system Rust:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    RUSTUP_HOME=$HOME/.cache/x4-scratch/cross/rustup \
    CARGO_HOME=$HOME/.cache/x4-scratch/cross/cargo \
    sh -s -- -y --no-modify-path --profile minimal --default-toolchain 1.99 \
    --component clippy --target x86_64-pc-windows-msvc
```

Then, in the checkout:

```bash
RUSTUP_HOME=$HOME/.cache/x4-scratch/cross/rustup CARGO_HOME=$HOME/.cache/x4-scratch/cross/cargo \
PATH=$HOME/.cache/x4-scratch/cross/cargo/bin:$PATH CARGO_TARGET_DIR=$HOME/.cache/x4-scratch/cross/target \
cargo clippy --workspace --all-targets --features x4-app/synthetic --locked \
  --target x86_64-pc-windows-msvc -- -D warnings
```

The build has a target directory of its own and leaves the one of `just check` alone. It runs no
tests: they run in the `windows` job alone.

### macOS in CI only

macOS has no such check. Clippy for `aarch64-apple-darwin` stops at `gpui-pre-apple`, the macOS
part of the GUI toolkit: its build script makes the Metal shaders of the renderer only when it runs
on macOS, and the crate does not compile without them. The `macos` job is the only check of code
for macOS, so a change to it needs a green CI run before it is merged.

## Packaging

Without a checkout, the package is built from GitHub, as [Install](../README.md#install) says.
With a checkout, `just package` builds the local `master` branch of it instead, so build it after
what you want is merged into `master`:

```bash
git switch master
git pull
just package
sudo pacman -U ~/.cache/4x4-pkg/packages/4x4-remote-git-<version>-1-x86_64.pkg.tar.zst
```

`just package` builds the package with `makepkg` and prints the path of the file it made as its
last line; that file is what `pacman -U` installs. The version names the commit it was built from,
such as `1.0.0.r320.g1a2b3c4` (the workspace version, the number of commits and the commit). The
build needs the tools and libraries of [Build from source](#build-from-source); makepkg names any
package that is missing, and `just package` never installs anything itself. The first build takes
about four minutes. `packaging/arch/PKGBUILD` itself is the file for the AUR, which builds GitHub's
`master`; makepkg runs a copy of it whose source is this checkout instead, and `just package` stops
if the file has no such source to replace. Everything it makes stays in `~/.cache/4x4-pkg` (crates
come from cargo's own cache, `~/.cargo`): the build dir keeps the compiled crates, about 2 GiB, so
later builds are faster, and all of it is safe to delete.

`just package` always builds the commits of the local `master` branch: not the working tree, so
uncommitted changes are left out, and not the branch checked out; its first line names the commit it
builds. While `master` has no `packaging/arch/PKGBUILD`, `just package` stops before it writes
anything and says to merge and pull first. Outside a git repository (a copy of the files without
`.git`) it also stops before it writes anything, but it shows git's message and does not mention
merging. If the local `master` is behind the `origin/master` you last fetched, `just package` warns
and still builds the local `master`; it fetches nothing itself. After the checkout has moved to
another directory, the next build starts over from it and takes the full four minutes again. The
package installs `/usr/bin/4x4`, its desktop file in `/usr/share/applications` and its icon in
`/usr/share/icons/hicolor`, so 4x4 Remote shows in the application menu with its icon at once,
without logging out. It also installs a second, hidden desktop file for debug builds (described
below). It depends on `geoip-database`, which holds the country files that the peers' flags come
from (see [Files and logs](user-guide.md#files-and-logs)), and installs, next to the licenses of
4x4, the notice of the flag pictures it embeds (`LICENSE-country-flag-icons` in
`/usr/share/licenses/4x4-remote-git`; see [License](../README.md#license)).

To update, run the four commands of a checkout again; pacman replaces the installed version.
Before 1.0 the package was called `4x4-git`: `pacman -U` of `4x4-remote-git` asks whether to
remove it (answer `y`; your settings stay). Its build files, `~/.cache/4x4-pkg/build/4x4-git` and
`~/.cache/4x4-pkg/sources/4x4`, are no longer used and can be deleted, and the first build of
`4x4-remote-git` takes the full four minutes.

To uninstall, first run `just default-handler --undo` if you made 4x4 the default app for
`.torrent` files and magnet links (described below), then `sudo pacman -R 4x4-remote-git`. Your
`config.toml`, `state.toml` and logs stay (see [Files and logs](../README.md#files-and-logs)). The
build files in `~/.cache/4x4-pkg`, about 2 GiB, can be deleted: `rm -rf ~/.cache/4x4-pkg`.

### Making 4x4 the default app, from a checkout

Without a checkout, use the desktop's settings or `xdg-mime`, as
[Install](../README.md#install) says; the command below needs one.

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

### The icon

The icon, a gearshift pattern, is one SVG, `packaging/linux/io.github.lexy898.FourByFour.svg`,
installed into `hicolor/scalable/apps`, which serves every size. A separate drawing for small
sizes would be installed under the same name into `16x16/apps`, `22x22/apps`, `24x24/apps` and
`32x32/apps` of `hicolor` too, four more `install` lines in the PKGBUILD's `package()`; KDE takes
the icon of a fixed size when one matches the size it draws, and the scalable one for every other.

## Debug builds

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

Debug builds (`just run`) give the window the Wayland app id `io.github.lexy898.FourByFour.Devel`
instead of `io.github.lexy898.FourByFour`, so window rules made for test windows (for example,
open on another screen without taking focus) never catch the window you actually use, and a debug
build never hands its files and links to the 4x4 you use. Set `X4_DEVEL=1` to give a release build
the same id; `just measure` does.

Debug builds (`just run`) and release builds never hand over to each other, and a 4x4 in
[`just virtual`](#windows-off-screen) never hands over to the one on your desktop.

A debug build offers the files and links it is started with in its add dialog too (see
[Launch arguments](user-guide.md#launch-arguments)). From the source tree, `just run 'magnet:?xt=…'`
passes them on; quote a magnet link, as its `&` means something to the shell. From the source tree,
without the package, start it with them or drop them on its window.

## Architecture

The workspace has three crates. `x4-rpc` speaks Transmission's RPC over HTTP: JSON-RPC 2.0 to
Transmission 4.1 and newer, and the legacy protocol to 3.00 – 4.1.0-beta, which it detects at each
connect. `x4-core` holds the app's model and logic, with no UI code, down to what the main window
shows (table rows, subtitles, filter counts, status bar), so the views only draw it. `x4-app` is the
`4x4` binary with the window and the Fluent messages that word everything it shows.

All network traffic runs on one worker thread, `x4-service`, which `x4-core` starts. It polls the
daemon on a schedule (faster while the window is visible), reconnects with a growing pause when the
daemon stops answering, and carries out the actions the user asks for, one request at a time. The
window talks to it through two channels: commands go in, and snapshots of the state and the results
of actions come out. Sending a command never blocks, the window awaits the events without blocking
its thread, and closing it does not wait for the worker to finish its last request, so the window
never waits on the network. When several snapshots queue up while the window is busy, it shows only
the newest, and it redraws only when what it shows has changed, so an idle daemon costs no frames.

## Windows off screen

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

## Synthetic demo

`just demo-synthetic` opens the main window over a synthetic daemon of 2000 torrents in every state,
made up in memory: nothing goes over the network, and none of 4x4's files is read or written (the
log goes to stderr only). The rates of the active torrents change on every poll. `--static` keeps
every rate at 0, an idle daemon; `--scroll` scrolls the table by one row per frame for 10 seconds,
logs the intervals between the frames and quits; `--select N` selects the row at index `N` (from 0)
once the list is there, and `--extend K` then extends the selection `K` rows down, as Shift+Down
would. `--showcase` makes 24 torrents instead, named like free software, films and texts that anyone
may share (an Arch Linux ISO, "Big Buck Bunny (2008) 1080p"), each as big as such content is (the
ISO 1.4 GiB, the Wikipedia dump 24 GiB), in a mix of states: every main state is among them, and the
first rows show several. None is the demo's torrent of 2000 files or the one of 50 000. `--theme
system|light|dark` starts the window with that theme instead of following the desktop's, so that a
shot of either theme changes nothing on the desktop. Both are for [screenshots](#screenshots); the
default demo is the same with and without them. The made-up daemon carries out the actions on its
own torrents: pausing, starting (a torrent that lacks data downloads, a complete one seeds),
starting ahead of the queue, verifying (the torrent checks for three updates, then goes back to what
it was), removing, the speed settings, and what the Files tab changes: which files download, their
priorities and downloading in order. It also adds torrents, from the content of a `.torrent` file
(with the files it lists, in its order) or from a magnet link (which stays without its metadata),
honouring `paused`, the folder and the files left out; it reports a torrent it already has as a
duplicate and changes nothing; and its free space answers for the session's folder and the folders
of its torrents, and their parent folders, with Transmission's error 5, "No such file or directory",
for any other. Its torrents have files, most one, some a folder of several, a few dozens, "Synthetic
torrent 0005" (`--select 4`) 2000 in folders, and "Synthetic torrent 0006" (`--select 5`, Down from
it), a seed, 50 000 in folders, the most the Files tab shows; their pieces follow the download, from
the start when it is in order. The running torrents have peers, at addresses reserved for
documentation (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 and 2001:db8::/32), which stay from one
read to the next while the torrent's rates are shared out among them; a country database the demo
makes in memory puts them in countries (Japan, South Korea and Finland among them, whose flags are
mostly white), in the region EU or in none, and the installed GeoIP files are not read. Their
trackers, none, one or twelve in tiers with a backup in the first, announce every 30 minutes counted
from the clock, and a few wait for a slot or announce right now; some fail, time out or fail only
their scrape, and those of a stopped torrent that has not run since the made-up daemon started were
never contacted. "Synthetic torrent 0016" (`--select 15`; with `--static`, "Synthetic torrent 0011",
`--select 10`) is busy: it downloads from 150 peers and has a tracker of every status, so that `just
virtual just demo-synthetic --select 15 --press alt-3` shows a long Peers tab and `--press alt-4`
the Trackers tab with each kind of row. `--press KEY`, which can be given several times, presses
each key in turn once the list is there and selected, one every three seconds, as on the keyboard
(`secondary-` is Ctrl, or ⌘ on macOS): for example `just virtual just demo-synthetic --static --select 5 --press secondary-p --press space`
pauses the sixth torrent and starts it again, `--press shift-delete` opens the dialog that would
delete its files, and `--press secondary-o` opens the desktop's file dialog, with its filter for torrent
files. `--add PATH_OR_MAGNET`, which can also be given several times, asks to add each `.torrent`
file or magnet link at the start, as if 4x4 had been opened with them, which opens the add dialog;
what it does not open makes its notification. The window is set to a made-up server,
`synthetic.invalid:9091`, and `--connection` opens the Connection dialog after the first frame, as
`Ctrl+,` would. Its Test answers for any server with the made-up daemon's version ("Connected:
Transmission 4.1.3 (synthetic)"), and its Save switches to a new made-up daemon, whatever other
server the fields name, as a switch in 4x4 does; it writes no `config.toml`. `just virtual just
demo-synthetic --scroll` runs it off screen. With `RUST_LOG=x4_synthetic=debug` the made-up daemon
logs each request it answers with the bytes of the request and of the answer, which is how the
requests and the bytes the app asks of a daemon are counted (see [Performance](performance.md)). The
demo is the `x4-synthetic` binary of the `synthetic` feature, so the `4x4` build does not contain
it.

## Screenshots

`docs/images/` holds the pictures the [README](../README.md) shows: `main-dark.png` and
`main-light.png`, the main window with a torrent selected and its details open, which the README
picks between by the reader's theme, and `peers-dark.png`, the Peers tab. They are shots of the
`--showcase` demo (see [Synthetic demo](#synthetic-demo)), so they show no one's torrents, server or
addresses: the names are the showcase's and the peers are at addresses reserved for documentation.
Retake them when the UI or the showcase changes, so that the README shows what 4x4 looks like.

The shots are taken inside `just virtual`, off your screens, with spectacle. The session's own
`HOME` is removed when it ends, so `shots` is a directory outside it. The demo is built first, and
`--theme` makes each shot in its theme without a change of the desktop's colors:

```bash
cargo build -p x4-app --features synthetic --bin x4-synthetic --locked
export shots=/tmp/4x4-shots
mkdir -p "$shots"
just virtual bash -c '
shot() {
    name=$1 wait=$2
    shift 2
    target/debug/x4-synthetic --showcase "$@" 2>/dev/null &
    demo=$!
    sleep "$wait"
    spectacle -b -n -f -o "$shots/$name-full.png"
    kill "$demo"
    wait "$demo" || true
}
shot main-dark 12 --theme dark --select 4
shot main-light 12 --theme light --select 4
shot peers-dark 14 --theme dark --select 15 --press alt-3
'
```

`--select 4` is the fifth row, a download with 40 peers; `--select 15` is the busy download, with
150 peers, and `--press alt-3` opens its Peers tab three seconds after the list is there. The rows
are in the default order, by name. The rates change on every poll, so what a shot holds depends on
how long it waits; with these waits the shots come out the same each time. The virtual screen is
1440x900 and the window opens 1200x720, centered, under KWin's title bar, so the crop
`1200x750+120+75` is the window and nothing else. `zopflipng` squeezes the files without loss, to
about 120 to 130 KiB each; keep each under 150 KiB (the repository is cloned by everyone):

```bash
for name in main-dark main-light peers-dark; do
    magick "$shots/$name-full.png" -crop 1200x750+120+75 +repage -strip "$shots/$name.png"
    zopflipng -y "$shots/$name.png" docs/images/$name.png
done
```

It needs ImageMagick (`magick`), `zopfli` (`zopflipng`) and spectacle, besides what
[Windows off screen](#windows-off-screen) needs. Look at each picture before committing it: only
the showcase's names, only addresses reserved for documentation, nothing cut off, no black margin
around the window.

## Live tests

`just test-live` runs the tests in `crates/x4-rpc/tests/live.rs`, `crates/x4-core/tests/live.rs` and
`crates/x4-app/tests/live.rs` against a real Transmission daemon, 3.00 or newer; `just test` and
`just check` skip them. They never touch existing torrents or the daemon's settings. Two tests only
read the server version: x4-rpc's through the client, and x4-app's as the Connection dialog's Test
does, with the `session_get` that Test sends (and the second one, in the legacy protocol, to a
daemon older than 4.1), whose answer must name a Transmission 3 or 4. Each of the others makes a
torrent of its own, a random 3 MiB file or a folder of a few random files of a few KiB, adds it as a
trackerless torrent into a test directory, works only on that torrent (stop, start, start now,
verify, reannounce, sequential download on and off, which files are downloaded and their priorities,
a file index past the last file, which the daemon refuses, adding it twice), and removes it together
with its data. Most write the data into the test directory first. Some do not: they add their
torrent paused, or by a magnet link of it, and the daemon then has none of its data. One removes its
torrent from the list only, checks that the daemon kept the file, and then deletes the file itself.
The tests of x4-core read the details of their torrent as the app does, in full and then only their
progress, with and without the trackers, and check that the app understands every answer; and they
add a torrent as the add dialog does, with a file unchecked that comes after a padding file and
before an empty file, and check that the daemon numbers the files, those two included, in the order
4x4 read them from the `.torrent`, which is not the order of their names, with only that file left
out. The tests of adding check that a torrent added paused stays stopped, though all its data is
there; that adding it again, started, into another folder and with another file left out, is a
duplicate that changes nothing; that a folder that does not exist yet, `4x4-live-new-<hex>` in the
test directory, is kept as it was sent and made only when the torrent starts; and that a magnet link
of a torrent of the test's own is added and listed, paused and without its metadata. One test of
x4-core changes nothing at all: it reads the peers and the trackers of the daemon's torrent with the
most peers as the Peers and the Trackers tab do, and checks what they become, printing only counts.
Every other test checks that every other torrent kept its status, sequential mode, file priorities
and wanted files, and prints how many torrents the daemon lists when it starts and once it has
removed its own. The torrent and its data are removed even when a test fails, and only from the
folder it was added to: the test directory, or the folder of its own that the daemon made directly
in it. The folder the daemon reports must be that one once both are normalized, so a doubled or a
trailing `/` makes no difference, and a folder with any control character in its name is never it.

On a daemon older than Transmission 4.1 the tests speak the legacy protocol, as the app does, and
some of what they check is not there: sequential download before 4.1.0-beta.2, the peers' `peer_id`
and `bytes_to_*` before 4.1, a tracker's `sitename` in 3.00, and the empty files and their folders
that a torrent makes when it starts before 4.1.0-beta. Transmission 4.0 leaves a torrent's empty
files out and numbers the rest differently, so there the add test cannot leave a file out. A test
then says on stderr what it skips, and checks what holds instead: a field the daemon lacks reads
none, and the add form offers no choice of files.

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

### Old daemons in Docker

For the legacy protocol, `just legacy-daemons up` starts Transmission 3.00, 4.0.5 and 4.1.0-beta.2,
each from the package of its distribution (Debian 12, Ubuntu 24.04, Debian 13), in Docker containers
that run as you, with their RPC on 127.0.0.1 ports 19300, 19405 and 19412. A fourth daemon makes a
test torrent of random files and seeds it to them over local peer discovery, slowly, so that each
keeps a peer for the tests of the peers and the trackers. DHT and port forwarding are off and the
torrent's tracker refuses at once, so nothing leaves the machine. `up` needs Docker, and the network
the first time it builds the images; it waits up to three minutes for every daemon to have its peer.

`just legacy-daemons status` gives each daemon's version and peers, and
`just legacy-daemons env <name>` (`v3.00`, `v4.0.5` or `v4.1.0-beta.2`) the `X4_LIVE_URL`,
`X4_LIVE_REMOTE_DIR` and `X4_LIVE_LOCAL_DIR` of one daemon, to `eval` before running a single test
against it. `just test-live-legacy` runs the live tests, without `geoip_live`, against the three in
turn, and fails at the end if any of them failed. `just record-fixtures-legacy` records the fixtures
of `crates/x4-rpc/tests/fixtures/legacy/<name>/` from them, with read-only calls, as
`just record-fixtures` records those of `crates/x4-rpc/tests/fixtures/` from the daemon at
`X4_LIVE_URL`.

`just legacy-daemons down` removes the containers, their network and the state directory, which
holds each daemon's settings and downloads: `~/.cache/x4-scratch/legacy-daemons`, or the directory
`X4_LEGACY_DIR` names. `up` leaves the file `.x4-legacy-daemons` in it and takes no directory that
holds other files without that file, and `down` removes only a directory that has it; both refuse
`/` and your home directory.

```bash
just legacy-daemons up
just test-live-legacy
eval "$(just legacy-daemons env v4.0.5)"
cargo nextest run -p x4-core --test live --run-ignored only
just legacy-daemons down
```

## Localization

The interface is in English. Every text the app shows is a message in
[`crates/x4-app/i18n/en/x4.ftl`](../crates/x4-app/i18n/en/x4.ftl), written in
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
