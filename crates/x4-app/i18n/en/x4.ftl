# The messages of 4x4, English; the fallback of every other language.
#
# Every language directory under i18n/ must define the same messages, with the same $variables
# (the parity test in src/i18n/parity.rs checks it). The app formats sizes, speeds and ratios
# itself and passes them as ready strings; counts are passed as numbers so that plural rules apply.

## Numbers

# The decimal separator of the language. The app formats numbers with a dot and swaps it for this.
number-decimal-separator = .
# What sets the groups of three digits of a whole number apart, as in 1,000,000. Fluent trims a
# plain space (U+0020) at either end of a value, so a space separator must be written { " " }, or
# be a no-break space (U+00A0) or a narrow no-break space (U+202F).
number-group-separator = ,

## Units

unit-b = B
unit-kib = KiB
unit-mib = MiB
unit-gib = GiB
unit-tib = TiB
unit-b-per-s = B/s
unit-kib-per-s = KiB/s
unit-mib-per-s = MiB/s
unit-gib-per-s = GiB/s
unit-day = d
unit-hour = h
unit-minute = m
unit-second = s

# A size or a speed. $value: the formatted number; $unit: one of the unit-* messages.
quantity = { $value } { $unit }

## Progress and ETA

# $percent: whole percent (a number); $size: the formatted size of the torrent.
progress-of = { $percent }% of { $size }

eta-none = —
eta-unknown = ∞
# The largest unit and the one right below it, such as "1 d 2 h"; the second is left out when it is
# zero, such as "1 d".
# $major and $minor: amounts (numbers); $major_unit and $minor_unit: unit-day, unit-hour, ...
eta-major = { $major } { $major_unit }
eta-major-minor = { $major } { $major_unit } { $minor } { $minor_unit }

ratio-none = —
ratio-infinite = ∞

## Torrent states

state-downloading = Downloading
state-seeding = Seeding
state-paused = Paused
state-queued = Queued
state-checking = Checking
state-error = Error

## The line under a torrent's name

# $connected: peers we are connected to; $from_peers: those of them we download from.
subtitle-downloading =
    { $connected ->
        [0] Downloading
       *[other]
            Downloading · from { $from_peers } of { $connected } { $connected ->
                [one] peer
               *[other] peers
            }
    }
# $peers: peers we upload to.
subtitle-seeding =
    { $peers ->
        [0] Seeding
       *[other]
            Seeding · { $peers ->
                [one] { $peers } peer downloading from you
               *[other] { $peers } peers downloading from you
            }
    }
subtitle-paused = Paused
subtitle-queued-download = Queued to download
subtitle-queued-seed = Queued to seed
# $percent: whole percent (a number).
subtitle-checking = Checking files · { $percent }%
subtitle-fetching-metadata = Fetching metadata · { $percent }%
# $text: the server's own text, shown as sent. The -generic messages stand in when it sent none.
subtitle-error = Error: { $text }
subtitle-error-generic = Error: no details from the server
subtitle-tracker-warning = Tracker warning: { $text }
subtitle-tracker-warning-generic = Tracker warning: no details from the server
subtitle-tracker-error = Tracker error: { $text }
subtitle-tracker-error-generic = Tracker error: no details from the server

## The connection to the server (the status bar shows only a dot when connected)

# What the dot alone says: its accessible name and its tooltip.
connection-connected = Connected
connection-connecting = Connecting…
# $attempt: failed tries in a row (a number); $reason: one of the connection-reason-* messages. The
# status bar shows this next to the speeds, so the reasons are a few words each.
connection-reconnecting = Reconnecting (attempt { $attempt })… { $reason }
connection-reason-unreachable = unreachable
connection-reason-timeout = timed out
connection-reason-forbidden = access denied
connection-reason-unauthorized = login required
connection-reason-protocol = unexpected answer
connection-reason-server = server error
# The same reasons in full sentences, for a tooltip.
connection-reason-unreachable-detail = The server cannot be reached
connection-reason-timeout-detail = The server did not answer in time
connection-reason-forbidden-detail = The server does not accept requests from this computer
connection-reason-unauthorized-detail = The server rejected the credentials
connection-reason-protocol-detail = The server answered unexpectedly
connection-reason-server-detail = The server reported an error
connection-unsupported-legacy = Transmission 4.1 or newer required
connection-unsupported-no-version = Transmission 4.1 or newer required (server did not report a version)
# $version: the RPC version the server reports.
connection-unsupported-version = Transmission 4.1 or newer required (server RPC { $version })
connection-service-stopped = Service stopped
connection-service-failed = Service could not start

