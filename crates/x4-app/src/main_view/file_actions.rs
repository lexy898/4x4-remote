//! The changes the Files tab of the details panel asks for, as the main view carries them out:
//! whether files of the panel's torrent are downloaded, their priority, and whether its pieces
//! come in order.
//!
//! The panel hands over a [`FileRequest`], whose action the drawn file tree built, so it names
//! the torrent and the files that were on screen. The view reads again, when it arrives, whether
//! it can go: the daemon must be connected, and the torrent must still be the one the panel is
//! about. A click on a frame drawn before the selection moved on names the torrent it showed, and
//! is refused rather than sent to another torrent with the same paths. Then it goes through the
//! same path as the other actions: kept until the daemon answers, not sent again while the same
//! change of the same files waits, and its failure shown in a notification of its own kind. The
//! service reads the torrent's progress again after a success, which shows the change.

use gpui_kit::{Context, Window};
use x4_core::{DetailsTarget, details_target};

use super::MainView;
use crate::actions::FileRequest;

impl MainView {
    /// Sends `request` from the details panel, if the daemon is connected and its torrent is the
    /// one the panel is about; otherwise sends nothing.
    pub(super) fn request_files(
        &mut self,
        request: &FileRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let action = request.kind.name();
        if !self.status.connected() {
            tracing::debug!(
                action,
                "the daemon is not connected; the change is not sent"
            );
            return;
        }
        let target = {
            let table = self.table.read(cx).delegate();
            details_target(table.selection(), table.rows())
        };
        if target != DetailsTarget::One(request.hash().clone()) {
            tracing::debug!(
                action,
                "the change was drawn for a torrent the panel no longer shows; nothing sent"
            );
            return;
        }
        let count = request.files.len();
        let Some(id) = self
            .pending
            .start_files(request.kind, request.hash(), &request.files)
        else {
            tracing::debug!(
                action,
                count,
                "the same change of the same files is on its way; nothing sent"
            );
            return;
        };
        self.run(id, request.kind, count, request.action.clone(), window, cx);
    }
}

