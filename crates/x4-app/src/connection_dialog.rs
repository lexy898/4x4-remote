//! The Connection dialog: a gpui-component `Dialog` in the main window, whose whole surface is one
//! [`ConnectionDialog`], where the user sets the daemon 4x4 talks to.
//!
//! The dialog shows the fields of x4-core's [`ConnectionForm`]: Host, Port, Path, User name and
//! Password (masked, with the eye that shows it), filled from the settings in use, or with
//! Transmission's port and path when no server is set. Under them it shows the address they make
//! while they make one ([`ConnectionSettings::url`], which never holds the credentials), and
//! after a Save or a Test what is wrong with each field, again at every edit until the dialog
//! closes. Save and Test with a problem show it and put the focus on the first wrong field;
//! neither sends anything then.
//!
//! A full address in the host field is spread over the fields by [`ConnectionForm::spread`]:
//! pasted, at once; typed, once the user leaves the field, saves or tests. Spreading at each key
//! would take `http://n` for the host `n` on port 80 before the rest is typed.
//!
//! Test asks the window's [`ServerCheck`] about the settings of the fields once: its button says
//! "Testing…" until the answer comes, and the line under the fields then says what the server
//! answered. An edit takes the line away and lets go of the answer of a test still running, so an
//! answer is only ever shown for the fields as they are. A window without a check has Test
//! disabled.
//!
//! The dialog writes no file and touches no service: Save hands the settings to the main view
//! ([`ConnectionDialogEvent::Save`]), which saves and switches, and shows here why it could not
//! save ([`ConnectionDialog::show_save_error`]). Cancel and Escape ask the main view to close the
//! dialog ([`ConnectionDialogEvent::Cancel`]); the dialog never closes itself, and a click beside
//! it does nothing, so that nothing typed is lost by a slip. Enter in a field saves. The dialog's
//! own Enter is bound to nothing in its key context `ConnectionDialog`, so that Enter on a focused
//! button presses it, and its confirm closes nothing. Tab goes from the host through the fields,
//! then the buttons.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogFooter, DialogTitle};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    TestSupportExt as _, WeakEntity, Window, div, px, rems,
};
use x4_core::{CheckOutcome, ConnectionForm, ConnectionSettings, FormProblems};

use crate::connection::ServerCheck;
use crate::i18n::{
    connection_address_text, connection_save_failed_text, connection_test_text, host_problem_text,
    path_problem_text, port_problem_text, t, username_problem_text,
};
use crate::keymap::CONNECTION_DIALOG_CONTEXT;

/// The width of the dialog.
const DIALOG_WIDTH: f32 = 520.;
/// The width of the port field: room for five digits.
const PORT_WIDTH: f32 = 112.;
/// The height of the line of the address and of the line of a test's answer, in rems: kept while
/// they say nothing, so that the dialog does not jump as they come and go.
const LINE: f32 = 1.25;

/// What the dialog asks the main view for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectionDialogEvent {
    /// Save these settings and switch to their server, or say why they could not be saved
    /// ([`ConnectionDialog::show_save_error`]); close the dialog once they are.
    Save(ConnectionSettings),
    /// Close the dialog and change nothing.
    Cancel,
}

/// A field of the dialog, in the order Tab goes through them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Host,
    Port,
    Path,
    Username,
    Password,
}

impl Field {
    /// The element id of the field.
    const fn id(self) -> &'static str {
        match self {
            Self::Host => "connection-host",
            Self::Port => "connection-port",
            Self::Path => "connection-path",
            Self::Username => "connection-username",
            Self::Password => "connection-password",
        }
    }

    /// The element id of the line under the field that says what is wrong with it.
    const fn problem_id(self) -> &'static str {
        match self {
            Self::Host => "connection-host-problem",
            Self::Port => "connection-port-problem",
            Self::Path => "connection-path-problem",
            Self::Username => "connection-username-problem",
            Self::Password => "connection-password-problem",
        }
    }

    /// The name of the field, over it and for screen readers.
    fn name(self) -> SharedString {
        match self {
            Self::Host => t!("connection-host"),
            Self::Port => t!("connection-port"),
            Self::Path => t!("connection-path"),
            Self::Username => t!("connection-username"),
            Self::Password => t!("connection-password"),
        }
        .into()
    }

    /// The first field, in the order of Tab, that `problems` finds fault with.
    fn first_wrong(problems: FormProblems) -> Option<Self> {
        if problems.host.is_some() {
            Some(Self::Host)
        } else if problems.port.is_some() {
            Some(Self::Port)
        } else if problems.path.is_some() {
            Some(Self::Path)
        } else if problems.username.is_some() {
            Some(Self::Username)
        } else {
            None
        }
    }
}

