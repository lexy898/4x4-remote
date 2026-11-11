//! The Connection dialog in a window of its own, driven as the user drives it: the fields typed
//! into or pasted into, Save, Test, Enter, Escape, Tab and a held key. A fake check stands in for
//! the server and answers when the test says, and the window records the key held as the main
//! view does. What the main view does with the dialog's events is the main view's to test.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt as _;
// The platform's input events; the dialog's own `InputEvent` is the text field's.
use gpui_kit::InputEvent as _;
use gpui_kit::{AnyWindowHandle, ClipboardItem, Keystroke, TestAppContext, WindowOptions, div};
use x4_core::{CheckOutcome, ConnectionError, UnsupportedReason};
use x4_rpc::Credentials;

use super::*;
use crate::app;

/// A made-up secret, which must never show.
const PASSWORD: &str = "hunter2";

const TITLE: &str = "connection-title";
const HOST: &str = "connection-host";
const PORT: &str = "connection-port";
const PATH: &str = "connection-path";
const USERNAME: &str = "connection-username";
const PASSWORD_FIELD: &str = "connection-password";
const HOST_PROBLEM: &str = "connection-host-problem";
const PORT_PROBLEM: &str = "connection-port-problem";
const PATH_PROBLEM: &str = "connection-path-problem";
const USERNAME_PROBLEM: &str = "connection-username-problem";
const ADDRESS: &str = "connection-address";
const TEST: &str = "connection-test";
const RESULT: &str = "connection-test-result";
const SAVE: &str = "connection-save";
const CANCEL: &str = "connection-cancel";
const SAVE_ERROR: &str = "connection-save-error";
/// The eye button of the password field, inside the field.
const MASK_TOGGLE: &str = "toggle-mask";
/// The key that selects the whole text of a field. gpui-base's text fields take ⌘A on macOS,
/// where Ctrl+A moves to the start of the line, and Ctrl+A elsewhere.
const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};
/// The key that pastes into a field: ⌘V on macOS and Ctrl+V elsewhere, as gpui-base's text
/// fields take them.
const PASTE: &str = if cfg!(target_os = "macos") {
    "cmd-v"
} else {
    "ctrl-v"
};

const PROBLEMS: [&str; 4] = [HOST_PROBLEM, PORT_PROBLEM, PATH_PROBLEM, USERNAME_PROBLEM];

fn settings(url: &str, credentials: Option<Credentials>) -> ConnectionSettings {
    ConnectionSettings::from_url(url, credentials).expect("a plain http address")
}

fn admin() -> Credentials {
    Credentials {
        username: "admin".to_owned(),
        password: PASSWORD.to_owned(),
    }
}

/// The window's own content, behind the dialog: nothing.
struct Behind;

impl Render for Behind {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
    }
}

/// One call of the fake check: the settings it was asked about, and where its answer goes.
struct Call {
    settings: ConnectionSettings,
    answer: async_channel::Sender<CheckOutcome>,
}

/// The calls of a fake check, oldest first.
type Calls = Rc<RefCell<Vec<Call>>>;

/// A check that notes each call and answers it when the test sends the answer; one whose answer
/// is let go of closes its channel.
fn fake_check() -> (ServerCheck, Calls) {
    let calls = Calls::default();
    let noted = Rc::clone(&calls);
    let check: ServerCheck = Rc::new(move |settings, cx| {
        let (answer, answered) = async_channel::bounded(1);
        noted.borrow_mut().push(Call {
            settings: settings.clone(),
            answer,
        });
        cx.background_executor()
            .spawn(async move { answered.recv().await.unwrap_or(CheckOutcome::NotRun) })
    });
    (check, calls)
}

/// The only call `calls` has had since the last one taken.
fn only_call(calls: &Calls) -> Call {
    let mut calls = calls.borrow_mut();
    assert_eq!(calls.len(), 1, "one check");
    calls.remove(0)
}