## The status bar

# $speed: the formatted total speed, such as "2.4 MiB/s".
status-download = ↓ { $speed }
status-upload = ↑ { $speed }
# $size: the formatted free space where new torrents are saved, such as "22.9 GiB".
status-free-space = { $size } free
# Shown when config.toml cannot be used; one of the config-problem-* messages is its tooltip.
status-config-problem = Config file not used
status-dismiss = Dismiss
# Shown while state.toml cannot be written, until a save works again; the detail is its tooltip.
status-state-not-saved = Layout not saved
status-state-not-saved-detail = The columns, sorting, filter, theme, window size and details panel cannot be saved to state.toml; the log says why

## The turtle and the speed limits, in the status bar

# The turtle button switches the alternative speed limits on and off: its name for screen readers,
# and its tooltip, which shows its key too. $down and $up: the alternative limits, such as "50 KiB/s".
alt-speed = Alternative speed limits
alt-speed-tooltip = Alternative speed limits: ↓ { $down } ↑ { $up }
# The button that opens the normal speed limits shows them. $down and $up: a limit such as
# "500 KiB/s", or limit-none while it is off.
speed-limits-button = ↓ { $down } · ↑ { $up }
limit-none = no limit
# The same button before the server has sent its settings; also its tooltip, which shows its key too.
speed-limits = Speed limits
# The popover the button opens: for each direction a switch that turns the limit on and a field for
# it, in KiB/s (unit-kib-per-s follows the field), and the button that applies both.
limit-down-switch = Limit download speed
limit-up-switch = Limit upload speed
limit-down-field = Download speed limit
limit-up-field = Upload speed limit
limit-apply = Apply
# Under a field, when its limit cannot be applied: the text is not a whole number; the number is out
# of range ($min: the lowest limit; $max: the highest, with its unit, such as "1,000,000 KiB/s");
# or the switch is on with no speed in the field.
limit-not-a-number = Enter a whole number of KiB/s
limit-out-of-range = Enter a speed from { $min } to { $max }
limit-needs-value = Enter a speed to turn the limit on

## Why config.toml is not used (the defaults are used instead)

config-problem-unreadable = config.toml cannot be read, so the defaults are used
# $line and $column: where the error is (numbers, from 1).
config-problem-invalid = config.toml has an error at line { $line }, column { $column }, so the defaults are used
config-problem-malformed = config.toml is not valid, so the defaults are used
config-problem-unsupported-url = The address in config.toml must start with http://, so the defaults are used

## The toolbar

# The theme switch names the current choice; each click moves on: system, light, dark.
theme-system = System theme
theme-light = Light theme
theme-dark = Dark theme
theme-switch-tooltip = Switch between the system, light and dark themes

# The toolbar buttons that start and pause the selected torrents: their names for screen readers,
# which their tooltips show too, with the key.
action-start = Start
action-pause = Pause
# The toolbar button whose menu removes the selected torrents: its name for screen readers, which
# its tooltip shows too, with the key; and the two items of its menu, each of which asks first.
action-remove = Remove
remove-from-list-menu = Remove from list…
remove-with-data-menu = Remove and delete files…
# The other items of the torrent list's context menu, besides action-start, action-pause and the
# two of the Remove menu. Start now starts ahead of the download queue; Verify checks the downloaded
# data; Reannounce asks the trackers for more peers now.
action-start-now = Start now
action-verify = Verify
action-reannounce = Reannounce
# Copy the magnet links or the info hashes of the selected torrents, one per line.
action-copy-magnet = Copy magnet link
action-copy-hash = Copy hash

## The dialogs that ask before torrents are removed

# Removing from the list keeps the downloaded files. $count: the torrents removed (a number).
remove-from-list-title =
    { $count ->
        [one] Remove { $count } torrent from the list?
       *[other] Remove { $count } torrents from the list?
    }
remove-from-list-keeps-files = The downloaded files stay on the server.
remove-from-list-confirm = Remove
# Removing with the data deletes the downloaded files from the server's disk.
# $count: the torrents removed (a number).
remove-with-data-title =
    { $count ->
        [one] Delete { $count } torrent and its files?
       *[other] Delete { $count } torrents and their files?
    }
