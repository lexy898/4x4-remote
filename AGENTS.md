# 4x4 Remote — Development Guide

4x4 Remote (4x4 for short) is a native desktop client for a remote Transmission daemon, written in
Rust on GPUI.
This file is the rulebook for every change. Product scope lives in the owner's local design notes.

## Tech Stack

- Rust, edition 2024, stable toolchain; MSRV is `rust-version` in the root `Cargo.toml`
- GPUI through `gpui-kit` (pinned with `=`); styled components via `gpui_kit::component`
- `just` (task runner), `cargo-nextest` (tests), `cargo-deny` and `cargo-machete` (dependencies)
- `rustfmt`, `taplo` (TOML formatting), `typos` (spelling)

## Commands

```bash
just check            # full gate: fmt, TOML, spelling, lint policy, clippy, tests, hook tests,
                      # doc tests, docs, deny, machete
just check-fast       # fmt + clippy + tests (run by the agent Stop hook)
just fmt              # format Rust and TOML
just test             # tests via nextest
just test-hooks       # tests for the hook and helper scripts in scripts/
just test-live        # live tests against the owner's daemon, and geoip_live, which reads the
                      # installed GeoIP files and needs no daemon; never part of check
just record-fixtures  # record and scrub test fixtures from the daemon (read-only calls)
just lint-policy      # lint exceptions, workspace lints, the one unsafe call, dependency
                      # direction, the vendored gpui-pre-wgpu patch, Markdown links
                      # (scripts/lint-policy.sh)
just run              # run the app (debug build)
just release          # optimized build
just measure          # startup time, memory and (--cpu <s>) CPU of the release build, with a temp
                      # state dir; --synthetic measures the demo, --binary <path> a built file;
                      # agents run it inside `just virtual` and measure the app with --offline (the
                      # session's config dir is empty, so the app would set no server, show the
                      # Connection dialog and poll nothing; --offline names a closed port, which
                      # it keeps polling; not with --synthetic: the demo reads no config, so it is
                      # refused there)
just package          # the Arch package of the committed master under ~/.cache/4x4-pkg; prints its
                      # path (installing it is the owner's). Agents run it only from a scratch
                      # clone, with XDG_CACHE_HOME under ~/.cache/x4-scratch, never in the owner's
                      # cache
just default-handler  # make the installed 4x4 the default app for .torrent files and magnet links
                      # (--undo gives them back); the owner's only: agents never run it
just virtual <cmd>    # run a command in a headless KWin with its own D-Bus, XDG and runtime dirs
                      # and HOME (cargo and rustup keep the owner's ~/.cargo and ~/.rustup)
just demo-synthetic   # the main window over 2000 made-up torrents, no network
                      # (--static, --scroll, --select N)
just update           # update dependencies, then run the full gate
just setup            # enable git hooks and verify required tools
```

## Definition of Done

- `just check` exits 0; cargo-deny duplicate-version warnings are expected.
- New behavior is covered by tests that fail without the change.
- `README.md` and `docs/user-guide.md` describe every user-visible change, and `CHANGELOG.md` lists
  it under the version being prepared.
- Hooks enforce this. `.claude/settings.json` formats each edited `.rs` file; a guard hook refuses
  Bash commands that bypass the gates (`git commit --no-verify` or `-n`, `git push --no-verify`,
  changing `core.hooksPath`, writing to `target/.agent-hooks`), commands that change the
  owner's default apps (`just default-handler`, `scripts/default-handler.sh`, `xdg-mime default`;
  see Hard Rules) and `sudo` or `doas` as a command word (see Hard Rules); and an agent may not
  finish while `just check-fast` is red. The Stop hook skips
  the check when nothing it depends on changed since the last green run, and gives up after 3
  blocked stops on the same state — then tell the owner that the checks are still red.
  `.githooks/pre-commit` runs `just pre-commit`. Never bypass them (`--no-verify`, disabling
  hooks, weakening hook scripts or lints to get green).
- After a toolchain upgrade brings new lints, fix them in a dedicated `chore:` commit.

## Architecture

```
crates/x4-rpc   Transmission RPC protocol and transport. No UI, no platform code.
crates/x4-core  Domain model and logic: torrent state, filters, sorting, polling worker, settings. No UI.
crates/x4-app   Binary `4x4`: GPUI views and platform integration (windows, clipboard, dialogs, IPC).
```