/// What a click does when it lands on a frame drawn before the panel, the details or the
/// connection changed: gpui draws a window whose state changed as soon as the change is carried
/// out, so these change the state and click in one go, without a frame in between.
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, AppContext as _, ElementId, Entity, InputEvent as _, Modifiers,
        MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    };
    use x4_core::test_support::{details, torrent};
    use x4_core::{
        Action, Command, ConnectionError, ConnectionState, DetailsTab, DisplayState, Event,
        Snapshot, Torrent, TorrentDetails, UiState, status_model,
    };
    use x4_rpc::{FilePriority, HashString};

    use super::*;
    use crate::app;
    use crate::torrent_table::{SelectionChange, change_selection};

    /// An open main window on the Files tab, and the far ends of its channels.
    struct Harness {
        handle: AnyWindowHandle,
        view: Entity<MainView>,
        events: async_channel::Sender<Event>,
        commands: crossbeam_channel::Receiver<Command>,
    }

    impl Harness {
        fn open(cx: &mut TestAppContext) -> Self {
            let (events, events_rx) = async_channel::unbounded();
            let (commands_tx, commands) = crossbeam_channel::unbounded();
            let mut ui = UiState::default();
            ui.details.tab = DetailsTab::Files;
            let (handle, view) = cx
                .update(|cx| {
                    app::init(cx);
                    app::open_main_window(events_rx, commands_tx, None, &ui, cx)
                })
                .expect("the window opens");
            cx.update_window(handle, |_, window, _| window.activate_window())
                .expect("the window is open");
            cx.run_until_parked();
            Self {
                handle,
                view,
                events,
                commands,
            }
        }

        fn send(&self, event: Event, cx: &mut TestAppContext) {
            self.events
                .send_blocking(event)
                .expect("the window listens");
            cx.run_until_parked();
        }

        /// A window with the torrents `a` and `b`, `a` selected and its details drawn.
        fn with_a_drawn(cx: &mut TestAppContext) -> Self {
            let harness = Self::open(cx);
            harness.send(Event::Snapshot(snapshot(ConnectionState::Connected)), cx);
            harness.act(cx, |window, cx| window.click(("row", 0usize), cx));
            harness.send(Event::Details(Arc::new(files_of(&hash("a"), true))), cx);
            harness
        }

        fn act<R>(
            &self,
            cx: &mut TestAppContext,
            act: impl FnOnce(&mut Window, &mut gpui_kit::App) -> R,
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

        /// Draws the window and says where the checkbox of `Show/info.nfo` is.
        fn checkbox(&self, cx: &mut TestAppContext) -> Point<Pixels> {
            self.middle_of("file-wanted", cx)
        }

        /// Draws the window and says where the middle of the part `part` of the row of
        /// `Show/info.nfo` is.
        fn middle_of(&self, part: &'static str, cx: &mut TestAppContext) -> Point<Pixels> {
            self.act(cx, |window, _| {
                window
                    .within("file-list")
                    .within(row("Show/info.nfo"))
                    .find(part)
                    .bounds()
                    .center()
            })
        }

        /// Draws the window and says which of the rows at `paths` the file cursor is on.
        fn cursor_rows(&self, paths: &[&str], cx: &mut TestAppContext) -> Vec<String> {
            self.act(cx, |window, _| {
                paths
                    .iter()
                    .filter(|path| {
                        window
                            .within("file-list")
                            .try_find(row(path))
                            .is_some_and(|row| row.selected() == Some(true))
                    })
                    .map(|path| (*path).to_owned())
                    .collect()
            })
        }

        /// Runs `change` on the main view, then clicks at `position` on the frame drawn before it.
        fn change_then_click(
            &self,
            position: Point<Pixels>,
            change: impl FnOnce(&mut MainView, &mut Context<'_, MainView>),
            cx: &mut TestAppContext,
        ) {
            let view = self.view.clone();
            cx.update_window(self.handle, |_, window, cx| {
                view.update(cx, change);
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
                for down in [true, false] {
                    let event = if down {
                        MouseDownEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers,
                            click_count: 1,
                            first_mouse: false,
                        }
                        .to_platform_input()
                    } else {
                        MouseUpEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers,
                            click_count: 1,
                        }
                        .to_platform_input()
                    };
                    window.dispatch_event(event, cx);
                }
            })
            .expect("the window is open");
            cx.run_until_parked();
        }

        /// The actions the window has sent since the last look.
        fn actions(&self) -> Vec<Action> {
            self.commands
                .try_iter()
                .filter_map(|command| match command {
                    Command::Run(_, action) => Some(action),
                    _ => None,
                })
                .collect()
        }
    }

    /// The element id of the row of the file list at `path`.
    fn row(path: &str) -> ElementId {
        ElementId::NamedChild(
            Arc::new(ElementId::Name("file-row".into())),
            path.to_owned().into(),
        )
    }

    fn hash(name: &str) -> HashString {
        torrent(name).hash
    }

    fn downloading(name: &str) -> Torrent {
        Torrent {
            state: DisplayState::Downloading,
            ..torrent(name)
        }
    }

    fn snapshot(connection: ConnectionState) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            torrents: vec![downloading("a"), downloading("b")],
            listed: true,
            session: None,
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            stale: connection != ConnectionState::Connected,
            connection,
        })
    }

    /// The details of the torrent `hash`: a film and its `info.nfo`, wanted or not.
    fn files_of(hash: &HashString, info_wanted: bool) -> TorrentDetails {
        details(
            hash,
            &[
                ("Show/film.mkv", 1024, 0, true, FilePriority::Normal),
                ("Show/info.nfo", 1, 1, info_wanted, FilePriority::Normal),
            ],
        )
    }

    #[gpui_kit::test]
    fn a_click_on_the_rows_of_a_torrent_the_panel_has_left_is_refused(cx: &mut TestAppContext) {
        let harness = Harness::with_a_drawn(cx);
        let position = harness.checkbox(cx);

        // The selection moves to b and the panel follows, but the frame still shows a's rows.
        harness.change_then_click(
            position,
            |view, cx| {
                let step = SelectionChange::Step {
                    step: 1,
                    extend: false,
                };
                view.table
                    .update(cx, |table, cx| change_selection(table, step, cx));
                view.follow_details(cx);
            },
            cx,
        );

        assert_eq!(harness.actions(), []);
    }

    #[gpui_kit::test]
    fn a_click_on_a_row_of_a_torrent_the_panel_has_left_leaves_the_new_cursor_alone(
        cx: &mut TestAppContext,
    ) {
        let harness = Harness::with_a_drawn(cx);
        let position = harness.middle_of("file-name", cx);

        // The selection moves to b, whose details come with the same paths, so that b's tree is
        // the one the panel has; the click lands on a's row on the frame drawn before.
        harness.change_then_click(
            position,
            |view, cx| {
                let step = SelectionChange::Step {
                    step: 1,
                    extend: false,
                };
                view.table
                    .update(cx, |table, cx| change_selection(table, step, cx));
                view.follow_details(cx);
                view.details_arrived(Arc::new(files_of(&hash("b"), true)), cx);
            },
            cx,
        );

        // The same path in b is another file: the cursor does not go there.
        let paths = ["Show", "Show/film.mkv", "Show/info.nfo"];
        assert_eq!(harness.cursor_rows(&paths, cx), Vec::<String>::new());
        assert_eq!(harness.actions(), []);
        // A click on b's own frame moves it.
        let position = harness.middle_of("file-name", cx);
        harness.change_then_click(position, |_, _| {}, cx);
        assert_eq!(harness.cursor_rows(&paths, cx), ["Show/info.nfo"]);
    }

    #[gpui_kit::test]
    fn a_click_takes_the_tree_drawn_when_newer_details_came_before_the_frame(
        cx: &mut TestAppContext,
    ) {
        let harness = Harness::with_a_drawn(cx);
        let position = harness.checkbox(cx);

        // A read says the file is skipped now; the box on screen is still checked, so the click
        // skips it, as the user saw it, rather than wanting it again.
        harness.change_then_click(
            position,
            |view, cx| {
                view.details_arrived(Arc::new(files_of(&hash("a"), false)), cx);
            },
            cx,
        );

        assert_eq!(
            harness.actions(),
            [Action::SetFilesWanted {
                hash: hash("a"),
                files: vec![1],
                wanted: false,
            }]
        );
    }

    #[gpui_kit::test]
    fn a_click_drawn_before_the_connection_broke_sends_nothing(cx: &mut TestAppContext) {
        let harness = Harness::with_a_drawn(cx);
        let position = harness.checkbox(cx);

        harness.change_then_click(
            position,
            |view, _| {
                view.status = status_model(&snapshot(ConnectionState::Reconnecting {
                    attempt: 1,
                    last_error: ConnectionError::Timeout,
                }));
            },
            cx,
        );

        assert_eq!(harness.actions(), []);
    }
}
