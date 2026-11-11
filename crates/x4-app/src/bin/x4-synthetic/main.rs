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
//! - `--add PATH_OR_MAGNET`, which may be given several times, asks the window to add each
//!   `.torrent` file or magnet link at the start, as launch arguments would, for screenshots of
//!   the add dialog. The arguments are told apart as launch arguments are, and the ones 4x4 does
//!   not open make their notification. Nothing is added until Add is pressed, and then only to
//!   the made-up daemon.
//! - `--right-click`, after `--select N` (and `--extend K`), then right-clicks the row `N` with
//!   the mouse, which opens its context menu, for screenshots of the menu.
//! - `--press KEY`, which may be given several times, presses each key in turn once the list is
//!   there (and selected), one every few seconds, as the user would on the keyboard: `secondary-p`
//!   (Ctrl, or ⌘ on macOS) pauses the selected torrents, `space` toggles them. The keys are in
//!   gpui's spelling, and each press is logged, for screenshots of what the actions do.
//! - `--connection` opens the Connection dialog after the first frame, as `Ctrl+,` would, for
//!   screenshots of the dialog.
//! - `--showcase` makes 24 torrents named like free software, films and texts that anyone may
//!   share, in a mix of states, instead of the 2000 "Synthetic torrent NNNN": the demo for the
//!   screenshots of the README.
//! - `--theme system|light|dark` starts the window with that theme, which the toolbar's button
//!   changes as ever, instead of `system`, which follows the desktop.
//!
//! The daemon carries out the actions on its own made-up torrents, so they can be tried here.
//! Its peers are at addresses reserved for documentation, which the demo's own country database,
//! made in memory, puts in countries. The window is set to a made-up server, and the Connection
//! dialog's Test and Save answer as if every server were the synthetic daemon: Save switches to a
//! new one ([`server`]). It reads and writes none of the app's files: there is no `config.toml`,
//! which Save would write, the UI state starts from the defaults and is never saved, the installed
//! `GeoIP` files are not opened, and the log goes to stderr only.

mod daemon;
mod frames;
mod server;

use std::env;
use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, App, AsyncApp, Entity, InputEvent as _, KeyUpEvent, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, point, px,
};
use x4_app::adding::{AddRequest, AddRequests, launch_requests};
use x4_app::allocator::{log_pin, pin_mmap_threshold};
use x4_app::app::{self, ServiceStart, Startup};
use x4_app::keymap::ExtendSelectionDown;
use x4_app::logging::init_stderr_logging;
use x4_app::main_view::MainView;
use x4_app::platform::host_os;
use x4_core::{CountryDb, Edition, GeoIp, ThemeChoice, UiState, build_country_file};

use crate::daemon::{Activity, Recipe};

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
    // First of all, as in `4x4`, so that the demo measures what the app does: no thread exists
    // yet, and mallopt is not thread-safe. Logged once logging is up.
    let mmap_pinned = pin_mmap_threshold();
    let started = Instant::now();
    // The demo writes no files, its log included.
    init_stderr_logging()?;
    log_pin(mmap_pinned);
    let options = Options::parse(env::args().skip(1))?;
    let recipe = options.recipe();
    tracing::info!(
        torrents = recipe.torrents(),
        showcase = options.showcase,
        theme = ?options.theme,
        activity = ?options.activity,
        scroll = options.scroll,
        select = ?options.select,
        extend = options.extend,
        right_click = options.right_click,
        press = ?options.press,
        adds = options.add.len(),
        connection = options.connection,
        "synthetic daemon"
    );
    let startup = Startup {
        service: ServiceStart::from(server::spawn(recipe)),
        connection: server::setup(recipe)?,
        ui: ui_state(options.theme),
        state_writable: false,
        state_path: None,
        config_problem: None,
        adds: AddRequests::new(),
    };
    let adds = startup.adds.sender.clone();
    let scroll = options.scroll;
    let cwd = env::current_dir()?;
    app::run_driven(started, startup, move |window, view, cx| {
        if !options.add.is_empty() {
            ask_to_add(&adds, &options.add, &cwd);
        }
        if options.connection {
            open_connection_after_the_first_frame(window, view.clone(), cx);
        }
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
/// would; right-clicks the row with `options.right_click`; then presses the keys of
/// `options.press`, one every [`PRESS_INTERVAL`].
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
        if options.right_click {
            // A frame or two first, so that the table has laid its rows out where they are now.
            cx.background_executor().timer(SELECT_POLL).await;
            right_click(window, &view, row, cx);
        }
        for keys in &options.press {
            cx.background_executor().timer(PRESS_INTERVAL).await;
            press(window, keys, cx);
        }
    })
    .detach();
}

