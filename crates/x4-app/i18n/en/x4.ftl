# The messages of 4x4, English; the fallback of every other language.
#
# Every language directory under i18n/ must define the same messages, with the same $variables
# (the parity test in src/i18n/parity.rs checks it). The app formats sizes, speeds and ratios
# itself and passes them as ready strings; counts are passed as numbers so that plural rules apply.

## Numbers

# The decimal separator of the language. The app formats numbers with a dot and swaps it for this.
number-decimal-separator = .

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
# Shown while the alternative speed limits apply; the detail is its tooltip.
status-alt-speed = Alt speed
status-alt-speed-detail = Alternative speed limits are on
# Shown when config.toml cannot be used; one of the config-problem-* messages is its tooltip.
status-config-problem = Config file not used
status-dismiss = Dismiss
# Shown while state.toml cannot be written, until a save works again; the detail is its tooltip.
status-state-not-saved = Layout not saved
status-state-not-saved-detail = The columns, sorting, filter, theme and window size cannot be saved to state.toml; the log says why

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

## Why an action on torrents failed, in a notification

# One message per action. $count: the torrents it named (a number). $outcome is what is known:
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
# An action whose kind is not known; $outcome and $reason as above.
action-failed-other =
    { $outcome ->
        [not-sent] An action could not be carried out: the server is not connected, so nothing was sent
        [no-answer] The server did not answer; the action may have happened. The list will show the result
       *[failed] An action failed: { $reason }
    }
# The reason when an action named no torrent, so it was not sent.
action-reason-nothing-selected = no torrent was selected

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
