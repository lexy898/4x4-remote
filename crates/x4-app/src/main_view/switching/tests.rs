//! Switching servers, with a fake connector that hands out channels the test holds, standing in
//! for the service threads.

use std::cell::{Ref, RefCell};
use std::io;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::TryRecvError;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, TestAppContext};
use x4_core::test_support::{details, torrent};
use x4_core::{
    Command, ConnectionState, Event, Filter, KbPerSecond, Peer, PeerList, PeerSources, PeerTraits,
    ServiceStatus, SessionSummary, Snapshot, SortDirection, SortKey, SortState, SpeedLimit,
    ThemeChoice, Torrent, UiState, Visibility, availability,
};

use super::*;
use crate::actions::TorrentRequest;
use crate::app::{self, ConfigProblem};
use crate::connection::Connector;
use crate::torrent_table::select_added;

/// The test's ends of the channels of one service.
struct FakeService {
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
}

impl FakeService {
    /// Sends `event` as the service would, and lets the window take it.
    fn send(&self, event: Event, cx: &mut TestAppContext) {
        self.events
            .send_blocking(event)
            .expect("the window listens");
        cx.run_until_parked();
    }

    /// The commands the window has sent since the last call.
    fn sent(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }
}

/// The services the fake connector started, oldest first, with the servers they were for.
type Started = Rc<RefCell<Vec<(ConnectionSettings, FakeService)>>>;

/// A connector that starts no thread: it keeps the far ends of the channels in `started`.
fn fake_connector(started: &Started) -> Connector {
    let started = Rc::clone(started);
    Rc::new(move |settings| {
        let (events_tx, events) = async_channel::unbounded();
        let (commands, commands_rx) = crossbeam_channel::unbounded();
        started.borrow_mut().push((
            settings.clone(),
            FakeService {
                events: events_tx,
                commands: commands_rx,
            },
        ));
        Ok(ServiceChannels {
            events,
            commands,
            handle: None,
        })
    })
}

fn settings(url: &str) -> ConnectionSettings {
    ConnectionSettings::from_url(url, None).expect("a plain http address")
}

/// The server the window starts with.
fn pi() -> ConnectionSettings {
    settings("http://pi:9091/transmission/rpc")
}

/// The server it switches to.
fn nas() -> ConnectionSettings {
    settings("http://nas:9091/transmission/rpc")
}

/// A main window set to [`pi`], over a first service whose channels the test holds, which
/// switches with a fake connector.
struct Switchable {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    first: FakeService,
    started: Started,
}

impl Switchable {
    /// Opens the window with the state `ui`.
    fn open(ui: &UiState, cx: &mut TestAppContext) -> Self {
        let (events, events_rx) = async_channel::unbounded();
        let (commands, commands_rx) = crossbeam_channel::unbounded();
        // A notification taken down goes at once.
        cx.update(|cx| cx.set_reduce_motion(true));
        let (handle, view) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands, None, ui, cx)
            })
            .expect("the window opens");
        let started = Started::default();
        let setup = ConnectionSetup {
            settings: Some(pi()),
            connector: Some(fake_connector(&started)),
            ..ConnectionSetup::default()
        };
        cx.update(|cx| view.update(cx, |view, cx| view.set_connection(setup, cx)));
        cx.run_until_parked();
        Self {
            handle,
            view,
            first: FakeService {
                events,
                commands: commands_rx,
            },
            started,
        }
    }

    /// Runs `act` on the view in its window, after drawing a frame, then lets what it started
    /// finish.
    fn act<R>(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut MainView, &mut Window, &mut Context<'_, MainView>) -> R,
    ) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                self.view.update(cx, |view, cx| act(view, window, cx))
            })
            .expect("the window is open");
        cx.run_until_parked();
        result
    }

    /// Reads the view.
    fn read<R>(&self, cx: &mut TestAppContext, read: impl FnOnce(&MainView, &App) -> R) -> R {
        cx.update(|cx| read(self.view.read(cx), cx))
    }

    fn switch_to(&self, settings: ConnectionSettings, cx: &mut TestAppContext) {
        self.act(cx, |view, window, cx| view.switch_to(settings, window, cx));
    }

    /// The service the connector started last.
    fn newest(&self) -> Ref<'_, FakeService> {
        Ref::map(self.started.borrow(), |started| {
            &started.last().expect("a service was started").1
        })
    }

    /// The names of the rows the table shows, in order.
    fn names(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.read(cx, |view, cx| {
            view.table
                .read(cx)
                .delegate()
                .rows()
                .iter()
                .map(|row| row.name.clone())
                .collect()
        })
    }

    /// How many notifications the window shows.
    fn notifications(&self, cx: &mut TestAppContext) -> usize {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_all("notification").len()
        })
        .expect("the window is open")
    }

    /// The accessible name of the connection indicator in the status bar.
    fn indicator(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("status-connection").label().map(str::to_owned)
        })
        .expect("the window is open")
    }
}