/// Opens the Connection dialog over `view` once the first frame of `window` is drawn, as `Ctrl+,`
/// would then.
fn open_connection_after_the_first_frame(
    window: AnyWindowHandle,
    view: Entity<MainView>,
    cx: &mut App,
) {
    let asked = window.update(cx, |_, window, _| {
        // The first hop runs ahead of the first frame; the second once it has been presented.
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                tracing::info!("opening the Connection dialog");
                view.update(cx, |view, cx| view.open_connection(window, cx));
            });
        });
    });
    if let Err(error) = asked {
        tracing::error!(%error, "the Connection dialog could not be opened");
    }
}

/// Asks the main window, through its channel `adds`, to add `args`, with the requests the same
/// launch arguments run in `cwd` would make ([`launch_requests`]): the `.torrent` files and magnet
/// links in one request, the rest in another, which the window shows as one notification.
fn ask_to_add(adds: &async_channel::Sender<AddRequest>, args: &[String], cwd: &Path) {
    tracing::info!(arguments = args.len(), "asking to add torrents");
    for request in launch_requests(args, cwd) {
        if adds.try_send(request).is_err() {
            tracing::error!("the window stopped taking torrents to add");
        }
    }
}

/// Presses `keys` in `window`, as the keyboard would: the keystroke goes through the key bindings
/// to the focused table, and the key goes up again, or the window would take the next press of
/// the same key for a repeat of a held one.
fn press(window: AnyWindowHandle, keys: &str, cx: &mut AsyncApp) {
    let pressed = cx.update(|cx| {
        let keystroke = Keystroke::parse_with_os(keys, cx.platform_info().os)?;
        window.update(cx, |_, window, cx| {
            window.dispatch_keystroke(keystroke.clone(), cx);
            window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
        })
    });
    match pressed {
        Ok(()) => tracing::info!(keys, "key pressed"),
        Err(error) => tracing::error!(keys, %error, "the key could not be pressed"),
    }
}

/// Right-clicks the row `row` of the table of `view` in `window`, as the mouse would: the pointer
/// moves there, and the right button goes down and up again.
fn right_click(window: AnyWindowHandle, view: &Entity<MainView>, row: usize, cx: &mut AsyncApp) {
    let clicked = cx.update(|cx| {
        let Some(position) = row_position(view, row, cx) else {
            return Err(anyhow::anyhow!("the row is not laid out"));
        };
        tracing::debug!(?position, "right-clicking a row");
        window.update(cx, |_, window, cx| {
            let modifiers = Modifiers::none();
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Right,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Right,
                    position,
                    modifiers,
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        })
    });
    match clicked {
        Ok(()) => tracing::info!(row, "row right-clicked"),
        Err(error) => tracing::error!(row, %error, "the row could not be right-clicked"),
    }
}