/// The content of the Connection dialog.
pub(crate) struct ConnectionDialog {
    /// The text of the host field.
    host: Entity<InputState>,
    /// The text of the port field.
    port: Entity<InputState>,
    /// The text of the path field.
    path: Entity<InputState>,
    /// The text of the user name field.
    username: Entity<InputState>,
    /// The text of the password field, drawn masked.
    password: Entity<InputState>,
    /// Asks a server whether 4x4 can use it; `None` in a window that cannot, whose Test is
    /// disabled.
    check: Option<ServerCheck>,
    /// What the fields make, as of the last edit.
    checked: Result<ConnectionSettings, FormProblems>,
    /// Whether Save or Test was tried: from then on the problems show.
    attempted: bool,
    /// The test running; dropping it lets go of its answer.
    testing: Option<Task<()>>,
    /// What the last test found, until the next edit.
    tested: Option<CheckOutcome>,
    /// Why the last Save could not be carried out, until the next edit.
    save_error: Option<SharedString>,
    /// What gets the focus at the next build of the dialog: the host field, once it opens.
    pending_focus: Option<FocusHandle>,
    /// The fields' events.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConnectionDialogEvent> for ConnectionDialog {}

impl ConnectionDialog {
    /// The dialog for `settings`, the server in use, or for a first one when no server is set;
    /// Test asks `check`. [`open`] shows it.
    pub(crate) fn new(
        settings: Option<&ConnectionSettings>,
        check: Option<ServerCheck>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let form = ConnectionForm::of(settings);
        let host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(SharedString::from(t!("connection-host-placeholder")))
                .default_value(form.host.clone())
        });
        let port = cx.new(|cx| InputState::new(window, cx).default_value(form.port.clone()));
        let path = cx.new(|cx| InputState::new(window, cx).default_value(form.path.clone()));
        let username =
            cx.new(|cx| InputState::new(window, cx).default_value(form.username.clone()));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .default_value(form.password.clone())
        });
        let mut subscriptions = vec![cx.subscribe_in(
            &host,
            window,
            |dialog, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => dialog.edited(cx),
                InputEvent::PressEnter { .. } => dialog.save(window, cx),
                // An address typed in is spread once the user is done with it.
                InputEvent::Blur => dialog.spread_host(window, cx),
                InputEvent::Focus => {}
            },
        )];
        for input in [&port, &path, &username, &password] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |dialog, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => dialog.edited(cx),
                    InputEvent::PressEnter { .. } => dialog.save(window, cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ));
        }
        let pending_focus = Some(host.focus_handle(cx));
        Self {
            host,
            port,
            path,
            username,
            password,
            check,
            checked: form.check(),
            attempted: false,
            testing: None,
            tested: None,
            save_error: None,
            pending_focus,
            _subscriptions: subscriptions,
        }
    }

    /// Shows under the fields that the settings could not be saved, for `reason`, until the next
    /// edit. The dialog stays open, with the fields as they are.
    pub(crate) fn show_save_error(&mut self, reason: SharedString, cx: &mut Context<'_, Self>) {
        self.save_error = Some(reason);
        cx.notify();
    }

    /// The focus to give at the next build of the dialog, once.
    fn take_pending_focus(&mut self, _: &mut Context<'_, Self>) -> Option<FocusHandle> {
        self.pending_focus.take()
    }

    /// The text field of `field`.
    fn input(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Host => &self.host,
            Field::Port => &self.port,
            Field::Path => &self.path,
            Field::Username => &self.username,
            Field::Password => &self.password,
        }
    }

    /// The text of the fields.
    fn form(&self, cx: &App) -> ConnectionForm {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        ConnectionForm {
            host: text(&self.host),
            port: text(&self.port),
            path: text(&self.path),
            username: text(&self.username),
            password: text(&self.password),
        }
    }

    /// Follows an edit of any field: the fields are checked again, and what was said about them
    /// before goes, the answer of a test still running with it.
    fn edited(&mut self, cx: &mut Context<'_, Self>) {
        self.testing = None;
        self.tested = None;
        self.save_error = None;
        self.checked = self.form(cx).check();
        cx.notify();
    }

    /// Puts the text of `form` into the fields that differ from it, as an edit.
    fn fill(&mut self, form: &ConnectionForm, window: &mut Window, cx: &mut Context<'_, Self>) {
        let current = self.form(cx);
        for (input, now, wanted) in [
            (&self.host, &current.host, &form.host),
            (&self.port, &current.port, &form.port),
            (&self.path, &current.path, &form.path),
            (&self.username, &current.username, &form.username),
            (&self.password, &current.password, &form.password),
        ] {
            if now != wanted {
                // Setting a value sends no change of its own, so this never comes back here.
                input.update(cx, |input, cx| input.set_value(wanted.clone(), window, cx));
            }
        }
        self.edited(cx);
    }

    /// Spreads a full address in the host field over the fields, if it holds one.
    fn spread_host(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // The host left by a spread is bare, which spreads no further.
        if let Some(spread) = self.form(cx).spread() {
            self.fill(&spread, window, cx);
        }
    }

    /// Takes `text`, pasted into the host field: a full address is spread over the fields, in
    /// place of the whole host, and the paste goes no further (`true`); anything else is pasted
    /// as text.
    fn paste_into_host(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let pasted = ConnectionForm {
            host: text,
            ..self.form(cx)
        };
        let Some(spread) = pasted.spread() else {
            return false;
        };
        self.fill(&spread, window, cx);
        true
    }

    /// Checks the fields for a Save or a Test: their settings, or `None` when a field is wrong,
    /// which then shows its problem and takes the focus. From the first try on, the problems
    /// follow the fields.
    fn attempt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<ConnectionSettings> {
        self.spread_host(window, cx);
        self.attempted = true;
        self.checked = self.form(cx).check();
        cx.notify();
        match &self.checked {
            Ok(settings) => Some(settings.clone()),
            Err(problems) => {
                if let Some(field) = Field::first_wrong(*problems) {
                    self.input(field).focus_handle(cx).focus(window, cx);
                }
                tracing::debug!(?problems, "the connection form has problems");
                None
            }
        }
    }

    /// Hands the settings of the fields to the main view, unless a field is wrong.
    fn save(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(settings) = self.attempt(window, cx) {
            cx.emit(ConnectionDialogEvent::Save(settings));
        }
    }

    /// Asks the server of the fields whether 4x4 can use it, unless a field is wrong or a test
    /// runs already. The answer is shown when it comes, unless the fields were edited meanwhile.
    fn test(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.testing.is_some() {
            return;
        }
        let Some(check) = self.check.clone() else {
            return;
        };
        let Some(settings) = self.attempt(window, cx) else {
            return;
        };
        let answer = check(&settings, cx);
        self.tested = None;
        self.testing = Some(cx.spawn(async move |dialog, cx| {
            let outcome = answer.await;
            if let Err(error) = dialog.update(cx, |dialog, cx| dialog.answered(outcome, cx)) {
                tracing::debug!(%error, "the Connection dialog closed before the test answered");
            }
        }));
        cx.notify();
    }

    /// Shows what the test found.
    fn answered(&mut self, outcome: CheckOutcome, cx: &mut Context<'_, Self>) {
        self.testing = None;
        self.tested = Some(outcome);
        cx.notify();
    }

    /// What is wrong with `field`, once Save or Test was tried.
    fn problem(&self, field: Field) -> Option<SharedString> {
        if !self.attempted {
            return None;
        }
        let problems = self.checked.as_ref().err()?;
        match field {
            Field::Host => problems.host.map(host_problem_text),
            Field::Port => problems.port.map(port_problem_text),
            Field::Path => problems.path.map(path_problem_text),
            Field::Username => problems.username.map(username_problem_text),
            Field::Password => None,
        }
    }

    /// The name of `field`, the field, and the line of its problem, if any. A full address pasted
    /// into the host field is spread, and the password field has the eye that shows its text.
    fn field(&self, field: Field, cx: &mut Context<'_, Self>) -> Div {
        let name = field.name();
        let dialog = cx.weak_entity();
        let input = Input::new(self.input(field))
            .id(field.id())
            .aria_label(name.clone())
            .when(field == Field::Host, |input| {
                input.on_paste(move |item, window, cx| pasted_into_host(&dialog, item, window, cx))
            })
            .when(field == Field::Password, Input::mask_toggle);
        let problem = self.problem(field).map(|text| {
            text_line(field.problem_id(), &text)
                .text_xs()
                .text_color(cx.theme().danger)
                .test_support()
        });
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(name),
            )
            .child(input)
            .children(problem)
    }

    /// The fields: host and port on a line, the path, then the credentials and their hint.
    fn fields(&self, cx: &mut Context<'_, Self>) -> Div {
        let hint = SharedString::from(t!("connection-credentials-hint"));
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(self.field(Field::Host, cx).flex_1().min_w_0())
                    .child(self.field(Field::Port, cx).w(px(PORT_WIDTH)).flex_none()),
            )
            .child(self.field(Field::Path, cx))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(self.field(Field::Username, cx).flex_1().min_w_0())
                            .child(self.field(Field::Password, cx).flex_1().min_w_0()),
                    )
                    .child(
                        text_line("connection-credentials-hint", &hint)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .test_support(),
                    ),
            )
    }

    /// The address the fields make, the answer of the last test, and why the last Save failed.
    fn lines(&self, cx: &mut Context<'_, Self>) -> Div {
        let theme = cx.theme();
        let address = self.checked.as_ref().ok().map(|settings| {
            text_line(
                "connection-address",
                &connection_address_text(&settings.url()),
            )
            .truncate()
            .text_color(theme.muted_foreground)
            .test_support()
        });
        let tested = self.tested.as_ref().map(|outcome| {
            let color = match outcome {
                CheckOutcome::Connected { .. } => theme.success,
                CheckOutcome::Failed(_) | CheckOutcome::Unsupported(_) | CheckOutcome::NotRun => {
                    theme.danger
                }
            };
            text_line("connection-test-result", &connection_test_text(outcome))
                .text_color(color)
                .test_support()
        });
        let save_error = self.save_error.as_ref().map(|reason| {
            text_line(
                "connection-save-error",
                &connection_save_failed_text(reason),
            )
            .text_color(theme.danger)
            .test_support()
        });
        v_flex()
            .gap_1()
            .child(
                div()
                    .min_h(rems(LINE))
                    .line_height(rems(LINE))
                    .children(address),
            )
            .child(
                div()
                    .min_h(rems(LINE))
                    .line_height(rems(LINE))
                    .children(tested),
            )
            .children(save_error)
    }

    /// Test on the left; Cancel and Save on the right. Test is disabled while a test runs and in
    /// a window that cannot test; Save always works.
    fn buttons(&self, cx: &mut Context<'_, Self>) -> DialogFooter {
        let test = if self.testing.is_some() {
            t!("connection-testing")
        } else {
            t!("connection-test")
        };
        DialogFooter::new()
            .child(
                Button::new("connection-test")
                    .label(test)
                    .disabled(self.check.is_none() || self.testing.is_some())
                    .on_click(cx.listener(|dialog, _, window, cx| dialog.test(window, cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("connection-cancel")
                    .label(t!("connection-cancel"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ConnectionDialogEvent::Cancel))),
            )
            .child(
                Button::new("connection-save")
                    .primary()
                    .label(t!("connection-save"))
                    .on_click(cx.listener(|dialog, _, window, cx| dialog.save(window, cx))),
            )
    }
}

/// Hands the text of `item`, pasted into the host field, to `dialog`; whether the dialog took the
/// paste.
fn pasted_into_host(
    dialog: &WeakEntity<ConnectionDialog>,
    item: &ClipboardItem,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(text) = item.text() else {
        return false;
    };
    dialog
        .update(cx, |dialog, cx| dialog.paste_into_host(text, window, cx))
        .unwrap_or_else(|error| {
            tracing::debug!(%error, "a paste came after the Connection dialog closed");
            false
        })
}

/// A line of the dialog's text, `id`, which screen readers read as its name.
fn text_line(id: &'static str, text: &SharedString) -> Stateful<Div> {
    div().id(id).aria_label(text.clone()).child(text.clone())
}

impl Render for ConnectionDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let title = SharedString::from(t!("connection-title"));
        v_flex()
            .id("connection-dialog")
            .key_context(CONNECTION_DIALOG_CONTEXT)
            .w_full()
            .p_4()
            .gap_3()
            .text_sm()
            .child(DialogTitle::new().child(text_line("connection-title", &title).test_support()))
            .child(self.fields(cx))
            .child(self.lines(cx))
            .child(self.buttons(cx))
            .test_support()
    }
}

/// Opens the Connection dialog of `dialog` in `window`. Its content fills the whole surface,
/// without the surface's own title, padding, close button or footer. The builder runs at every
/// frame, so it only lays the entity out, and gives the host field the focus, once.
///
/// Escape asks the main view to close the dialog, as Cancel does; once the content is gone it
/// closes the dialog itself. A click beside the dialog does nothing.
pub(crate) fn open(dialog: Entity<ConnectionDialog>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |surface, window, cx| {
        if let Some(focus) = dialog.update(cx, ConnectionDialog::take_pending_focus) {
            focus.focus(window, cx);
        }
        let escape = dialog.downgrade();
        surface
            .w(px(DIALOG_WIDTH))
            .p_0()
            .close_button(false)
            .overlay_closable(false)
            // The content binds Enter to nothing; should the dialog's confirm come anyway, it
            // closes nothing.
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, _, cx| {
                escape
                    .update(cx, |_, cx| cx.emit(ConnectionDialogEvent::Cancel))
                    .is_err()
            })
            .child(dialog.clone())
    });
}

#[cfg(test)]
mod tests;