/// A session whose limits can be edited.
fn session() -> SessionSummary {
    let limit = SpeedLimit {
        kbps: KbPerSecond(500),
        enabled: true,
    };
    SessionSummary {
        version: "4.1.0".to_owned(),
        download_dir: "/data".to_owned(),
        alt_speed_enabled: false,
        speed_limit_down: limit,
        speed_limit_up: limit,
        alt_speed_down_kbps: KbPerSecond(50),
        alt_speed_up_kbps: KbPerSecond(50),
        start_added_torrents: true,
        sequential_download_supported: true,
        lists_empty_files: true,
    }
}

/// A snapshot of `torrents` with the daemon connected, some traffic and the session known.
fn snapshot(torrents: Vec<Torrent>) -> Event {
    Event::Snapshot(Arc::new(Snapshot {
        torrents,
        listed: true,
        session: Some(session()),
        free_space_bytes: Some(1 << 30),
        rate_down: 4096,
        rate_up: 2048,
        connection: ConnectionState::Connected,
        stale: false,
    }))
}

fn peer(address: &str) -> Peer {
    Peer {
        ip: address.parse().ok(),
        address: address.to_owned(),
        port: 6881,
        client: String::new(),
        progress: 0.0,
        rate_down: 0,
        rate_up: 0,
        traits: PeerTraits::default(),
        flags: String::new(),
        country: None,
    }
}

/// The state of a window whose details panel is open on the Peers tab.
fn on_the_peers_tab() -> UiState {
    let mut ui = UiState::default();
    ui.details.open = true;
    ui.details.tab = DetailsTab::Peers;
    ui
}

#[gpui_kit::test]
fn switching_forgets_the_old_daemon(cx: &mut TestAppContext) {
    let window = Switchable::open(&on_the_peers_tab(), cx);
    let film = torrent("film");
    window
        .first
        .send(snapshot(vec![film.clone(), torrent("other")]), cx);
    // The film is selected, its details and peers shown on the Peers tab; a pause and a read of
    // its magnet link wait for their answers, a failure shows, and the speed limits are open.
    window.act(cx, |view, _, cx| {
        view.table
            .update(cx, |table, cx| select_added(table, film.hash.clone(), cx));
    });
    window
        .first
        .send(Event::Details(Arc::new(details(&film.hash, &[]))), cx);
    window.first.send(
        Event::Peers(Arc::new(PeerList {
            hash: film.hash.clone(),
            peers: vec![peer("192.0.2.1"), peer("192.0.2.2")],
            sources: PeerSources::default(),
        })),
        cx,
    );
    window.act(cx, |view, window, cx| {
        view.request(TorrentRequest::Pause, window, cx);
        view.copy_selected_magnet_links(window, cx);
        view.show_failure("pause", "Could not pause film".into(), window, cx);
        view.edit_speed_limits(window, cx);
        view.show_config_problem(ConfigProblem::Unreadable, cx);
    });
    let panel = window.read(cx, |view, _| view.details_panel.clone());
    let peer_rows =
        |cx: &mut TestAppContext| cx.update(|cx| panel.read(cx).peers_dimming(cx).1.len());
    // What the old daemon left, so that the test sees it go.
    window.read(cx, |view, cx| {
        assert_eq!(view.model.rows.len(), 2);
        assert_eq!(view.table.read(cx).delegate().selection().len(), 1);
        assert!(!view.details_panel.read(cx).is_about_nothing());
        assert!(view.status.connected());
        assert_eq!(view.pending.len(), 1, "the pause waits");
        assert!(view.magnet.is_waiting());
        assert!(view.last_failure.is_some());
        assert!(view.limits_open);
        assert!(!view.speed_limits.read(cx).is_empty(cx));
        assert!(view.config_problem.is_some());
        assert_eq!(view.selected_details, Some(film.hash.clone()));
    });
    assert_eq!(peer_rows(cx), 2);
    assert!(window.notifications(cx) > 0);

    window.switch_to(nas(), cx);

    window.read(cx, |view, cx| {
        assert!(view.last_snapshot.is_none());
        assert_eq!(view.model, TableModel::default(), "no rows, no counts");
        assert_eq!(view.model.shown_counts(), None, "the sidebar says nothing");
        let table = view.table.read(cx).delegate();
        assert_eq!(table.rows(), []);
        assert!(table.selection().is_empty(), "nothing selected");
        assert_eq!(table.selection().lead(), None, "no lead either");
        assert!(
            view.details_panel.read(cx).is_about_nothing(),
            "the panel is empty"
        );
        assert_eq!(
            view.status,
            StatusModel::default(),
            "connecting, no numbers"
        );
        assert_eq!(view.connection_text(), "Connecting…");
        assert_eq!(view.pending.len(), 0, "no pending action");
        assert!(!view.magnet.is_waiting(), "no magnet copy");
        assert_eq!(view.last_failure, None);
        assert!(!view.limits_open, "the popover closed");
        assert!(view.speed_limits.read(cx).is_empty(cx), "the form is empty");
        assert_eq!(view.config_problem, None);
        assert_eq!(view.selected_details, None);
        assert_eq!(view.availability(), availability(false, &[]));
        assert_eq!(view.connection_settings(), Some(&nas()));
    });
    assert_eq!(peer_rows(cx), 0, "no peers");
    // A notification taken down leaves on the list's clock.
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(window.notifications(cx), 0, "the old failure is gone");

    // The new daemon has the same torrent: it is not selected again, and the panel stays empty.
    window.newest().send(snapshot(vec![film.clone()]), cx);
    window.read(cx, |view, cx| {
        assert_eq!(view.model.rows.len(), 1);
        assert!(view.table.read(cx).delegate().selection().is_empty());
        assert!(view.details_panel.read(cx).is_about_nothing());
        assert!(view.status.connected());
    });
    assert_eq!(peer_rows(cx), 0);
    // The pause went to the old daemon; the same pause goes to the new one, as nothing waits.
    window.act(cx, |view, window, cx| {
        view.table
            .update(cx, |table, cx| select_added(table, film.hash.clone(), cx));
        view.request(TorrentRequest::Pause, window, cx);
    });
    assert!(
        window
            .newest()
            .sent()
            .iter()
            .any(|command| matches!(command, Command::Run(..))),
        "the pause is sent to the new server"
    );
}