/// An open dialog in a window of its own, and what it has asked for.
struct Shown {
    handle: AnyWindowHandle,
    dialog: Entity<ConnectionDialog>,
    events: Rc<RefCell<Vec<ConnectionDialogEvent>>>,
    /// Records the key-downs of the window in the dialog's record of the key held, as the main
    /// view's keystroke interceptor does.
    _keys: Subscription,
}

impl Shown {
    /// Opens the dialog for `settings`, testing with `check`, in an active window with the app's
    /// keys and animations off, which records the key held as the main window does.
    fn open(
        settings: Option<&ConnectionSettings>,
        check: Option<ServerCheck>,
        cx: &mut TestAppContext,
    ) -> Self {
        cx.update(|cx| {
            app::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Behind))
            })
            .expect("the window opens");
        let events = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&events);
        let repeats = Rc::new(RefCell::new(KeyRepeats::default()));
        let recorded = Rc::clone(&repeats);
        let keys = cx.update(|cx| {
            cx.intercept_keystrokes(move |event, _, _| {
                recorded.borrow_mut().key_down(&event.keystroke);
            })
        });
        let dialog = cx
            .update_window(handle, |_, window, cx| {
                let dialog =
                    cx.new(|cx| ConnectionDialog::new(repeats, settings, check, window, cx));
                cx.subscribe(&dialog, move |_, event: &ConnectionDialogEvent, _| {
                    seen.borrow_mut().push(event.clone());
                })
                .detach();
                open(dialog.clone(), window, cx);
                window.activate_window();
                dialog
            })
            .expect("the window is open");
        cx.run_until_parked();
        Self {
            handle,
            dialog,
            events,
            _keys: keys,
        }
    }

    fn act<R>(&self, cx: &mut TestAppContext, act: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                act(window, cx)
            })
            .expect("the window is open");
        cx.run_until_parked();
        result
    }

    fn click(&self, id: &'static str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.click(id, cx));
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    /// Holds `key` down until the system has repeated it `repeats` times, then lets it go. The
    /// repeats are marked held, as Wayland marks them; X11 does not, and sends no key-up between
    /// them either, which the record of the key held tells all the same.
    fn hold(&self, key: &str, repeats: usize, cx: &mut TestAppContext) {
        let keystroke = Keystroke::parse_with_os(key, cx.update(|cx| cx.platform_info().os))
            .expect("a valid key");
        for is_held in std::iter::once(false).chain(std::iter::repeat_n(true, repeats)) {
            let down = KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held,
                prefer_character_input: false,
            };
            self.act(cx, |window, cx| {
                window.dispatch_event(down.to_platform_input(), cx);
            });
        }
        let up = KeyUpEvent { keystroke };
        self.act(cx, |window, cx| {
            window.dispatch_event(up.to_platform_input(), cx);
        });
    }

    /// Types `text` into what has the focus, a key at a time.
    fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.input(text, cx));
    }

    /// Replaces the text of the field `id` with `text`, as the user would: a click into it,
    /// [`SELECT_ALL`] and the new text, typed.
    fn retype(&self, id: &'static str, text: &str, cx: &mut TestAppContext) {
        self.click(id, cx);
        self.press(SELECT_ALL, cx);
        if text.is_empty() {
            self.press("backspace", cx);
        } else {
            self.type_text(text, cx);
        }
    }

    /// Pastes `text` over the whole of the field `id`, with [`PASTE`].
    fn paste(&self, id: &'static str, text: &str, cx: &mut TestAppContext) {
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned())));
        self.click(id, cx);
        self.press(SELECT_ALL, cx);
        self.press(PASTE, cx);
    }

    /// What the element `id` says: the accessible name of a field, a button or a line of text;
    /// `None` when it is not on screen.
    fn label(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| {
            window
                .try_find(id)
                .and_then(|found| found.label().map(str::to_owned))
        })
    }

    /// The text the field `id` shows to screen readers.
    fn shown_value(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| window.find(id).value().map(str::to_owned))
    }

    fn placeholder(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| {
            window.find(id).placeholder().map(str::to_owned)
        })
    }

    fn focused(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).focused() == Some(true))
    }

    fn disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).disabled() == Some(true))
    }

    fn dialog_open(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, WindowExt::has_active_dialog)
    }

    /// The text of the five fields: host, port, path, user name and password.
    fn fields(&self, cx: &mut TestAppContext) -> [String; 5] {
        cx.update(|cx| {
            let dialog = self.dialog.read(cx);
            [
                &dialog.host,
                &dialog.port,
                &dialog.path,
                &dialog.username,
                &dialog.password,
            ]
            .map(|input| input.read(cx).value().to_string())
        })
    }

    /// Whether the password field draws its text masked.
    fn password_masked(&self, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| {
            self.dialog
                .read(cx)
                .password
                .read(cx)
                .presentation()
                .is_masked()
        })
    }

    /// What the dialog has asked for since the last call.
    fn events(&self) -> Vec<ConnectionDialogEvent> {
        self.events.borrow_mut().drain(..).collect()
    }

    /// Every text an element of the window gives screen readers: names, values, descriptions and
    /// placeholders.
    fn every_text(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.act(cx, |window, _| {
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .flat_map(|element| {
                    [
                        element.label(),
                        element.value(),
                        element.description(),
                        element.placeholder(),
                    ]
                })
                .flatten()
                .map(str::to_owned)
                .collect()
        })
    }
}