/// A point in the row `row` of the table of `view`, in the window, near the left of the name: from
/// where the table drew its rows last, how tall they all are and how far they are scrolled. `None`
/// before the rows are drawn, or for a row index too large to place.
fn row_position(view: &Entity<MainView>, row: usize, cx: &App) -> Option<Point<Pixels>> {
    let table = view.read(cx).table().read(cx);
    let rows = f32::from(u16::try_from(table.delegate().rows().len()).ok()?);
    let row = f32::from(u16::try_from(row).ok()?);
    let scroll = table.vertical_scroll_handle.0.borrow();
    let bounds = scroll.base_handle.bounds();
    let content_height = scroll.base_handle.max_offset().y + bounds.size.height;
    if rows < 1. || content_height <= px(0.) {
        return None;
    }
    let row_height = content_height / rows;
    let offset = scroll.base_handle.offset();
    Some(point(
        bounds.origin.x + px(120.),
        bounds.origin.y + offset.y + row_height * (row + 0.5),
    ))
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

/// The demo's own country databases, made in memory: they put the documentation ranges the
/// made-up peers are in into countries, the region EU and none ([`daemon::country_blocks`]), so
/// that the Peers tab shows flags without the installed files, which know no such address.
fn countries() -> GeoIp {
    let blocks = daemon::country_blocks();
    let open = |edition| {
        let file = Cursor::new(build_country_file(edition, &blocks));
        match CountryDb::open(Box::new(file), edition) {
            Ok(db) => Some(db.named(format!("the synthetic {edition} countries"))),
            Err(error) => {
                tracing::error!(
                    %edition,
                    %error,
                    "the synthetic countries cannot be read; peers show none"
                );
                None
            }
        }
    };
    GeoIp::new(open(Edition::V4), open(Edition::V6))
}

/// The UI state the window starts from: the defaults, with `theme` when one is given. The demo
/// never saves it.
fn ui_state(theme: Option<ThemeChoice>) -> UiState {
    match theme {
        Some(theme) => UiState {
            theme,
            ..UiState::default()
        },
        None => UiState::default(),
    }
}

/// The command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is a command-line switch of its own, and none depends on another's value"
)]
struct Options {
    activity: Activity,
    scroll: bool,
    /// The index of the row to select.
    select: Option<usize>,
    /// How many rows to extend the selection down by, after selecting `select`.
    extend: usize,
    /// Whether to right-click the row `select` then.
    right_click: bool,
    /// The keys to press in turn, in gpui's spelling.
    press: Vec<String>,
    /// The `.torrent` files and magnet links to ask to add at the start.
    add: Vec<String>,
    /// Whether to open the Connection dialog after the first frame.
    connection: bool,
    /// Whether the daemon is the showcase of a few named torrents, not the default mix of many.
    showcase: bool,
    /// The theme to start with; `None` is the default, which follows the system.
    theme: Option<ThemeChoice>,
}

impl Options {
    /// The daemon these options ask for.
    fn recipe(&self) -> Recipe {
        Recipe {
            activity: self.activity,
            showcase: self.showcase,
        }
    }

