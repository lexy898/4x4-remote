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
//! - `--extend K`, after `--select N`, then extends the selection `K` rows down, as Shift+Down
//!   pressed `K` times would, for screenshots of several selected rows.
//! - `--press KEY`, which may be given several times, presses each key in turn once the list is
//!   there (and selected), one every few seconds, as the user would on the keyboard: `ctrl-p`
//!   pauses the selected torrents, `space` toggles them. The keys are in gpui's spelling, and each
//!   press is logged, for screenshots of what the actions do.
//!
//! The daemon carries out the actions on its own made-up torrents, so they can be tried here.
//! It reads and writes none of the app's files: there is no `config.toml`, the UI state starts
//! from the defaults and is never saved, and the log goes to stderr only.

mod daemon;
mod frames;

use std::env;
use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, App, AsyncApp, Entity, Keystroke, OperatingSystem};
use x4_app::app::{self, Startup};
use x4_app::keymap::ExtendSelectionDown;
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
/// How long `--press` waits before each key, for the window to show what the one before did.
const PRESS_INTERVAL: Duration = Duration::from_secs(3);

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
        extend = options.extend,
        press = ?options.press,
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
    let scroll = options.scroll;
    app::run_driven(started, startup, move |window, view, cx| {
        if options.select.is_some() || !options.press.is_empty() {
            drive_when_listed(window, view.clone(), options, cx);
        }
        if scroll {
            frames::scroll_then_quit(window, view, SCROLL_AFTER, SCROLL_FOR, cx);
        }
    })
}

/// Once the table of `view` in `window` has its rows: selects the row `options.select`, as a
/// click on the row would, and extends the selection `options.extend` rows down, as Shift+Down
/// would; then presses the keys of `options.press`, one every [`PRESS_INTERVAL`].
fn drive_when_listed(
    window: AnyWindowHandle,
    view: Entity<MainView>,
    options: Options,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let row = options.select.unwrap_or(0);
        let deadline = Instant::now() + SELECT_WAIT;
        loop {
            let listed = cx.update(|cx| {
                let table = view.read(cx).table().clone();
                let listed = row < table.read(cx).delegate().rows().len();
                if listed && options.select.is_some() {
                    table.update(cx, |table, cx| table.set_selected_row(row, cx));
                }
                listed
            });
            if listed {
                break;
            }
            if Instant::now() >= deadline {
                tracing::error!(row, "the table never had the rows to select and act on");
                return;
            }
            cx.background_executor().timer(SELECT_POLL).await;
        }
        if options.select.is_some() {
            tracing::info!(row, "row selected");
            extend_down(window, options.extend, cx);
        }
        for keys in &options.press {
            cx.background_executor().timer(PRESS_INTERVAL).await;
            press(window, keys, cx);
        }
    })
    .detach();
}

/// Presses `keys` in `window`, as the keyboard would: the keystroke goes through the key bindings
/// to the focused table.
fn press(window: AnyWindowHandle, keys: &str, cx: &mut AsyncApp) {
    let pressed = cx.update(|cx| {
        let keystroke = Keystroke::parse_with_os(keys, cx.platform_info().os)?;
        window.update(cx, |_, window, cx| {
            window.dispatch_keystroke(keystroke, cx);
        })
    });
    match pressed {
        Ok(()) => tracing::info!(keys, "key pressed"),
        Err(error) => tracing::error!(keys, %error, "the key could not be pressed"),
    }
}

/// Presses Shift+Down `times` times in `window`: the action of the key goes to the focused
/// table, as a key press would send it.
fn extend_down(window: AnyWindowHandle, times: usize, cx: &mut AsyncApp) {
    if times == 0 {
        return;
    }
    let extended = cx.update(|cx| {
        window.update(cx, |_, window, cx| {
            for _ in 0..times {
                window.dispatch_action(Box::new(ExtendSelectionDown), cx);
            }
        })
    });
    match extended {
        Ok(()) => tracing::info!(rows = times, "selection extended"),
        Err(error) => tracing::error!(%error, "the selection could not be extended"),
    }
}

/// The command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Options {
    activity: Activity,
    scroll: bool,
    /// The index of the row to select.
    select: Option<usize>,
    /// How many rows to extend the selection down by, after selecting `select`.
    extend: usize,
    /// The keys to press in turn, in gpui's spelling.
    press: Vec<String>,
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
                "--extend" => {
                    let rows = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--extend needs a number of rows"))?;
                    options.extend = rows.parse().map_err(|error| {
                        anyhow::anyhow!("--extend needs a number of rows, not {rows:?}: {error}")
                    })?;
                }
                "--press" => {
                    let keys = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--press needs a key, such as ctrl-p"))?;
                    // The demo runs on Linux, the only platform the app supports.
                    Keystroke::parse_with_os(&keys, OperatingSystem::Linux).map_err(|error| {
                        anyhow::anyhow!("--press needs a key, not {keys:?}: {error}")
                    })?;
                    options.press.push(keys);
                }
                other => anyhow::bail!(
                    "unknown argument {other:?}; the options are --static, --scroll, --select N, \
                     --extend K and --press KEY"
                ),
            }
        }
        if options.extend > 0 && options.select.is_none() {
            anyhow::bail!("--extend extends the selection of --select N; give --select too");
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
                extend: 0,
                press: Vec::new(),
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
                extend: 0,
                press: Vec::new(),
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
    fn extend_takes_a_number_of_rows_after_select() {
        let options = parse(&["--select", "4", "--extend", "2"]).expect("a range");
        assert_eq!((options.select, options.extend), (Some(4), 2));
        let invalid = parse(&["--select", "4", "--extend", "-1"]).expect_err("not a count");
        assert!(invalid.to_string().contains("\"-1\""), "{invalid}");
        let alone = parse(&["--extend", "2"]).expect_err("nothing to extend");
        assert!(alone.to_string().contains("--select"), "{alone}");
    }

    #[test]
    fn press_takes_keys_in_order() {
        let options =
            parse(&["--select", "3", "--press", "ctrl-p", "--press", "space"]).expect("two keys");
        assert_eq!(options.press, ["ctrl-p", "space"]);
        let missing = parse(&["--press"]).expect_err("no key");
        assert!(missing.to_string().contains("ctrl-p"), "{missing}");
        let invalid = parse(&["--press", "p-ctrl"]).expect_err("the key comes last");
        assert!(invalid.to_string().contains("\"p-ctrl\""), "{invalid}");
    }

    #[test]
    fn an_unknown_argument_is_an_error() {
        let error = parse(&["--static", "--fast"]).expect_err("an unknown flag");
        assert!(error.to_string().contains("\"--fast\""), "{error}");
    }
}