fn texts(fields: [&str; 5]) -> [String; 5] {
    fields.map(str::to_owned)
}

#[gpui_kit::test]
fn opens_with_the_current_settings(cx: &mut TestAppContext) {
    let current = settings("http://nas:9092/rpc", Some(admin()));
    let shown = Shown::open(Some(&current), None, cx);

    assert_eq!(shown.label(TITLE, cx).as_deref(), Some("Connection"));
    assert_eq!(
        shown.fields(cx),
        texts(["nas", "9092", "/rpc", "admin", PASSWORD])
    );
    for (id, name) in [
        (HOST, "Host"),
        (PORT, "Port"),
        (PATH, "Path"),
        (USERNAME, "User name"),
        (PASSWORD_FIELD, "Password"),
    ] {
        assert_eq!(shown.label(id, cx).as_deref(), Some(name), "{id}");
    }
    assert!(shown.focused(HOST, cx), "the focus starts on the host");
    assert_eq!(
        shown.label("connection-credentials-hint", cx).as_deref(),
        Some("Leave empty if the server asks for no login")
    );
    assert_eq!(
        shown.label(ADDRESS, cx).as_deref(),
        Some("Address: http://nas:9092/rpc")
    );
    assert_eq!(shown.label(TEST, cx).as_deref(), Some("Test"));
    assert!(shown.disabled(TEST, cx), "the window has no check");
    assert_eq!(shown.label(CANCEL, cx).as_deref(), Some("Cancel"));
    assert_eq!(shown.label(SAVE, cx).as_deref(), Some("Save"));
    for id in PROBLEMS.into_iter().chain([RESULT, SAVE_ERROR]) {
        assert_eq!(shown.label(id, cx), None, "{id}");
    }

    // The password is masked, and no element shows it, until the eye shows it.
    assert!(shown.password_masked(cx));
    assert_eq!(shown.shown_value(PASSWORD_FIELD, cx), None);
    let every_text = shown.every_text(cx);
    assert!(
        every_text.iter().any(|text| text == "admin"),
        "the user name shows: {every_text:?}"
    );
    let leaks: Vec<&String> = every_text
        .iter()
        .filter(|text| text.contains(PASSWORD))
        .collect();
    assert!(leaks.is_empty(), "the password shows in {leaks:?}");
    shown.act(cx, |window, cx| {
        window.within(PASSWORD_FIELD).click(MASK_TOGGLE, cx);
    });
    assert!(!shown.password_masked(cx));
    assert_eq!(
        shown.shown_value(PASSWORD_FIELD, cx).as_deref(),
        Some(PASSWORD)
    );
    assert_eq!(shown.events(), []);
}