    /// The options of `args`, the arguments after the program name.
    fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--static" => options.activity = Activity::Static,
                "--scroll" => options.scroll = true,
                "--right-click" => options.right_click = true,
                "--connection" => options.connection = true,
                "--showcase" => options.showcase = true,
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
                    let keys = args.next().ok_or_else(|| {
                        anyhow::anyhow!("--press needs a key, such as secondary-p")
                    })?;
                    Keystroke::parse_with_os(&keys, host_os()).map_err(|error| {
                        anyhow::anyhow!("--press needs a key, not {keys:?}: {error}")
                    })?;
                    options.press.push(keys);
                }
                "--add" => {
                    let item = args.next().ok_or_else(|| {
                        anyhow::anyhow!("--add needs a .torrent file or a magnet link")
                    })?;
                    options.add.push(item);
                }
                "--theme" => {
                    let choice = args.next().ok_or_else(|| {
                        anyhow::anyhow!("--theme needs one of system, light or dark")
                    })?;
                    options.theme = Some(match choice.as_str() {
                        "system" => ThemeChoice::System,
                        "light" => ThemeChoice::Light,
                        "dark" => ThemeChoice::Dark,
                        _ => anyhow::bail!(
                            "--theme needs one of system, light or dark, not {choice:?}"
                        ),
                    });
                }
                other => anyhow::bail!(
                    "unknown argument {other:?}; the options are --static, --scroll, --select N, \
                     --extend K, --right-click, --press KEY, --add PATH_OR_MAGNET, --connection, \
                     --showcase and --theme system|light|dark"
                ),
            }
        }
        if options.extend > 0 && options.select.is_none() {
            anyhow::bail!("--extend extends the selection of --select N; give --select too");
        }
        if options.right_click && options.select.is_none() {
            anyhow::bail!("--right-click right-clicks the row of --select N; give --select too");
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
                right_click: false,
                press: Vec::new(),
                add: Vec::new(),
                connection: false,
                showcase: false,
                theme: None,
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
                right_click: false,
                press: Vec::new(),
                add: Vec::new(),
                connection: false,
                showcase: false,
                theme: None,
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
    fn right_click_clicks_the_selected_row() {
        let options = parse(&["--select", "3", "--right-click"]).expect("a right-click");
        assert_eq!((options.select, options.right_click), (Some(3), true));
        let alone = parse(&["--right-click"]).expect_err("no row to click");
        assert!(alone.to_string().contains("--select"), "{alone}");
    }

    #[test]
    fn press_takes_keys_in_order() {
        let options = parse(&[
            "--select",
            "3",
            "--press",
            "secondary-p",
            "--press",
            "space",
        ])
        .expect("two keys");
        assert_eq!(options.press, ["secondary-p", "space"]);
        let missing = parse(&["--press"]).expect_err("no key");
        assert!(missing.to_string().contains("secondary-p"), "{missing}");
        let invalid = parse(&["--press", "p-ctrl"]).expect_err("the key comes last");
        assert!(invalid.to_string().contains("\"p-ctrl\""), "{invalid}");
    }

    #[test]
    fn add_takes_files_and_links_in_order() {
        let link = format!("magnet:?xt=urn:btih:{}", "a".repeat(40));
        let options =
            parse(&["--add", "show.torrent", "--add", &link, "--static"]).expect("two items");
        assert_eq!(options.add, ["show.torrent", link.as_str()]);
        let missing = parse(&["--add"]).expect_err("nothing to add");
        assert!(missing.to_string().contains("magnet link"), "{missing}");
    }

    #[test]
    fn connection_opens_the_connection_dialog() {
        assert!(!parse(&[]).expect("no flags").connection);
        let options = parse(&["--connection", "--static"]).expect("the dialog");
        assert!(options.connection);
        assert_eq!(options.activity, Activity::Static);
    }

    #[test]
    fn showcase_makes_the_daemon_a_showcase() {
        assert!(!parse(&[]).expect("no flags").showcase);
        let options = parse(&["--showcase", "--static"]).expect("the showcase");
        assert!(options.showcase);
        assert_eq!(options.activity, Activity::Static);
    }

    #[test]
    fn theme_takes_one_of_the_three_choices() {
        assert_eq!(parse(&[]).expect("no flags").theme, None);
        for (name, choice) in [
            ("system", ThemeChoice::System),
            ("light", ThemeChoice::Light),
            ("dark", ThemeChoice::Dark),
        ] {
            let options = parse(&["--theme", name, "--static"]).expect("a theme");
            assert_eq!(options.theme, Some(choice), "--theme {name}");
        }
    }

    #[test]
    fn theme_without_a_choice_or_with_another_names_the_choices() {
        let missing = parse(&["--theme"]).expect_err("no choice");
        let invalid = parse(&["--theme", "blue"]).expect_err("not a choice");
        for error in [&missing, &invalid] {
            let message = error.to_string();
            assert!(
                ["system", "light", "dark"]
                    .iter()
                    .all(|choice| message.contains(choice)),
                "{message}"
            );
        }
        assert!(invalid.to_string().contains("\"blue\""), "{invalid}");
    }

    #[test]
    fn a_theme_flag_sets_the_theme_of_the_ui_state_and_nothing_else() {
        assert_eq!(ui_state(None), UiState::default());
        assert_eq!(
            ui_state(Some(ThemeChoice::Dark)),
            UiState {
                theme: ThemeChoice::Dark,
                ..UiState::default()
            }
        );
    }

    #[test]
    fn the_flags_choose_the_daemon_to_make() {
        assert_eq!(
            parse(&[]).expect("no flags").recipe(),
            Recipe {
                activity: Activity::Ticking,
                showcase: false
            }
        );
        assert_eq!(
            parse(&["--showcase", "--static"])
                .expect("both flags")
                .recipe(),
            Recipe {
                activity: Activity::Static,
                showcase: true
            }
        );
    }

    #[test]
    fn an_unknown_argument_is_an_error() {
        let error = parse(&["--static", "--fast"]).expect_err("an unknown flag");
        let message = error.to_string();
        assert!(message.contains("\"--fast\""), "{message}");
        assert!(message.contains("--showcase"), "{message}");
        assert!(message.contains("--theme"), "{message}");
    }
}
