//! The main window of 4x4 over a synthetic daemon of 2000 torrents, for demos and measurements.
//! Nothing goes over the network. `just demo-synthetic` runs it.
//!
//! A binary of its own behind the `synthetic` feature, so that `4x4` is built without it, and so
//! that it is built as `4x4` is: an example would get the features of the dev-dependencies, test
//! support included, and measure more than the app.
//!
//! - With no flag, the rates of the active torrents change on every poll of the recently active
//!   torrents, as on a busy daemon.
//! - `--static` keeps every rate at 0 and no torrent recently active: an idle daemon, for
//!   measuring the CPU the window takes while nothing changes.
//! - `--scroll` waits 3 seconds, scrolls the table by one row per frame for 10 seconds, logs the
//!   intervals between the frames and quits. The summary is logged at info under the target
//!   `x4_synthetic`, which `just demo-synthetic` shows.
//! - `--select N` selects the row at index `N` (from 0, in the shown order) once the list is
//!   there, as a click would, for screenshots of a selected row.
//!
//! It reads and writes none of the app's files: there is no `config.toml`, the UI state starts
//! from the defaults and is never saved, and the log goes to stderr only.

mod daemon;
mod frames;

use std::env;
use std::time::{Duration, Instant};

use gpui_kit::{App, Entity};
use x4_app::app::{self, Startup};
use x4_app::logging::init_stderr_logging;
use x4_app::main_view::MainView;
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
/// How often `--select` looks whether the list is there.
const SELECT_POLL: Duration = Duration::from_millis(100);
/// How long `--select` waits for the list before it gives up.
const SELECT_WAIT: Duration = Duration::from_secs(10);

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    // The demo writes no files, its log included.
    init_stderr_logging()?;
    let options = Options::parse(env::args().skip(1))?;
    tracing::info!(
        torrents = TORRENTS,
        activity = ?options.activity,
        scroll = options.scroll,
        select = ?options.select,
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
        if let Some(row) = options.select {
            select_when_listed(view.clone(), row, cx);
        }
        if options.scroll {
            frames::scroll_then_quit(window, view, SCROLL_AFTER, SCROLL_FOR, cx);
        }
    })
}

/// Selects the row at index `row` of the table of `view` once the table has it, as a click on
/// the row would.
fn select_when_listed(view: Entity<MainView>, row: usize, cx: &mut App) {
    cx.spawn(async move |cx| {
        let deadline = Instant::now() + SELECT_WAIT;
        while Instant::now() < deadline {
            let selected = cx.update(|cx| {
                let table = view.read(cx).table().clone();
                let listed = row < table.read(cx).delegate().rows().len();
                if listed {
                    table.update(cx, |table, cx| table.set_selected_row(row, cx));
                }
                listed
            });
            if selected {
                tracing::info!(row, "row selected");
                return;
            }
            cx.background_executor().timer(SELECT_POLL).await;
        }
        tracing::error!(row, "the table never had the row to select");
    })
    .detach();
}

/// The command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Options {
    activity: Activity,
    scroll: bool,
    /// The index of the row to select.
    select: Option<usize>,
}

impl Options {
    /// The options of `args`, the arguments after the program name.
    fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--static" => options.activity = Activity::Static,
                "--scroll" => options.scroll = true,
                "--select" => {
                    let row = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--select needs a row index"))?;
                    let row = row.parse().map_err(|error| {
                        anyhow::anyhow!("--select needs a row index, not {row:?}: {error}")
                    })?;
                    options.select = Some(row);
                }
                other => anyhow::bail!(
                    "unknown argument {other:?}; the options are --static, --scroll and --select N"
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
                select: None,
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
                select: None,
            }
        );
    }

    #[test]
    fn select_takes_the_index_of_a_row() {
        assert_eq!(
            parse(&["--select", "4", "--static"]).expect("a row").select,
            Some(4)
        );
    }

    #[test]
    fn select_without_a_row_index_is_an_error() {
        let missing = parse(&["--select"]).expect_err("no index");
        assert!(missing.to_string().contains("row index"), "{missing}");
        let invalid = parse(&["--select", "first"]).expect_err("not a number");
        assert!(invalid.to_string().contains("\"first\""), "{invalid}");
    }

    #[test]
    fn an_unknown_argument_is_an_error() {
        let error = parse(&["--static", "--fast"]).expect_err("an unknown flag");
        assert!(error.to_string().contains("\"--fast\""), "{error}");
    }
}
