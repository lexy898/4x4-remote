//! Headless tests for the UI state the main window remembers between runs: what it restores from
//! `state.toml`, when it saves (a second after the last change, and at the close or the quit),
//! what it never saves over, and how a failed save shows.
//!
//! Each test keeps `state.toml` in a temporary directory and opens the window the way the app
//! does, from a [`Startup`], with no service behind it, so nothing touches the network. Time is
//! the test executor's: `advance` moves its clock, and nothing sleeps.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, Role, TestAppContext, VisualTestContext, px,
    size,
};
use tempfile::TempDir;
use x4_app::app::{self, Startup};
use x4_app::main_view::MainView;
use x4_core::{
    ColumnId, ColumnState, Filter, SortDirection, SortKey, SortState, ThemeChoice, UiState,
    ViewState, WindowState,
};

/// The position of the size column in the default layout.
const SIZE: usize = 1;

/// A state file that changes every remembered value from its default.
const SAVED: &str = r#"
version = 1
filter = "seeding"
theme = "dark"

[window]
width = 1000.0
height = 650.0
maximized = false

[sort]
key = "size"
direction = "descending"

[[columns]]
id = "name"
visible = true
width = 420.0

[[columns]]
id = "peers"
visible = true
width = 70.0

[[columns]]
id = "size"
visible = true
width = 90.0

[[columns]]
id = "progress"
visible = true
width = 140.0

[[columns]]
id = "state"
visible = true
width = 90.0

[[columns]]
id = "down"
visible = true
width = 85.0

[[columns]]
id = "up"
visible = true
width = 85.0

[[columns]]
id = "eta"
visible = false
width = 65.0

[[columns]]
id = "ratio"
visible = true
width = 60.0
"#;

/// An open main window that saves its state to `path`.
struct Window {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    path: PathBuf,
}

impl Window {
    /// Starts as the app does with the state file at `path`, with no service, and opens the
    /// main window.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that does not open fails the test"
    )]
    fn open(path: &Path, cx: &mut TestAppContext) -> Self {
        let startup = Startup::from_state_file(
            Err(io::Error::other("the tests run no service")),
            Some(path.to_path_buf()),
        );
        let (handle, view) = cx
            .update(|cx| {
                app::init(cx);
                app::launch(startup, cx)
            })
            .expect("the main window opens");
        cx.run_until_parked();
        Self {
            handle,
            view,
            path: path.to_path_buf(),
        }
    }

    /// Runs `act` on the window, after drawing a frame.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a closed window fails the test"
    )]
    fn act<R>(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut gpui_kit::Window, &mut App) -> R,
    ) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                act(window, cx)
            })
            .expect("the window is open");
        cx.run_until_parked();
        result
    }

    /// Clicks the header of the size column, which sorts by size or reverses the order.
    fn click_size_header(&self, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.click(("col-header", SIZE), cx));
    }

    /// Asks the window to close, as its close button does, and returns whether it may.
    fn close(&self, cx: &mut TestAppContext) -> bool {
        VisualTestContext::from_window(self.handle, cx).simulate_close()
    }

    fn ui_state(&self, cx: &mut TestAppContext) -> UiState {
        cx.update(|cx| self.view.read(cx).ui_state().clone())
    }

    fn saves(&self, cx: &mut TestAppContext) -> u64 {
        cx.update(|cx| self.view.read(cx).state_saves())
    }

    /// The state in the file, or `None` if there is no file.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a file that is not a valid state fails the test"
    )]
    fn saved(&self) -> Option<UiState> {
        let text = fs::read_to_string(&self.path).ok()?;
        Some(UiState::parse(&text).expect("a valid state file").state)
    }

    /// The label of the status bar's notice that the state cannot be saved, if it is shown.
    fn not_saved_notice(&self, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| {
            window
                .try_find("state-not-saved")
                .and_then(|notice| notice.label().map(str::to_owned))
        })
    }
}

