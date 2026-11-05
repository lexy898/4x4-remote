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

- The password of the daemon, kept as plain text in `config.toml` (readable by its owner only) and
  sent with every request over plain `http://` with basic authentication, which the network
  carries unencrypted; HTTPS is not supported yet (see [Limitations](README.md#limitations)).
- The parsing of `.torrent` files and magnet links, which come from files, the clipboard, the
  command line and other programs.
- The per-user socket and lock file of the single instance, through which a second start hands its
  files and links to the first.
