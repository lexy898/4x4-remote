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
# The two largest non-zero units, such as "1 d 2 h"; or just the one, such as "5 m".
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

connection-connecting = Connecting…
# $attempt: failed tries in a row (a number); $reason: one of the connection-reason-* messages.
connection-reconnecting = Reconnecting (attempt { $attempt })… { $reason }
connection-reason-unreachable = The server cannot be reached
connection-reason-timeout = The server did not answer in time
connection-reason-forbidden = The server does not accept requests from this computer
connection-reason-unauthorized = The server rejected the credentials
connection-reason-protocol = The server answered unexpectedly
connection-reason-server = The server reported an error
connection-unsupported-legacy = Transmission 4.1 or newer required
connection-unsupported-no-version = Transmission 4.1 or newer required (the server reports no version)
# $version: the RPC version the server reports.
connection-unsupported-version = Transmission 4.1 or newer required (server RPC { $version })
connection-service-stopped = Service stopped
connection-service-failed = Service could not start