/// Moves the test clock on by `millis` and runs what came due.
fn advance(millis: u64, cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.run_until_parked();
}

#[expect(
    clippy::expect_used,
    reason = "a test helper: no temporary directory fails the test"
)]
fn temp_dir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The names in `dir`, sorted.
#[expect(
    clippy::expect_used,
    reason = "a test helper: an unreadable directory fails the test"
)]
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("a readable directory")
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn sorted(key: SortKey, direction: SortDirection) -> SortState {
    SortState { key, direction }
}

/// The accessible names of the column headers, in the order shown.
fn header_labels(window: &gpui_kit::Window) -> Vec<String> {
    let mut headers = window.find_all_by_role(Role::ColumnHeader);
    headers.sort_by(|a, b| f32::from(a.bounds().left()).total_cmp(&f32::from(b.bounds().left())));
    headers
        .iter()
        .filter_map(|header| header.label().map(str::to_owned))
        .collect()
}

#[gpui_kit::test]
fn state_restores_columns_sort_filter(cx: &mut TestAppContext) {
    let dir = temp_dir();
    let path = dir.path().join("state.toml");
    fs::write(&path, SAVED).expect("written");
    let column = |id, visible, width| ColumnState { id, visible, width };
    let columns = vec![
        column(ColumnId::Name, true, 420.),
        column(ColumnId::Peers, true, 70.),
        column(ColumnId::Size, true, 90.),
        column(ColumnId::Progress, true, 140.),
        column(ColumnId::State, true, 90.),
        column(ColumnId::Down, true, 85.),
        column(ColumnId::Up, true, 85.),
        column(ColumnId::Eta, false, 65.),
        column(ColumnId::Ratio, true, 60.),
    ];

    let window = Window::open(&path, cx);

    let ui = window.ui_state(cx);
    assert_eq!(ui.columns, columns);
    assert_eq!(ui.sort, sorted(SortKey::Size, SortDirection::Descending));
    assert_eq!(ui.filter, Filter::Seeding);
    assert_eq!(ui.theme, ThemeChoice::Dark);
    assert_eq!(
        ui.window,
        WindowState {
            width: 1000.,
            height: 650.,
            maximized: false
        }
    );
    let (view_state, table_columns, table_sort, name_width, mode) = cx.update(|cx| {
        let view = window.view.read(cx);
        let table = view.table().read(cx);
        (
            view.view_state().clone(),
            table.delegate().columns().to_vec(),
            table.delegate().sort(),
            table.col_width(0),
            cx.theme().mode,
        )
    });
    assert_eq!(
        view_state,
        ViewState {
            filter: Filter::Seeding,
            search: String::new(),
            sort_key: SortKey::Size,
            sort_direction: SortDirection::Descending,
        }
    );
    assert_eq!(table_columns, columns, "the table has the saved columns");
    assert_eq!(table_sort, (SortKey::Size, SortDirection::Descending));
    assert_eq!(name_width, Some(px(420.)));
    assert_eq!(mode, ThemeMode::Dark);
    window.act(cx, |window, _| {
        assert_eq!(window.bounds().size, size(px(1000.), px(650.)));
        let headers = header_labels(window);
        assert_eq!(
            headers.get(..3),
            Some(&["Name", "Peers", "Size"].map(str::to_owned)[..])
        );
        assert!(!headers.iter().any(|header| header == "ETA"), "{headers:?}");
        let mark = window.within(("col-header", 2usize)).find("sort-mark");
        assert_eq!(mark.label(), Some("sorted descending"));
    });

    // Opening changes nothing, so it saves nothing.
    advance(5_000, cx);
    assert_eq!(window.saves(cx), 0);
    assert_eq!(fs::read_to_string(&path).expect("readable"), SAVED);
}