#[gpui_kit::test]
fn opens_with_the_defaults_when_no_server_is_set(cx: &mut TestAppContext) {
    let shown = Shown::open(None, None, cx);

    assert_eq!(
        shown.fields(cx),
        texts(["", "9091", "/transmission/rpc", "", ""])
    );
    assert_eq!(
        shown.placeholder(HOST, cx).as_deref(),
        Some("Name or IP address")
    );
    assert!(shown.password_masked(cx));
    assert!(shown.focused(HOST, cx));
    // No host yet: no address, and no problem before Save or Test.
    for id in PROBLEMS.into_iter().chain([ADDRESS, RESULT, SAVE_ERROR]) {
        assert_eq!(shown.label(id, cx), None, "{id}");
    }
}

#[gpui_kit::test]
fn the_address_line_follows_the_fields(cx: &mut TestAppContext) {
    let current = settings("http://nas:9091/transmission/rpc", Some(admin()));
    let shown = Shown::open(Some(&current), None, cx);
    assert_eq!(
        shown.label(ADDRESS, cx).as_deref(),
        Some("Address: http://nas:9091/transmission/rpc"),
        "never with the credentials"
    );

    shown.retype(PORT, "9092", cx);
    assert_eq!(
        shown.label(ADDRESS, cx).as_deref(),
        Some("Address: http://nas:9092/transmission/rpc")
    );
    shown.retype(HOST, "::1", cx);
    shown.retype(PATH, "rpc", cx);
    assert_eq!(
        shown.label(ADDRESS, cx).as_deref(),
        Some("Address: http://[::1]:9092/rpc")
    );

    // While the form has a problem there is no address, and the problem does not show before
    // Save or Test.
    shown.retype(PORT, "x", cx);
    assert_eq!(shown.label(ADDRESS, cx), None);
    assert_eq!(shown.label(PORT_PROBLEM, cx), None);
    shown.retype(USERNAME, "a:b", cx);
    shown.retype(PORT, "9093", cx);
    assert_eq!(shown.label(ADDRESS, cx), None, "the user name has a colon");
    shown.retype(USERNAME, "", cx);
    assert_eq!(
        shown.label(ADDRESS, cx).as_deref(),
        Some("Address: http://[::1]:9093/rpc")
    );
    assert_eq!(shown.events(), []);
}

#[gpui_kit::test]
fn save_with_problems_shows_them_and_emits_nothing(cx: &mut TestAppContext) {
    let shown = Shown::open(None, None, cx);
    shown.click(SAVE, cx);
    assert_eq!(shown.events(), []);
    assert_eq!(
        shown.label(HOST_PROBLEM, cx).as_deref(),
        Some("Enter the server's name or IP address")
    );
    for id in [PORT_PROBLEM, PATH_PROBLEM, USERNAME_PROBLEM] {
        assert_eq!(shown.label(id, cx), None, "{id}");
    }
    assert!(shown.focused(HOST, cx), "the field of the first problem");

    // From then on the problems follow the fields.
    shown.retype(HOST, "https://nas", cx);
    assert_eq!(
        shown.label(HOST_PROBLEM, cx).as_deref(),
        Some("HTTPS is not supported yet")
    );
    shown.retype(HOST, "nas/x", cx);
    assert_eq!(
        shown.label(HOST_PROBLEM, cx).as_deref(),
        Some("Not a valid name or IP address")
    );
    shown.retype(HOST, "nas", cx);
    assert_eq!(shown.label(HOST_PROBLEM, cx), None);
    assert_eq!(
        shown.label(ADDRESS, cx).as_deref(),
        Some("Address: http://nas:9091/transmission/rpc")
    );

    shown.retype(PORT, "", cx);
    assert_eq!(
        shown.label(PORT_PROBLEM, cx).as_deref(),
        Some("Enter the port")
    );
    for port in ["x", "0", "70000"] {
        shown.retype(PORT, port, cx);
        assert_eq!(
            shown.label(PORT_PROBLEM, cx).as_deref(),
            Some("The port is a number from 1 to 65535"),
            "{port}"
        );
    }
    shown.retype(PATH, "a b", cx);
    assert_eq!(
        shown.label(PATH_PROBLEM, cx).as_deref(),
        Some("The path cannot hold ?, # or spaces")
    );
    shown.retype(PATH, "", cx);
    assert_eq!(
        shown.label(PATH_PROBLEM, cx).as_deref(),
        Some("Enter the path, usually /transmission/rpc")
    );
    shown.retype(USERNAME, "a:b", cx);
    assert_eq!(
        shown.label(USERNAME_PROBLEM, cx).as_deref(),
        Some("A user name cannot contain a colon")
    );
    assert_eq!(shown.label(ADDRESS, cx), None);

    shown.click(SAVE, cx);
    assert_eq!(shown.events(), []);
    assert!(
        shown.focused(PORT, cx),
        "the port is the first wrong field now"
    );
    assert!(shown.dialog_open(cx));
}