- Dependencies point one way: `x4-app → x4-core → x4-rpc`.
- `x4-rpc` and `x4-core` never depend on GPUI or OS-specific APIs. Platform code lives in `x4-app`
  or behind a trait that `x4-app` implements.
- Views are thin. Deciding what to show (state, order, counts, formatting) belongs in `x4-core`
  and is unit-tested there; `x4-app` renders it and forwards user intent.
- The UI thread never waits on the network; all RPC goes through the core worker.
- Long-lived references to torrents use `hash_string`, never the numeric `id`
  (ids change when the daemon restarts).
- Key bindings live only in `crates/x4-app/src/keymap.rs`.
- An open menu, popover or dialog keeps the keys: a key bound outside it must not act while it is
  open (the `NoAction` rows and their test).

## Code Style

The lints in `[workspace.lints]` are the baseline. This section covers what they cannot.

### Errors
- `x4-rpc` and `x4-core` define error enums with `thiserror`; variants carry context
  (what was attempted, with which input).
- `anyhow` is allowed only at the top level of `x4-app`.
- No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` outside tests. Handle the case or
  return an error; fallible indexing uses `.get()`.
- Never swallow an error: return it, or log it with `tracing` and show it to the user.

### Lint exceptions
- Only `#[expect(clippy::lint_name, reason = "why this case is fine")]` on the smallest possible item.
  `#[allow]` is rejected by lint. Workspace-wide exceptions need a comment in `Cargo.toml`.

### Unsafe code
- The workspace denies `unsafe_code`. The one exception, approved by the owner, is the `mallopt`
  call in `crates/x4-app/src/allocator.rs`: a `#[expect(unsafe_code, reason = "...")]` on that
  `let` statement and a `// SAFETY:` comment above it. Any other `unsafe` needs the owner's explicit
  approval first; `just lint-policy` rejects an `unsafe_code` exception anywhere else, and one in
  the allocator that does not sit on a `let` (on a whole function it would cover more calls). It
  also checks that `allocator.rs` holds exactly one `unsafe` keyword, and that the workspace keeps
  `multiple_unsafe_ops_per_block` at `warn` or `deny`, so that the one block holds one operation.

### Logging
- `tracing` only; `println!`/`eprintln!` are rejected by lint. English messages with structured
  fields: `tracing::info!(torrent = %hash, "torrent added")`.
- `error` — failed operation the user must know about; `warn` — recoverable (reconnect, retry);
  `info` — lifecycle (connect, add, remove); `debug` — per-request details; `trace` — payloads.

### Documentation
- Every public item has a doc comment; functions returning `Result` have an `# Errors` section;
  every crate root has a `//!` overview.
- Comments explain why, not what.

### Structure and naming
- One responsibility per module; prefer small files. No `mod.rs` (use `foo.rs` + `foo/`).
- Use `pub(crate)` unless an item is part of the crate's API.
- Newtypes for identifiers and units at API boundaries (`TorrentId`, `HashString`, `Bytes`,
  `BytesPerSecond`) instead of bare integers and strings.

### Tests
- Unit tests next to the code in `#[cfg(test)] mod tests`; cross-module tests in `crates/<crate>/tests/`.
- Test names state the behavior: `seeding_without_peers_is_idle`, not `test_state_2`.
- Default tests never touch the network. Server responses come from `tests/fixtures/`, recorded
  from a real daemon and scrubbed (no real names, paths, hashes, trackers or magnet links).
- Live tests against a real daemon are `#[ignore]`, take the URL from `X4_LIVE_URL` and run only
  via `just test-live`.

### UI text and localization
- The UI is English and localization-ready: every user-visible string goes through the i18n layer
  by key. The only allowed literal is the product name "4x4 Remote" (`PRODUCT_NAME`); the
  packages are `4x4-remote` and `4x4-remote-git`; the command, the config and log folders and the
  messages keep the short name, 4x4.
- To add a string: add the message to `crates/x4-app/i18n/en/x4.ftl` and fetch it with `t!` from
  `crates/x4-app/src/i18n.rs` (`t!("id")` or `t!("id", name = value)`). `t!` wraps `fl!`, which
  checks the message id against `en` at compile time. It checks the argument names only when the
  call passes at least one argument; a call with none is not checked, so a message that has
  variables is fetched only inside a helper of `i18n.rs`, and that helper has a test. Text for a
  `x4-core` value (size, speed, ETA, subtitle, state, connection) is such a helper, so views never
  pick words themselves.
- Never build sentences by concatenation; use message arguments and plural rules. Pass counts as
  numbers, so that plural selection works (every `{ $n -> }` needs a `*[other]` variant).