#[gpui_kit::test]
fn changes_are_saved_debounced(cx: &mut TestAppContext) {
    let dir = temp_dir();
    let window = Window::open(&dir.path().join("state.toml"), cx);

    window.click_size_header(cx);
    advance(400, cx);
    window.click_size_header(cx);
    advance(400, cx);
    window.click_size_header(cx);
    assert_eq!(
        window.ui_state(cx).sort,
        sorted(SortKey::Size, SortDirection::Descending),
        "the header click is copied into the state"
    );

    // A second after the first click, but not after the last one.
    advance(900, cx);
    assert_eq!(window.saves(cx), 0);
    assert_eq!(window.saved(), None);

    advance(100, cx);
    assert_eq!(window.saves(cx), 1, "one write for the three clicks");
    let saved = window.saved().expect("the state is saved");
    assert_eq!(saved.sort, sorted(SortKey::Size, SortDirection::Descending));
    assert_eq!(saved, window.ui_state(cx));

    advance(5_000, cx);
    assert_eq!(window.saves(cx), 1, "nothing more to save");
    assert_eq!(
        names(dir.path()),
        ["state.toml"],
        "no temporary file is left"
    );
}

#[gpui_kit::test]
fn newer_state_file_is_not_overwritten(cx: &mut TestAppContext) {
    let dir = temp_dir();
    let path = dir.path().join("state.toml");
    let newer = "version = 99\nlayout = \"from the future\"\n\n[sort]\nkey = \"name\"\n";
    fs::write(&path, newer).expect("written");
    let window = Window::open(&path, cx);
    assert_eq!(
        window.ui_state(cx),
        UiState::default(),
        "the defaults are used"
    );

    window.click_size_header(cx);
    window.act(cx, |window, cx| window.click("theme-toggle", cx));
    cx.simulate_window_resize(window.handle, size(px(1000.), px(700.)));
    advance(5_000, cx);
    assert!(window.close(cx));

    assert_eq!(window.saves(cx), 0);
    assert_eq!(fs::read_to_string(&path).expect("readable"), newer);
    assert_eq!(names(dir.path()), ["state.toml"]);
}

#[gpui_kit::test]
fn typing_in_the_search_box_never_saves(cx: &mut TestAppContext) {
    let dir = temp_dir();
    let window = Window::open(&dir.path().join("state.toml"), cx);

    window.act(cx, |window, cx| window.press("ctrl-f", cx));
    for key in ["u", "b", "u"] {
        window.act(cx, |window, cx| window.input(key, cx));
        advance(400, cx);
    }
    assert_eq!(
        cx.update(|cx| window.view.read(cx).view_state().search.clone()),
        "ubu"
    );
    advance(5_000, cx);
    assert_eq!(window.saves(cx), 0);
    assert_eq!(window.saved(), None);

    // Typing does not hold back the save of a real change either.
    window.click_size_header(cx);
    advance(500, cx);
    window.act(cx, |window, cx| window.press("ctrl-f", cx));
    window.act(cx, |window, cx| window.input("n", cx));
    advance(500, cx);
    assert_eq!(window.saves(cx), 1);
    assert_eq!(
        window.saved().map(|saved| saved.sort),
        Some(sorted(SortKey::Size, SortDirection::Descending))
    );
}

#[gpui_kit::test]
fn closing_the_window_saves_a_pending_change(cx: &mut TestAppContext) {
    let dir = temp_dir();
    let window = Window::open(&dir.path().join("state.toml"), cx);

    window.click_size_header(cx);
    assert_eq!(window.saved(), None, "the second has not passed");
    assert!(window.close(cx), "saving never keeps the window open");

    assert_eq!(window.saves(cx), 1);
    assert_eq!(
        window.saved().map(|saved| saved.sort),
        Some(sorted(SortKey::Size, SortDirection::Descending))
    );
    advance(5_000, cx);
    assert_eq!(window.saves(cx), 1, "the pending save was dropped");
    assert!(window.close(cx));
    assert_eq!(window.saves(cx), 1, "nothing is left to save");
}