# $size: how much downloaded data goes, such as "22.9 GiB".
remove-with-data-deletes-files = { $size } of downloaded data will be deleted from the server. This cannot be undone.
remove-with-data-confirm = Delete files
# Both dialogs list the names of the first five torrents, then how many more they remove.
# $more: the torrents not named (a number).
remove-more = and { $more } more
remove-cancel = Cancel

## Why an action failed, in a notification

# One message per action. $count: the torrents it named (a number), for the actions on torrents.
# $outcome is what is known:
#   not-sent: the server is not connected, so the action was not sent;
#   no-answer: the server did not answer, so the action may have happened;
#   failed: the server refused it or answered unexpectedly, and $reason says why: one of the
#     connection-reason-* messages, action-reason-nothing-selected or the server's own text.
action-failed-start =
    { $outcome ->
        [not-sent]
            { $count ->
                [one] Could not start the torrent: the server is not connected, so nothing was sent
               *[other] Could not start { $count } torrents: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $count ->
                [one] The server did not answer while starting the torrent; it may have started. The list will show the result
               *[other] The server did not answer while starting { $count } torrents; they may have started. The list will show the result
            }
       *[failed]
            { $count ->
                [one] Could not start the torrent: { $reason }
               *[other] Could not start { $count } torrents: { $reason }
            }
    }
action-failed-pause =
    { $outcome ->
        [not-sent]
            { $count ->
                [one] Could not pause the torrent: the server is not connected, so nothing was sent
               *[other] Could not pause { $count } torrents: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $count ->
                [one] The server did not answer while pausing the torrent; it may have been paused. The list will show the result
               *[other] The server did not answer while pausing { $count } torrents; they may have been paused. The list will show the result
            }
       *[failed]
            { $count ->
                [one] Could not pause the torrent: { $reason }
               *[other] Could not pause { $count } torrents: { $reason }
            }
    }
# Start now: start ahead of the download queue.
action-failed-start-now =
    { $outcome ->
        [not-sent]
            { $count ->
                [one] Could not start the torrent right away: the server is not connected, so nothing was sent
               *[other] Could not start { $count } torrents right away: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $count ->
                [one] The server did not answer while starting the torrent right away; it may have started. The list will show the result
               *[other] The server did not answer while starting { $count } torrents right away; they may have started. The list will show the result
            }
       *[failed]
            { $count ->
                [one] Could not start the torrent right away: { $reason }
               *[other] Could not start { $count } torrents right away: { $reason }
            }
    }
# Verify: check the downloaded data against the torrent.
action-failed-verify =
    { $outcome ->
        [not-sent]
            { $count ->
                [one] Could not verify the torrent: the server is not connected, so nothing was sent
               *[other] Could not verify { $count } torrents: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $count ->
                [one] The server did not answer while verifying the torrent; the check may have started. The list will show the result
               *[other] The server did not answer while verifying { $count } torrents; the checks may have started. The list will show the result
            }
       *[failed]
            { $count ->
                [one] Could not verify the torrent: { $reason }
               *[other] Could not verify { $count } torrents: { $reason }
            }
    }
# Reannounce: ask the trackers for more peers now.
action-failed-reannounce =
    { $outcome ->
        [not-sent]
            { $count ->
                [one] Could not ask the trackers of the torrent for more peers: the server is not connected, so nothing was sent
               *[other] Could not ask the trackers of { $count } torrents for more peers: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $count ->
                [one] The server did not answer while asking the trackers of the torrent for more peers; they may have been asked. The list will show the result
               *[other] The server did not answer while asking the trackers of { $count } torrents for more peers; they may have been asked. The list will show the result
            }
       *[failed]
            { $count ->
                [one] Could not ask the trackers of the torrent for more peers: { $reason }
               *[other] Could not ask the trackers of { $count } torrents for more peers: { $reason }
            }
    }
# Remove: take the torrents off the server's list and keep their downloaded files.
action-failed-remove =
    { $outcome ->
        [not-sent]
            { $count ->
                [one] Could not remove the torrent: the server is not connected, so nothing was sent
               *[other] Could not remove { $count } torrents: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $count ->
                [one] The server did not answer while removing the torrent; it may have been removed. The list will show the result
               *[other] The server did not answer while removing { $count } torrents; they may have been removed. The list will show the result
            }
       *[failed]
            { $count ->
                [one] Could not remove the torrent: { $reason }
               *[other] Could not remove { $count } torrents: { $reason }
            }
    }