- Pass decimals (sizes, speeds, ratios) as preformatted strings made by `i18n::decimal`: Fluent does
  not format numbers by the rules of a language, and the decimal separator is the message
  `number-decimal-separator`.
- Another language is one more `crates/x4-app/i18n/<code>/x4.ftl`, and the language must also be
  selected in code (the loader loads only `en` until then; once it selects languages,
  `set_use_isolating(false)` must be applied again after each selection). The parity test in
  `src/i18n/parity.rs` covers every language directory: the file parses, defines the same messages
  as `en` and uses the same `$variables` in each. `fl!` checks only `en`, so keep that test green.
- Sizes use binary IEC units (KiB, MiB, GiB). `x4-core` scales them (`human_size`, `human_speed`);
  `x4-app/src/i18n.rs` turns them into text.

## Dependencies

- Declare every dependency in `[workspace.dependencies]` and use it with `workspace = true`.
- Prefer small, maintained crates; justify heavy ones in the commit message. No git dependencies.
- `gpui-kit` is pinned with `=`; bump it in a dedicated commit and run the app by hand. Follow
  `vendor/gpui-pre-wgpu/PATCHED.md` (`cargo update -p gpui-kit`), not `just update`: that updates
  everything and fails lint-policy partway through. The same commit re-vendors `gpui-pre-wgpu` and
  re-applies its patch; `just lint-policy` stays red until the vendored copy is the version
  `gpui-kit` asks for and is patched.
- `vendor/` holds third-party code used through `[patch.crates-io]`. Change it only by re-vendoring
  and patching as its `PATCHED.md` describes. Our formatting, lint and spelling rules do not apply
  to it, and the gates skip it.
- After changing dependencies run `just deny` and `just machete`. GPL, LGPL and AGPL licenses are
  not allowed; if one appears, stop and ask the owner.
- Never add advisory `ignore` entries, `skip`/`skip-tree` entries, license `allow` entries or
  `exceptions` in `deny.toml` without the owner's explicit approval; stop and ask.

## Library Documentation

- Look up library APIs with the context7 MCP server before coding against them; do not rely on memory.
- GPUI moves fast: check docs.rs/gpui-kit, gpui-kit.com and the sources of the pinned version in
  `~/.cargo/registry/src/`.
- Transmission RPC: `docs/rpc-spec.md` in the Transmission repository, 4.1.x. When the docs and the
  live server disagree, the server wins.

## Hard Rules

- Never delete the owner's downloads. `torrent_remove` with `delete_local_data: true` may target
  only torrents a test created in its own test directory.
- Never modify the owner's existing torrents (start, stop, verify, priorities, location) or daemon
  settings (`session_set`) from tests or while debugging. Reading is fine.
- The owner's daemon: `http://192.168.1.5:9091/transmission/rpc`, on a Raspberry Pi 3 with little
  RAM — keep the request rate modest. Server notes: `/home/alexey/garage/raspberrypi/CLAUDE.md`.
- Agents open app windows only through `just virtual …` (headless KWin, own D-Bus, own XDG and
  runtime dirs, own HOME); never run KWin, spectacle or test windows against the owner's session
  bus, XDG dirs, home or screens. Debug builds use the `.Devel` app id.
- Agents never run `sudo`, `pacman -U`, `makepkg -i`/`-s` or `just default-handler`; installing and
  changing the default apps is the owner's. The guard hook refuses `sudo` and `doas` (as command
  words, with any options), `just default-handler`, `scripts/default-handler.sh` and
  `xdg-mime default` (reading with `xdg-mime query default` is fine); tests run the script only in
  a throwaway desktop (see `test-hooks.sh`).
- Everything in the repository is English.

## Git

- Conventional Commits in English: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `ci:`, `build:`.
- No AI attribution of any kind in commits, PRs or other git metadata.
- `.specs/` is local working material and is never committed.
- The repository is public: commits and merge commits carry the owner's GitHub noreply address,
  never a private one; this checkout sets `user.email` to it. `gh pr merge` would sign the merge
  with the account's private address (and refuses the noreply one), so merge a green PR here:
  `git merge --no-ff origin/<branch> -m "Merge pull request #<n> from lexy898/<branch>"`, then
  push `master`; GitHub marks the PR merged.
- One branch per stage or feature, merged through a PR once CI is green.
- Work in place in this checkout on a branch per stage/feature; do not create git worktrees — the
  Stop hook and the build cache live in this directory.