#[gpui_kit::test]
fn switching_tells_the_new_service_the_visibility_and_the_tab(cx: &mut TestAppContext) {
    let window = Switchable::open(&on_the_peers_tab(), cx);
    let film = torrent("film");
    window.first.send(snapshot(vec![film.clone()]), cx);
    window.act(cx, |view, _, cx| {
        view.table
            .update(cx, |table, cx| select_added(table, film.hash.clone(), cx));
    });
    let told_first = window.first.sent();
    let visibility = told_first
        .iter()
        .find_map(|command| match command {
            Command::SetVisibility(visibility) => Some(*visibility),
            _ => None,
        })
        .expect("the first service was told the visibility");
    assert!(told_first.contains(&Command::ShowTab(DetailsTab::Peers)));
    assert!(told_first.contains(&Command::Select(Some(film.hash.clone()))));

    window.switch_to(nas(), cx);

    assert_eq!(
        window.newest().sent(),
        [
            Command::SetVisibility(visibility),
            Command::ShowTab(DetailsTab::Peers),
        ],
        "what a service is told at the start, and no torrent's details"
    );
    assert_eq!(
        window.first.sent(),
        [],
        "nothing more goes to the old service"
    );
    let switched_to = window
        .started
        .borrow()
        .iter()
        .map(|(settings, _)| settings.clone())
        .collect::<Vec<_>>();
    assert_eq!(switched_to, [nas()]);

    // What changes later goes to the new service alone.
    cx.update_window(window.handle, |_, window, _| window.activate_window())
        .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(
        window.newest().sent(),
        [Command::SetVisibility(Visibility::Visible)]
    );
}

#[gpui_kit::test]
fn the_old_service_no_longer_reaches_the_view(cx: &mut TestAppContext) {
    let window = Switchable::open(&UiState::default(), cx);
    window.first.send(snapshot(vec![torrent("film")]), cx);

    window.switch_to(nas(), cx);
    let late = window
        .first
        .events
        .send_blocking(snapshot(vec![torrent("late")]));
    cx.run_until_parked();

    assert!(late.is_err(), "the old pump let go of its channel");
    assert!(window.first.events.is_closed());
    assert_eq!(window.names(cx), [] as [String; 0]);
    assert_eq!(
        window.read(cx, |view, _| view.connection_text()),
        "Connecting…"
    );
    let _ = window.first.sent();
    assert_eq!(
        window.first.commands.try_recv(),
        Err(TryRecvError::Disconnected),
        "the view let go of the old service's commands too"
    );

    window.newest().send(snapshot(vec![torrent("new")]), cx);
    assert_eq!(window.names(cx), ["new"]);
}