# Remove with the data: take the torrents off the list and delete their files from the server.
action-failed-remove-with-data =
    { $outcome ->
        [not-sent]
            { $count ->
                [one] Could not remove the torrent and delete its files: the server is not connected, so nothing was sent
               *[other] Could not remove { $count } torrents and delete their files: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $count ->
                [one] The server did not answer while removing the torrent and deleting its files; it may have been removed and its files deleted. The list will show the result
               *[other] The server did not answer while removing { $count } torrents and deleting their files; they may have been removed and their files deleted. The list will show the result
            }
       *[failed]
            { $count ->
                [one] Could not remove the torrent and delete its files: { $reason }
               *[other] Could not remove { $count } torrents and delete their files: { $reason }
            }
    }
# Switching the alternative speed limits. $state: on or off, the way they were switched;
# $outcome and $reason as above.
action-failed-alt-speed =
    { $outcome ->
        [not-sent]
            { $state ->
                [on] Could not turn the alternative speed limits on: the server is not connected, so nothing was sent
               *[off] Could not turn the alternative speed limits off: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $state ->
                [on] The server did not answer while turning the alternative speed limits on; they may be on now. The status bar will show the result
               *[off] The server did not answer while turning the alternative speed limits off; they may be off now. The status bar will show the result
            }
       *[failed]
            { $state ->
                [on] Could not turn the alternative speed limits on: { $reason }
               *[off] Could not turn the alternative speed limits off: { $reason }
            }
    }
# Setting a normal speed limit, or turning it off. $direction: down or up; $outcome and $reason as
# above.
action-failed-speed-limit =
    { $outcome ->
        [not-sent]
            { $direction ->
                [down] Could not change the download speed limit: the server is not connected, so nothing was sent
               *[up] Could not change the upload speed limit: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $direction ->
                [down] The server did not answer while changing the download speed limit; it may have changed. The status bar will show the result
               *[up] The server did not answer while changing the upload speed limit; it may have changed. The status bar will show the result
            }
       *[failed]
            { $direction ->
                [down] Could not change the download speed limit: { $reason }
               *[up] Could not change the upload speed limit: { $reason }
            }
    }
# Choosing whether files of a torrent are downloaded, in the Files tab. $wanted: yes when they
# were to be downloaded, no when they were to be skipped; $outcome and $reason as above.
action-failed-files-wanted =
    { $outcome ->
        [not-sent]
            { $wanted ->
                [yes] Could not choose files to download: the server is not connected, so nothing was sent
               *[no] Could not skip files: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $wanted ->
                [yes] The server did not answer while choosing files to download; they may have been chosen. The Files tab will show the result
               *[no] The server did not answer while skipping files; they may have been skipped. The Files tab will show the result
            }
       *[failed]
            { $wanted ->
                [yes] Could not choose files to download: { $reason }
               *[no] Could not skip files: { $reason }
            }
    }
# Changing the priority of files of a torrent, in the Files tab; $outcome and $reason as above.
action-failed-file-priority =
    { $outcome ->
        [not-sent] Could not change the priority of files: the server is not connected, so nothing was sent
        [no-answer] The server did not answer while changing the priority of files; it may have changed. The Files tab will show the result
       *[failed] Could not change the priority of files: { $reason }
    }
# Switching the download in order of a torrent, in the Files tab. $state: on or off, the way it
# was switched; $outcome and $reason as above.
action-failed-sequential =
    { $outcome ->
        [not-sent]
            { $state ->
                [on] Could not turn on downloading in order: the server is not connected, so nothing was sent
               *[off] Could not turn off downloading in order: the server is not connected, so nothing was sent
            }
        [no-answer]
            { $state ->
                [on] The server did not answer while turning on downloading in order; it may be on now. The Files tab will show the result
               *[off] The server did not answer while turning off downloading in order; it may be off now. The Files tab will show the result
            }
       *[failed]
            { $state ->
                [on] Could not turn on downloading in order: { $reason }
               *[off] Could not turn off downloading in order: { $reason }
            }
    }