#[gpui_kit::test]
fn save_emits_the_settings(cx: &mut TestAppContext) {
    let shown = Shown::open(None, None, cx);
    shown.retype(HOST, "nas", cx);
    shown.retype(USERNAME, "admin", cx);
    shown.retype(PASSWORD_FIELD, PASSWORD, cx);

    shown.click(SAVE, cx);

    let events = shown.events();
    assert_eq!(
        events,
        [ConnectionDialogEvent::Save(settings(
            "http://nas:9091/transmission/rpc",
            Some(admin())
        ))]
    );
    assert!(
        !format!("{events:?}").contains(PASSWORD),
        "the event hides the password"
    );
    assert!(shown.dialog_open(cx), "the main view closes the dialog");
    for id in PROBLEMS {
        assert_eq!(shown.label(id, cx), None, "{id}");
    }

    // Spaces around the host, port and path are dropped; the credentials are taken as typed.
    shown.retype(HOST, " nas ", cx);
    shown.retype(USERNAME, " admin", cx);
    shown.click(SAVE, cx);
    assert_eq!(
        shown.events(),
        [ConnectionDialogEvent::Save(settings(
            "http://nas:9091/transmission/rpc",
            Some(Credentials {
                username: " admin".to_owned(),
                password: PASSWORD.to_owned(),
            })
        ))]
    );
}

#[gpui_kit::test]
fn enter_saves_and_escape_cancels(cx: &mut TestAppContext) {
    let current = settings("http://nas:9091/transmission/rpc", Some(admin()));
    let shown = Shown::open(Some(&current), None, cx);

    for field in [HOST, PATH, PASSWORD_FIELD] {
        shown.click(field, cx);
        shown.press("enter", cx);
        assert_eq!(
            shown.events(),
            [ConnectionDialogEvent::Save(current.clone())],
            "{field}"
        );
    }
    shown.press("escape", cx);
    assert_eq!(shown.events(), [ConnectionDialogEvent::Cancel]);
    assert!(shown.dialog_open(cx), "the main view closes the dialog");
    shown.click(CANCEL, cx);
    assert_eq!(shown.events(), [ConnectionDialogEvent::Cancel]);

    // Enter with a problem saves nothing, and shows it.
    shown.retype(PORT, "x", cx);
    shown.press("enter", cx);
    assert_eq!(shown.events(), []);
    assert_eq!(
        shown.label(PORT_PROBLEM, cx).as_deref(),
        Some("The port is a number from 1 to 65535")
    );
}

#[gpui_kit::test]
fn tab_goes_through_the_fields_then_the_buttons(cx: &mut TestAppContext) {
    let (check, _calls) = fake_check();
    let current = settings("http://nas:9091/transmission/rpc", None);
    let shown = Shown::open(Some(&current), Some(check), cx);
    assert!(shown.focused(HOST, cx));

    for id in [PORT, PATH, USERNAME, PASSWORD_FIELD, TEST, CANCEL, SAVE] {
        shown.press("tab", cx);
        assert!(shown.focused(id, cx), "{id}");
    }

    // Enter presses the button that has the focus.
    shown.press("shift-tab", cx);
    assert!(shown.focused(CANCEL, cx));
    shown.press("enter", cx);
    assert_eq!(shown.events(), [ConnectionDialogEvent::Cancel]);
}

