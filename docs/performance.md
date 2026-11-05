# Performance

The latest figures, of stage 9 (2026-11-05, a release build in `just virtual` on a laptop with an
AMD Radeon 780M, reaching no daemon): the first frame in 198 ms, 15.6 MiB of private memory,
54.4 MiB of PSS, 140.9 MiB of RSS, 31 threads, 0.00 to 0.10 % of one core when idle, and a 40.6 MiB
binary. Each section below is a stage, oldest first, and says how it was measured.

## Measuring, and the first window (stage 1)

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
measures the release build of the [synthetic demo](development.md#synthetic-demo) instead of the
app, and arguments after `--` go to the program, as in `just measure --synthetic --cpu 30 --
--static`. `--binary <path>` measures that file and builds nothing, such as the installed
`/usr/bin/4x4` or one taken out of a package (not with `--synthetic`, which builds the demo); a
relative path is taken from the repository's directory. `--offline` gives the program a temporary
config directory of its own, removed afterwards, whose `config.toml` names a closed port on this
machine (`http://127.0.0.1:1/transmission/rpc`), so the app reaches no daemon and keeps
reconnecting; it is refused with `--synthetic`, as the demo reads no config and reaches no network
anyway. Without it the app reads your `config.toml`; inside `just virtual`, whose config directory
is empty, no server is set, so the app polls nothing and shows the Connection dialog over the
window, which is not how it runs in use. For example, `just virtual just measure --binary
/usr/bin/4x4 --offline --cpu 60` measures the installed 4x4 off screen and off the network. Measure
the app with no development 4x4 running in the same session, or inside `just virtual`: otherwise the
measured start hands over to it and exits. (`just measure` gives the program the development app id,
so it never hands over to an installed 4x4.)

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
([`vendor/gpui-pre-wgpu`](../vendor/gpui-pre-wgpu/PATCHED.md)) until upstream fixes this
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

## The main window (stage 2)

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
30`, the release build of `4x4`). The 2000 torrents are the [synthetic
demo](development.md#synthetic-demo), the release build of its `x4-synthetic` binary, measured after
a 10 second settle (`X4_MEASURE_SETTLE_SECONDS=10 just virtual just measure --synthetic --cpu 30`,
with `-- --static` for idle); while busy, 513 of them change their rates or progress on every poll.
The synthetic daemon runs inside the measured process, so its own work, making up the answers,
counts in those two columns.

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

## The details panel (stage 4)

Measured on 2026-11-02 on the same laptop, inside the headless KWin of `just virtual`, with the
release build of the [synthetic demo](development.md#synthetic-demo), its 2000 torrents busy. The
torrent of 2000 files is "Synthetic torrent 0005" (`--select 4`), which downloads; "Synthetic
torrent 0014" (`--select 13`) seeds. The requests and their bytes were counted in the demo's debug
log (`RUST_LOG=warn,x4_synthetic=debug,x4_core::service=debug`) over 60 seconds, from 15 seconds
after the start; the bytes are those of the bodies, without the HTTP headers. The stage 4 targets
are at most 1.75 requests per second with the panel open on a torrent that downloads, and 1.45 on
one that seeds.

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

## Adding (stage 5)

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

## The allocator (stage 6)

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

## Installing (stage 6)

Measured on 2026-11-03 on the same laptop, inside the headless KWin of `just virtual`, with release
builds; each figure is the median of three runs, each the first start of its own session, with the
runs of the app and of the package taken in turn. The app (`just virtual just measure --offline
--cpu 60`) and the binary of the package that `just package` builds from the same commit (see
[Packaging](development.md#packaging)), taken out of the package file with `bsdtar` instead of
installed (`just virtual just measure --binary <dir>/usr/bin/4x4 --offline --cpu 60`), reached no
daemon and kept reconnecting to the closed port. The synthetic demo answered from memory with
nothing changing (`X4_MEASURE_SETTLE_SECONDS=10 just virtual just measure --synthetic --cpu 60 --
--static`); as in stage 2, the targets are those of the app with a real daemon, and the demo's
column also carries the synthetic daemon. The CPU is that of the 60 seconds after the memory
figures; "not met" marks every cell over its target:

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

## Peers and trackers (stage 7)

Measured on 2026-11-04 on the same laptop, inside the headless KWin of `just virtual`, with release
builds; each figure is the median of three runs, each the first start of its own session. The
[synthetic demo](development.md#synthetic-demo) had its 2000 torrents busy and "Synthetic torrent
0016" (`--select 15`) selected, which downloads from 150 peers and has twelve trackers, with its
General tab, its Peers tab (`--press alt-3`) or its Trackers tab (`--press alt-4`) open
(`X4_MEASURE_SETTLE_SECONDS=15 just virtual just measure --synthetic --cpu 30 -- --select 15 --press
alt-3`); "not met" marks every cell over its target:

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

## The connection (stage 8)

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

## The gpui-kit 0.9 bump (stage 9)

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
  [`PATCHED.md`](../vendor/gpui-pre-wgpu/PATCHED.md) stand. The only `WARN:` lines were the usual
  ones on the memory figures: PSS in 2 of the 6 runs above (4 of the 6 of the stage 8 app, whose
  55.0 MiB is a hair over the proposed 55) and private memory in the first run of a session.
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
-  Spot checks with the [synthetic demo](development.md#synthetic-demo) (a debug build) in `just
  virtual`, each next to the stage 8 app's picture of the same scene, taken the same way. The main
  window with row 4 selected and General open (`--select 4`) is identical to the pixel, dark and
  light. The add dialog over a made-up magnet link with a 120-character name of dots, dashes and
  slashes breaks the line after a slash ("`…/Extras/`" over "`Behind-The-Scenes…`") where the stage
  8 app broke it before one: GPUI now wraps by Unicode line breaking, and a narrow General tab does
  the same with the path ("`/data/torrents/`" over "`Synthetic torrent`"); nothing else wraps
  differently, and the hash still breaks at any character. In the torrent's context menu (`--select
  4 --right-click --press down`) the first Down lands on Pause, the first item that can be clicked,
  as the torrent is downloading and Start is dimmed; the stage 8 app showed no highlight there. The
  Connection dialog (`--connection`) is the same as before. A window resized 0.5 to 1.2 seconds
  after start (KWin scripting through D-Bus), to 900x600, 1300x820 and 760x520 in turn, is redrawn
  whole at 1300x820 and at 760x520, the two sizes looked at, with no stale or black area.