# An action whose kind is not known; $outcome and $reason as above.
action-failed-other =
    { $outcome ->
        [not-sent] An action could not be carried out: the server is not connected, so nothing was sent
        [no-answer] The server did not answer; the action may have happened. The list will show the result
       *[failed] An action failed: { $reason }
    }
# The reason when an action named no torrent, so it was not sent.
action-reason-nothing-selected = no torrent was selected
# Copying magnet links failed: they are read from the server first. $count: the torrents whose
# links were asked for (a number). $outcome is what is known:
#   not-sent: the server is not connected, so nothing was asked;
#   gone: the server answered, and has none of the torrents any more;
#   failed: $reason says why, as for the actions above.
copy-magnet-failed =
    { $outcome ->
        [not-sent]
            { $count ->
                [one] Could not copy the magnet link: the server is not connected
               *[other] Could not copy the magnet links of { $count } torrents: the server is not connected
            }
        [gone]
            { $count ->
                [one] Could not copy the magnet link: the torrent is no longer on the server
               *[other] Could not copy the magnet links of { $count } torrents: they are no longer on the server
            }
       *[failed]
            { $count ->
                [one] Could not copy the magnet link: { $reason }
               *[other] Could not copy the magnet links of { $count } torrents: { $reason }
            }
    }
# Some of the magnet links were copied: the server no longer has the other torrents.
# $copied: the links copied; $asked: the torrents whose links were asked for; $missing: the
# torrents left out (numbers).
copy-magnet-partial =
    Copied { $copied } of { $asked } magnet links; { $missing ->
        [one] { $missing } torrent is
       *[other] { $missing } torrents are
    } no longer on the server
# The magnet links came from the server while another window had the focus, so they were not
# copied: 4x4 writes the clipboard only while it is in focus. It names no key: the links can be
# copied from the context menu too, and the keys may change.
copy-magnet-not-in-focus = The magnet links are ready, but 4x4 was not in focus — copy them again

## The sidebar

# The filters of the torrent list. The sidebar shows each with the number of torrents it matches.
filter-all = All
filter-downloading = Downloading
filter-seeding = Seeding
filter-paused = Paused
filter-active = Active
filter-error = Error
# The number next to a filter. $count: the torrents it matches (a number).
filter-count = { $count }
# What a screen reader says for a filter once its number is known.
# $filter: one of the filter-* names; $count: the torrents it matches (a number).
filter-label =
    { $count ->
        [one] { $filter }, { $count } torrent
       *[other] { $filter }, { $count } torrents
    }

## The search box

# What the empty search box says: what it is for, and the key that moves the focus to it.
search-placeholder = Search (Ctrl+F)

## The torrent list

# $count: the torrents shown (a number); the accessible name of the list.
torrent-count =
    { $count ->
        [one] { $count } torrent
       *[other] { $count } torrents
    }
# What the empty list says: there is no torrent at all, or the filter and the search hide them all.
torrent-list-empty = No torrents
torrent-list-no-matches = No torrents match

# The column headers. The speed columns are narrow, so their headers are only arrows; the -long names
# are for the column menu and screen readers.
column-name = Name
column-size = Size
column-progress = Progress
column-state = State
column-down = ↓
column-down-long = Download speed
column-up = ↑
column-up-long = Upload speed
column-eta = ETA
column-ratio = Ratio
column-peers = Peers
# The mark after the header of the column the list is sorted by, and what it means to a screen reader.
sort-ascending = ▲
sort-descending = ▼
sort-ascending-label = sorted ascending
sort-descending-label = sorted descending
# The toolbar button that opens the list of columns to show or hide, and its tooltip.
column-menu = Columns
column-menu-tooltip = Show or hide columns
# The peers column. $count: the connected peers (a number).
peer-count = { $count }

## Dates

# A date and time in the local time zone, such as 2026-10-29 14:05. Each part comes as digits
# with its leading zeros: $year has four, $month, $day, $hour (0 to 23) and $minute two each.
date-time = { $year }-{ $month }-{ $day } { $hour }:{ $minute }

## The details panel under the table

# The tabs of the panel, which Alt+1 and Alt+2 choose too.
details-tab-general = General
details-tab-files = Files
# What the panel says when it shows no torrent: none is selected, or several are.
# $count: the torrents selected (a number, 2 or more).
details-none = Select a torrent
details-many =
    { $count ->
        [one] { $count } torrent selected
       *[other] { $count } torrents selected
    }
