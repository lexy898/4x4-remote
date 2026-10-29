//! The main window of 4x4 over a synthetic daemon of 2000 torrents, for demos and measurements.
//! Nothing goes over the network. `just demo-synthetic` runs it.
//!
//! - With no flag, the rates of the active torrents change on every poll of the recently active
//!   torrents, as on a busy daemon.
//! - `--static` keeps every rate at 0 and no torrent recently active: an idle daemon, for
//!   measuring the CPU the window takes while nothing changes.
//! - `--scroll` waits 3 seconds, scrolls the table by one row per frame for 10 seconds, logs the
//!   intervals between the frames and quits. The summary is logged at info under the target
//!   `synthetic`, which `just demo-synthetic` shows.
//!
//! It reads and writes none of the app's files: there is no `config.toml`, the UI state starts
//! from the defaults and is never saved, and the log goes to stderr only.

mod daemon;
mod frames;

use std::env;
use std::time::{Duration, Instant};

use x4_app::app::{self, Startup};
use x4_app::logging::init_logging;
use x4_core::service;
use x4_core::{EngineConfig, UiState};
use x4_rpc::Client;

use crate::daemon::{Activity, SyntheticDaemon};

/// How many torrents the daemon has.
const TORRENTS: u32 = 2000;
/// How long `--scroll` waits before it scrolls, for the list to arrive and the window to settle.
const SCROLL_AFTER: Duration = Duration::from_secs(3);
/// How long `--scroll` scrolls.
const SCROLL_FOR: Duration = Duration::from_secs(10);

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    // No log directory: the log goes to stderr only.
    let _log_guard = init_logging(None)?;
    let options = Options::parse(env::args().skip(1))?;
    tracing::info!(
        torrents = TORRENTS,
        activity = ?options.activity,
        scroll = options.scroll,
        "synthetic daemon"
    );
    let client = Client::new(SyntheticDaemon::new(TORRENTS, options.activity));
    let startup = Startup {
        service: service::spawn(client, EngineConfig::default()),
        ui: UiState::default(),
        state_writable: false,
        state_path: None,
        config_problem: None,
    };
    app::run_driven(started, startup, move |window, view, cx| {
        if options.scroll {
            frames::scroll_then_quit(window, view, SCROLL_AFTER, SCROLL_FOR, cx);
        }
    })
}

/// The command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Options {
    activity: Activity,
    scroll: bool,
}

impl Options {
    /// The options of `args`, the arguments after the program name.
    fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut options = Self::default();
        for arg in args {
            match arg.as_str() {
                "--static" => options.activity = Activity::Static,
                "--scroll" => options.scroll = true,
                other => anyhow::bail!(
                    "unknown argument {other:?}; the options are --static and --scroll"
                ),
            }
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> anyhow::Result<Options> {
        Options::parse(args.iter().map(|&arg| arg.to_owned()))
    }

    #[test]
    fn without_flags_the_rates_tick_and_nothing_scrolls() {
        assert_eq!(
            parse(&[]).expect("no flags"),
            Options {
                activity: Activity::Ticking,
                scroll: false,
            }
        );
    }

    #[test]
    fn the_flags_make_the_daemon_static_and_the_table_scroll() {
        assert_eq!(
            parse(&["--scroll", "--static"]).expect("both flags"),
            Options {
                activity: Activity::Static,
                scroll: true,
            }
        );
    }

    #[test]
    fn an_unknown_argument_is_an_error() {
        let error = parse(&["--static", "--fast"]).expect_err("an unknown flag");
        assert!(error.to_string().contains("\"--fast\""), "{error}");
    }
}