#[gpui_kit::test]
fn a_pasted_address_is_spread_into_the_fields(cx: &mut TestAppContext) {
    let shown = Shown::open(None, None, cx);

    shown.paste(HOST, "http://admin:hunter2@[::1]:9092/rpc", cx);
    assert_eq!(
        shown.fields(cx),
        texts(["::1", "9092", "/rpc", "admin", PASSWORD])
    );
    assert_eq!(
        shown.label(ADDRESS, cx).as_deref(),
        Some("Address: http://[::1]:9092/rpc")
    );

    // `host:port` fills those two; an address without a port means 80, and a bare `/` leaves the
    // path as it is.
    shown.paste(HOST, "nas:9093", cx);
    assert_eq!(
        shown.fields(cx),
        texts(["nas", "9093", "/rpc", "admin", PASSWORD])
    );
    shown.paste(HOST, "http://seedbox/", cx);
    assert_eq!(
        shown.fields(cx),
        texts(["seedbox", "80", "/rpc", "admin", PASSWORD])
    );
    // A plain host is pasted as it is.
    shown.paste(HOST, "nas", cx);
    assert_eq!(
        shown.fields(cx),
        texts(["nas", "80", "/rpc", "admin", PASSWORD])
    );

    // Typed, an address is spread once the user leaves the field, not while it is typed.
    shown.retype(HOST, "box:9094", cx);
    assert_eq!(
        shown.fields(cx),
        texts(["box:9094", "80", "/rpc", "admin", PASSWORD])
    );
    shown.press("tab", cx);
    assert_eq!(
        shown.fields(cx),
        texts(["box", "9094", "/rpc", "admin", PASSWORD])
    );
    assert!(shown.focused(PORT, cx));

    // And before it is saved.
    shown.retype(HOST, "http://nas:9095/transmission/rpc", cx);
    shown.press("enter", cx);
    assert_eq!(
        shown.events(),
        [ConnectionDialogEvent::Save(settings(
            "http://nas:9095/transmission/rpc",
            Some(admin())
        ))]
    );
    assert_eq!(
        shown.fields(cx),
        texts(["nas", "9095", "/transmission/rpc", "admin", PASSWORD])
    );
}

#[gpui_kit::test]
fn test_shows_testing_then_the_outcome(cx: &mut TestAppContext) {
    let (check, calls) = fake_check();
    let current = settings("http://nas:9091/transmission/rpc", Some(admin()));
    let shown = Shown::open(Some(&current), Some(check), cx);
    assert!(!shown.disabled(TEST, cx));

    for (outcome, text) in [
        (
            CheckOutcome::Connected {
                version: "4.1.0 (abc123)".to_owned(),
            },
            "Connected: Transmission 4.1.0 (abc123)",
        ),
        (
            CheckOutcome::Failed(ConnectionError::Unauthorized),
            "The server rejected the credentials",
        ),
        (
            CheckOutcome::Unsupported(UnsupportedReason::Version("15".to_owned())),
            "Transmission 3.00 or newer required (server RPC 15)",
        ),
        (CheckOutcome::NotRun, "The test could not run"),
    ] {
        shown.click(TEST, cx);
        assert_eq!(shown.label(TEST, cx).as_deref(), Some("Testing…"));
        assert!(shown.disabled(TEST, cx), "one test at a time");
        assert_eq!(shown.label(RESULT, cx), None, "the last answer goes");
        let call = only_call(&calls);
        assert_eq!(call.settings, current, "the settings of the fields");

        call.answer.try_send(outcome).expect("the dialog waits");
        cx.run_until_parked();

        assert_eq!(shown.label(RESULT, cx).as_deref(), Some(text));
        assert_eq!(shown.label(TEST, cx).as_deref(), Some("Test"));
        assert!(!shown.disabled(TEST, cx));
    }
    assert_eq!(shown.events(), [], "a test asks the main view for nothing");
}