#[gpui_kit::test]
fn quitting_saves_a_pending_change(cx: &mut TestAppContext) {
    let dir = temp_dir();
    let window = Window::open(&dir.path().join("state.toml"), cx);

    window.click_size_header(cx);
    // Ctrl+Q quits without closing the window first.
    cx.quit();

    assert_eq!(
        window.saved().map(|saved| saved.sort),
        Some(sorted(SortKey::Size, SortDirection::Descending))
    );
}

#[gpui_kit::test]
fn window_size_theme_filter_and_columns_are_saved(cx: &mut TestAppContext) {
    let dir = temp_dir();
    let window = Window::open(&dir.path().join("state.toml"), cx);

    cx.simulate_window_resize(window.handle, size(px(1000.), px(700.)));
    window.act(cx, |window, cx| window.click("theme-toggle", cx));
    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                ViewState {
                    filter: Filter::Paused,
                    ..view.view_state().clone()
                },
                cx,
            );
        });
    });
    // Hide the peers column through the column menu.
    window.act(cx, |window, cx| window.click("column-menu", cx));
    window.act(cx, |window, cx| {
        window.within("popup-menu").click(8usize, cx);
    });
    advance(1_000, cx);

    assert_eq!(window.saves(cx), 1);
    let saved = window.saved().expect("the state is saved");
    assert_eq!(
        saved.window,
        WindowState {
            width: 1000.,
            height: 700.,
            maximized: false
        }
    );
    assert_eq!(saved.theme, ThemeChoice::Light);
    assert_eq!(saved.filter, Filter::Paused);
    let peers = saved
        .columns
        .iter()
        .find(|column| column.id == ColumnId::Peers);
    assert_eq!(peers.map(|column| column.visible), Some(false));
    assert_eq!(saved, window.ui_state(cx));
}

#[gpui_kit::test]
fn a_state_file_that_cannot_be_read_is_never_saved_over(cx: &mut TestAppContext) {
    let dir = temp_dir();
    let path = dir.path().join("state.toml");
    fs::create_dir(&path).expect("a directory where the file should be");
    let window = Window::open(&path, cx);

    window.click_size_header(cx);
    advance(5_000, cx);
    assert!(window.close(cx));

    assert_eq!(window.saves(cx), 0);
    assert!(path.is_dir());
    assert_eq!(names(&path), [] as [String; 0]);
}

#[gpui_kit::test]
fn a_failed_save_is_shown_until_a_save_works(cx: &mut TestAppContext) {
    let dir = temp_dir();
    // The directory of the state does not exist at the start, so the state may be saved; then a
    // file takes its name, so the directory cannot be created.
    let blocker = dir.path().join("sub");
    let window = Window::open(&blocker.join("state.toml"), cx);
    fs::write(&blocker, "").expect("written");
    assert_eq!(window.not_saved_notice(cx), None);

    window.click_size_header(cx);
    advance(1_000, cx);
    assert_eq!(window.saves(cx), 1);
    assert_eq!(
        window.not_saved_notice(cx).as_deref(),
        Some("Layout not saved")
    );
    window.act(cx, |window, _| {
        assert_eq!(
            window.find("state-not-saved").description(),
            Some(
                "The columns, sorting, filter, theme, window size and details panel cannot be \
                 saved to state.toml; the log says why"
            )
        );
    });

    // Another change fails again: the notice stays, and is not added twice.
    window.click_size_header(cx);
    advance(1_000, cx);
    assert_eq!(window.saves(cx), 2);
    window.act(cx, |window, _| {
        assert_eq!(window.find_all("state-not-saved").len(), 1);
    });

    // No change, no new try.
    advance(5_000, cx);
    assert_eq!(window.saves(cx), 2);

    fs::remove_file(&blocker).expect("removed");
    window.click_size_header(cx);
    advance(1_000, cx);
    assert_eq!(window.saves(cx), 3);
    assert_eq!(window.not_saved_notice(cx), None, "the save worked");
    assert_eq!(
        window.saved().map(|saved| saved.sort),
        Some(sorted(SortKey::Size, SortDirection::Descending))
    );
}
