# Security policy

## Supported versions

Only the latest release of 4x4 Remote gets security fixes.

## Reporting a vulnerability

Please do not open a public issue for a vulnerability. Report it privately: on the
[Security tab](https://github.com/lexy898/4x4-remote/security) of the repository, choose "Report a
vulnerability" (GitHub's private vulnerability reporting). Say which version it concerns (the
output of `4x4 --version`) and how to reproduce it. 4x4 is a one-person project, so a reply may
take a few days.

## What is in scope

- The handling of the password of the daemon: 4x4 keeps it in `config.toml`, readable by its owner
  only, and sends it with each request to the daemon. A leak of it anywhere else, such as the log,
  is a vulnerability.
- The parsing of `.torrent` files and magnet links, which come from files, the clipboard, the
  command line and other programs.
- The parsing of the daemon's RPC responses, which a daemon, or whatever stands between it and 4x4,
  controls.
- The per-user socket and lock file of the single instance, through which a second start hands its
  files and links to the first.

## What is out of scope

The documented plain `http://` connection (HTTPS is not supported yet) and the plain-text password
in `config.toml` are known limitations, not vulnerabilities; see
[Limitations](README.md#limitations).