#[gpui_kit::test]
fn an_edit_drops_a_running_test(cx: &mut TestAppContext) {
    let (check, calls) = fake_check();
    let current = settings("http://nas:9091/transmission/rpc", None);
    let shown = Shown::open(Some(&current), Some(check), cx);

    shown.click(TEST, cx);
    let call = only_call(&calls);
    shown.click(USERNAME, cx);
    shown.type_text("x", cx);

    assert!(call.answer.is_closed(), "the dialog let go of the answer");
    assert!(call.answer.try_send(CheckOutcome::NotRun).is_err());
    cx.run_until_parked();
    assert_eq!(shown.label(RESULT, cx), None);
    assert_eq!(shown.label(TEST, cx).as_deref(), Some("Test"));
    assert!(!shown.disabled(TEST, cx));

    // An edit takes an answer shown away too.
    shown.click(TEST, cx);
    only_call(&calls)
        .answer
        .try_send(CheckOutcome::Failed(ConnectionError::Timeout))
        .expect("the dialog waits");
    cx.run_until_parked();
    assert_eq!(
        shown.label(RESULT, cx).as_deref(),
        Some("The server did not answer in time")
    );
    shown.click(PATH, cx);
    shown.type_text("x", cx);
    assert_eq!(shown.label(RESULT, cx), None);
}

#[gpui_kit::test]
fn test_with_problems_shows_them_and_checks_nothing(cx: &mut TestAppContext) {
    let (check, calls) = fake_check();
    let shown = Shown::open(None, Some(check), cx);

    shown.click(TEST, cx);

    assert!(calls.borrow().is_empty(), "nothing is checked");
    assert_eq!(
        shown.label(HOST_PROBLEM, cx).as_deref(),
        Some("Enter the server's name or IP address")
    );
    assert!(shown.focused(HOST, cx));
    assert_eq!(shown.label(TEST, cx).as_deref(), Some("Test"));
    assert_eq!(shown.label(RESULT, cx), None);
    assert_eq!(shown.events(), []);
}

#[gpui_kit::test]
fn the_save_error_is_shown(cx: &mut TestAppContext) {
    let current = settings("http://nas:9091/transmission/rpc", None);
    let shown = Shown::open(Some(&current), None, cx);
    shown.click(SAVE, cx);
    assert_eq!(shown.events(), [ConnectionDialogEvent::Save(current)]);

    cx.update(|cx| {
        shown.dialog.update(cx, |dialog, cx| {
            dialog.show_save_error("Permission denied (os error 13)".into(), cx);
        });
    });

    assert_eq!(
        shown.label(SAVE_ERROR, cx).as_deref(),
        Some("The settings could not be saved: Permission denied (os error 13)")
    );
    assert!(shown.dialog_open(cx));
    shown.click(PATH, cx);
    shown.type_text("x", cx);
    assert_eq!(shown.label(SAVE_ERROR, cx), None, "an edit takes it away");
}

#[gpui_kit::test]
fn a_held_enter_saves_once(cx: &mut TestAppContext) {
    let current = settings("http://nas:9091/transmission/rpc", None);
    let shown = Shown::open(Some(&current), None, cx);

    // In a field: the press saves, the repeats do not.
    shown.click(PATH, cx);
    shown.hold("enter", 5, cx);
    assert_eq!(
        shown.events(),
        [ConnectionDialogEvent::Save(current.clone())]
    );

    // On the focused Save, the last stop of Tab: its key-up presses it, once.
    shown.click(HOST, cx);
    for _ in 0..8 {
        if shown.focused(SAVE, cx) {
            break;
        }
        shown.press("tab", cx);
    }
    assert!(shown.focused(SAVE, cx));
    shown.hold("enter", 5, cx);
    assert_eq!(
        shown.events(),
        [ConnectionDialogEvent::Save(current.clone())]
    );

    // A new press after the key went up saves again.
    shown.click(HOST, cx);
    shown.press("enter", cx);
    assert_eq!(shown.events(), [ConnectionDialogEvent::Save(current)]);
}