# What a value says while it is read from the server and has not come yet.
details-pending = —

# The labels of the General tab.
general-path = Path
general-size = Size
general-hash = Hash
general-pieces = Pieces
general-downloaded = Downloaded
general-uploaded = Uploaded
general-ratio = Ratio
general-added = Added
general-done = Completed
general-created = Created
general-source = Source
general-comment = Comment
general-private = Private
general-trackers = Trackers
# The label of the problem the server reports for the torrent: a local error, such as a full disk,
# or what the tracker said.
general-error-local = Error
general-error-tracker = Tracker error
general-warning-tracker = Tracker warning
# The value of the problem when the server sent no text for it.
general-error-no-text = no details from the server
# The buttons next to the path and the hash: their names for screen readers and their tooltips.
general-copy-path = Copy path
general-copy-hash = Copy hash

# The values of the General tab.
# A value the torrent does not have, such as an empty comment or an unknown date.
general-none = —
# The torrent has not finished downloading.
general-not-done = not yet
general-yes = Yes
general-no = No
# The size when only some of the files are wanted. $size: the size of all files; $wanted: the
# size of the wanted ones, both formatted, such as "22.9 GiB".
general-size-wanted = { $size } ({ $wanted } wanted)
# $count: the pieces (a number), for the plural; $pieces: the same number with its digits grouped,
# such as "1,500"; $size: the size of one piece, such as "4.0 MiB".
general-pieces-value =
    { $count ->
        [one] { $pieces } piece of { $size }
       *[other] { $pieces } pieces of { $size }
    }
# Who made the metainfo and when. $creator: the program's name, as the metainfo says it; $date: a
# date-time. When only the date is known, the value is the date alone.
general-created-by-on = by { $creator } on { $date }
general-created-by = by { $creator }
# How the trackers are doing, counted by what they answered: working, they answered their last
# announce; failing, it failed or was not answered in time; not contacted yet, no announce has
# gone to them, as for a torrent that is paused or queued, or a backup tracker. A count that is 0
# is left out. $count: a number.
general-trackers-working = { $count } working
general-trackers-failing = { $count } failing
general-trackers-not-contacted = { $count } not contacted yet
# Two or three of the counts above, in that order, such as "2 working, 1 failing".
general-trackers-two = { $first }, { $second }
general-trackers-three = { $first }, { $second }, { $third }
# A torrent without trackers.
general-trackers-none = none
# The worst problem among the trackers, under the line above. $host: the tracker's host name;
# $text: the server's own text, shown as sent, or general-error-no-text when it sent none.
# A failed announce: the tracker could not be reached, or refused, as $text says.
tracker-problem-announce-failed = Tracker { $host }: { $text }
# The tracker did not answer the announce in time.
tracker-problem-announce-timed-out = Tracker { $host }: connection timed out
# A failed scrape: the tracker sends no counts of seeders and leechers, as many trackers never
# do.
tracker-problem-scrape-failed = Tracker { $host } sends no peer counts: { $text }
tracker-problem-scrape-timed-out = Tracker { $host } sends no peer counts: connection timed out

## The Files tab of the details panel

# The switch that has the server download the torrent's pieces in order, from the start of each
# file, so that a file can be played while it downloads.
files-sequential = Download in order (sequential)
# What the tab says instead of the files of a magnet link, before its metadata has come.
files-no-metadata = The file list arrives with the torrent's metadata
# The name of the list of files and folders, for screen readers.
files-list = Files
# The name of the checkbox of a file or folder, for screen readers. $name: its name.
files-wanted = Download { $name }
# The priority of a file or folder, on its button and in its menu; a folder whose files have
# different priorities shows files-priority-mixed.
files-priority-low = Low
files-priority-normal = Normal
files-priority-high = High
files-priority-mixed = Mixed
# The name of the priority button of a file or folder, for screen readers. $name: its name;
# $priority: one of the files-priority-* values.
files-priority-label = Priority of { $name }: { $priority }
# The name of the progress bar of a file or folder, for screen readers. $name: its name.
files-progress-label = Progress of { $name }
# Progress in whole percent. $percent: a number.
files-percent = { $percent }%
# How much of a file that downloads can be played from its start: what has come without a gap.
# $size: the formatted size, such as "12.4 GiB"; $percent: the share of the file in whole percent
# (a number).
files-playable = Playable from start: { $size } ({ $percent }%)
