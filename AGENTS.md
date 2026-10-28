# 4x4 — Development Guide

4x4 is a native desktop client for a remote Transmission daemon, written in Rust on GPUI.
This file is the rulebook for every change. Product scope lives in the owner's local design notes.

## Tech Stack

- Rust, edition 2024, stable toolchain; MSRV is `rust-version` in the root `Cargo.toml`
- GPUI through `gpui-kit` (pinned with `=`); styled components via `gpui_kit::component`
- `just` (task runner), `cargo-nextest` (tests), `cargo-deny` and `cargo-machete` (dependencies)
- `rustfmt`, `taplo` (TOML formatting), `typos` (spelling)

## Commands

```bash
just check        # full gate: fmt, TOML, spelling, clippy, tests, doc tests, docs, deny, machete
just check-fast   # fmt + clippy + tests (run by the agent Stop hook)
just fmt          # format Rust and TOML
just test         # tests via nextest
just run          # run the app (debug build)
just release      # optimized build
just measure      # startup time and memory of the release build
just update       # update dependencies, then run the full gate
just setup        # enable git hooks and verify required tools
```

## Definition of Done

- `just check` passes with zero warnings.
- New behavior is covered by tests that fail without the change.
- `README.md` reflects every user-visible change.
- Hooks enforce this: `.claude/settings.json` formats each edited `.rs` file and runs `just check-fast`
  before an agent may finish; `.githooks/pre-commit` runs `just pre-commit`. Never bypass them
  (`--no-verify`, disabling hooks, weakening hook scripts or lints to get green).

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
  by key (introduced in stage 2; until then the only allowed literal is the product name "4x4").
- Never build sentences by concatenation; use message arguments and plural rules.
- Sizes use binary IEC units (KiB, MiB, GiB) and are formatted in `x4-core`.

## Dependencies

- Declare every dependency in `[workspace.dependencies]` and use it with `workspace = true`.
- Prefer small, maintained crates; justify heavy ones in the commit message. No git dependencies.
- `gpui-kit` is pinned with `=`; bump it in a dedicated commit with `just update` and a manual app run.
- After changing dependencies run `just deny` and `just machete`. GPL, LGPL and AGPL licenses are
  not allowed; if one appears, stop and ask the owner.

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
- Everything in the repository is English.

## Git

- Conventional Commits in English: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `ci:`, `build:`.
- No AI attribution of any kind in commits, PRs or other git metadata.
- `.specs/` is local working material and is never committed.
- One branch per stage or feature, merged through a PR once CI is green.