#[gpui_kit::test]
fn switching_keeps_the_ui_state(cx: &mut TestAppContext) {
    let mut ui = UiState {
        filter: Filter::Seeding,
        theme: ThemeChoice::Dark,
        sort: SortState {
            key: SortKey::Size,
            direction: SortDirection::Descending,
        },
        ..on_the_peers_tab()
    };
    ui.details.height = 300.0;
    if let Some(column) = ui.columns.get_mut(1) {
        column.visible = !column.visible;
    }
    let window = Switchable::open(&ui, cx);
    window.first.send(snapshot(vec![torrent("film")]), cx);
    cx.update_window(window.handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-f", cx);
    })
    .expect("the window is open");
    cx.update_window(window.handle, |_, window, cx| window.input("fil", cx))
        .expect("the window is open");
    cx.run_until_parked();
    let (before, view_before) = window.read(cx, |view, _| (view.ui.clone(), view.view.clone()));
    assert_eq!(view_before.search, "fil");

    window.switch_to(nas(), cx);

    let (after, view_after) = window.read(cx, |view, _| (view.ui.clone(), view.view.clone()));
    assert_eq!(after, before, "the columns, sort, filter, theme and panel");
    assert_eq!(
        view_after, view_before,
        "the filter, the search and the order"
    );
    let search_box = cx
        .update_window(window.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("search").value().map(str::to_owned)
        })
        .expect("the window is open");
    assert_eq!(search_box.as_deref(), Some("fil"));
}

#[gpui_kit::test]
fn a_connector_that_fails_shows_the_service_failed(cx: &mut TestAppContext) {
    let window = Switchable::open(&UiState::default(), cx);
    window.first.send(snapshot(vec![torrent("film")]), cx);
    let failing: Connector = Rc::new(|_| Err(io::Error::other("no more threads")));
    window.act(cx, |view, _, cx| {
        view.set_connection(
            ConnectionSetup {
                settings: Some(pi()),
                connector: Some(failing),
                ..ConnectionSetup::default()
            },
            cx,
        );
    });

    window.switch_to(nas(), cx);

    assert!(
        window.first.events.is_closed(),
        "the old service is dropped all the same"
    );
    window.read(cx, |view, cx| {
        assert_eq!(view.status.service, ServiceStatus::Failed);
        assert_eq!(view.connection_text(), "Service could not start");
        assert_eq!(view.table.read(cx).delegate().rows(), []);
        assert_eq!(
            view.connection_settings(),
            Some(&nas()),
            "set to the new server"
        );
        assert_eq!(view.availability(), availability(false, &[]));
    });
    assert_eq!(
        window.indicator(cx).as_deref(),
        Some("Service could not start")
    );
}

#[gpui_kit::test]
fn connected_tooltip_names_the_server(cx: &mut TestAppContext) {
    let window = Switchable::open(&UiState::default(), cx);
    window.first.send(snapshot(Vec::new()), cx);
    assert_eq!(
        window.indicator(cx).as_deref(),
        Some("Connected to pi:9091")
    );

    window.switch_to(nas(), cx);
    assert_eq!(window.indicator(cx).as_deref(), Some("Connecting…"));
    window.newest().send(snapshot(Vec::new()), cx);

    assert_eq!(
        window.indicator(cx).as_deref(),
        Some("Connected to nas:9091")
    );
}

#[gpui_kit::test]
fn a_window_that_cannot_switch_stays_as_it_is(cx: &mut TestAppContext) {
    let window = Switchable::open(&UiState::default(), cx);
    window.act(cx, |view, _, cx| {
        view.set_connection(ConnectionSetup::default(), cx);
    });
    window.first.send(snapshot(vec![torrent("film")]), cx);
    let _ = window.first.sent();

    window.switch_to(nas(), cx);

    assert_eq!(window.names(cx), ["film"]);
    assert_eq!(
        window.read(cx, |view, _| view.connection_settings().cloned()),
        None
    );
    assert_eq!(window.indicator(cx).as_deref(), Some("Connected"));
    window.first.send(snapshot(vec![torrent("still")]), cx);
    assert_eq!(
        window.names(cx),
        ["still"],
        "the first service still reaches it"
    );
}
