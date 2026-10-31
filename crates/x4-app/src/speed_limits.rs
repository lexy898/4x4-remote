//! The popover of the global speed limits, which the limits button of the status bar opens (see
//! the `status_bar` module): for each direction a switch that turns its limit on or off and a
//! field for the limit in KiB/s, and an Apply button.
//!
//! The form takes the daemon's limits when it opens and measures every edit against them:
//!
//! - each field starts with `x4-core`'s [`limit_prefill`] of its limit, the stored value whether
//!   the limit is on or off, and each switch with whether the limit is on. The download field has
//!   the focus, with its text selected, so that what is typed replaces it;
//! - typing a speed into a field switches its limit on, and clearing the field switches it off, so
//!   that what the form shows is what it sends. A switch clicked on while its field is empty asks
//!   for a speed;
//! - Enter in a field, or Apply, checks both directions with `x4-core`'s [`limit_edit`] (and
//!   [`limit_needs_value`]). If either cannot be applied, the line under its field says why and
//!   nothing is sent; otherwise the form hands both edits to the main view
//!   ([`LimitsApplied`]), which sends each one that changes something and closes the popover.
//!   An edit of the text or the switch takes the line away again.
//!
//! While the daemon is not connected, or its settings are not known, Apply is disabled and Enter
//! applies nothing; the popover stays open as it is. The main view closes the popover when the
//! focus leaves the form, as Tab, Shift+Tab or Ctrl+F take it out: the list's keys must not act
//! behind a popover that looks active. A switch to another app does not close it: the window's
//! focus goes away and comes back, and what was typed is still there. The form is the key context
//! `SpeedLimits`, where the keymap binds the details panel's keys to nothing, so that they change
//! nothing behind the popover.
//!
//! Like the other keys, the key that opened the popover acts only when it is pressed, not when it
//! repeats while held. An Enter held on the limits button opens the popover, and its repeats must
//! not apply what it was filled with; a Space held there must not type over the selected value.
//! So the repeats of that key do nothing in the form until it goes up, and Enter in a field
//! applies only when it is a new press.
//!
//! The fields are plain text fields, not number inputs: gpui-component's number input names its
//! buttons in English, outside our messages, and `x4-core` decides what the text means.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, MouseButton,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, TestSupportExt as _, Window, div, px,
};
use x4_core::{
    Direction, LimitEdit, LimitError, SpeedControls, SpeedLimit, limit_edit, limit_needs_value,
    limit_prefill,
};

use crate::i18n::{limit_feedback_text, t};
use crate::key_repeats::{KeyRepeats, same_key};
use crate::keymap::SPEED_LIMITS_CONTEXT;

/// The width of the form inside the popover.
const FORM_WIDTH: f32 = 320.;
/// The width of a limit's field, room for seven digits and the unit.
const FIELD_WIDTH: f32 = 132.;

/// Why a limit cannot be applied, as the line under its field says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LimitFeedback {
    /// The text is not a limit.
    Invalid(LimitError),
    /// The switch is on, but the field has no speed.
    NeedsValue,
}

/// What Apply asks the main view to send: the edit of each limit, as `x4-core` decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LimitsApplied {
    /// The edit of the download limit.
    pub(crate) down: LimitEdit,
    /// The edit of the upload limit.
    pub(crate) up: LimitEdit,
}

/// The daemon's limits as the form was filled from them, which its edits are measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Opened {
    down: SpeedLimit,
    up: SpeedLimit,
}

/// One direction's switch and field, and what its line says.
struct LimitField {
    /// Which limit it is.
    direction: Direction,
    /// The text of the field.
    input: Entity<InputState>,
    /// Whether the switch is on.
    enabled: bool,
    /// Why the limit could not be applied, until it is edited.
    feedback: Option<LimitFeedback>,
}

impl LimitField {
    /// Fills the field and the switch from `limit`, and takes its line away.
    fn fill(&mut self, limit: SpeedLimit, window: &mut Window, cx: &mut App) {
        self.enabled = limit.enabled;
        self.feedback = None;
        self.input.update(cx, |input, cx| {
            input.set_value(limit_prefill(limit), window, cx);
        });
    }

    /// What applying the field does to `current`, the limit it was filled from.
    fn check(&self, current: SpeedLimit, cx: &App) -> Result<LimitEdit, LimitFeedback> {
        let text = self.input.read(cx).value();
        if limit_needs_value(current, &text, self.enabled) {
            return Err(LimitFeedback::NeedsValue);
        }
        limit_edit(current, &text, self.enabled).map_err(LimitFeedback::Invalid)
    }

    /// The element ids of the field, its switch and its line, and their names.
    fn ids(&self) -> FieldIds {
        match self.direction {
            Direction::Down => FieldIds {
                field: "limit-down",
                switch: "limit-down-enabled",
                feedback: "limit-down-feedback",
                field_name: t!("limit-down-field").into(),
                switch_name: t!("limit-down-switch").into(),
            },
            Direction::Up => FieldIds {
                field: "limit-up",
                switch: "limit-up-enabled",
                feedback: "limit-up-feedback",
                field_name: t!("limit-up-field").into(),
                switch_name: t!("limit-up-switch").into(),
            },
        }
    }
}

/// The ids and names of a direction's elements.
struct FieldIds {
    field: &'static str,
    switch: &'static str,
    feedback: &'static str,
    field_name: SharedString,
    switch_name: SharedString,
}

/// The form of the speed limits popover.
pub(crate) struct SpeedLimits {
    down: LimitField,
    up: LimitField,
    /// The limits the form was filled from; `None` before it was first opened.
    opened: Option<Opened>,
    /// Whether Apply can send: the daemon is connected and its settings are known.
    can_apply: bool,
    /// The focus of the whole form, which the main view watches to close the popover when the
    /// focus leaves it.
    focus: FocusHandle,
    /// The key that was held down when the form was filled, which opened the popover, until a
    /// key goes up in the form; its repeats do nothing here.
    opener: Option<Keystroke>,
    /// The key held down now, shared with the main view, so that a held key acts once.
    key_repeats: Rc<RefCell<KeyRepeats>>,
    /// The fields' events.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LimitsApplied> for SpeedLimits {}

impl SpeedLimits {
    /// An empty form, which [`SpeedLimits::fill`] fills when the popover opens. `key_repeats` is
    /// the main view's: it tells whether the last key-down repeated a held key.
    pub(crate) fn new(
        key_repeats: Rc<RefCell<KeyRepeats>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let down = cx.new(|cx| InputState::new(window, cx));
        let up = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&down, window, |form, _, event, _, cx| {
                form.on_field_event(Direction::Down, event, cx);
            }),
            cx.subscribe_in(&up, window, |form, _, event, _, cx| {
                form.on_field_event(Direction::Up, event, cx);
            }),
        ];
        let field = |direction, input| LimitField {
            direction,
            input,
            enabled: false,
            feedback: None,
        };
        Self {
            down: field(Direction::Down, down),
            up: field(Direction::Up, up),
            opened: None,
            can_apply: false,
            focus: cx.focus_handle(),
            opener: None,
            key_repeats,
            _subscriptions: subscriptions,
        }
    }

    /// The focus of the download field, which gets the focus when the popover opens.
    pub(crate) fn first_field(&self, cx: &App) -> FocusHandle {
        self.down.input.focus_handle(cx)
    }

    /// Fills the form from the daemon's limits in `speed`, which its edits are measured against
    /// from now on, and selects the download field's text. The key held down now opened the
    /// popover; its repeats are held back.
    pub(crate) fn fill(
        &mut self,
        speed: SpeedControls,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.opener = self.key_repeats.borrow().held().cloned();
        self.opened = Some(Opened {
            down: speed.limit_down,
            up: speed.limit_up,
        });
        self.down.fill(speed.limit_down, window, cx);
        self.up.fill(speed.limit_up, window, cx);
        self.down
            .input
            .update(cx, |input, cx| input.select_all(window, cx));
        cx.notify();
    }

    /// Enables Apply while `can_apply`: the daemon is connected and its settings are known.
    pub(crate) fn set_can_apply(&mut self, can_apply: bool, cx: &mut Context<'_, Self>) {
        if self.can_apply != can_apply {
            self.can_apply = can_apply;
            cx.notify();
        }
    }

    fn field_mut(&mut self, direction: Direction) -> &mut LimitField {
        match direction {
            Direction::Down => &mut self.down,
            Direction::Up => &mut self.up,
        }
    }

    /// Follows the field of `direction`: an edit takes its line away, and switches the limit on
    /// when the field has a speed and off when it is empty; Enter applies.
    fn on_field_event(
        &mut self,
        direction: Direction,
        event: &InputEvent,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            InputEvent::Change => {
                let field = self.field_mut(direction);
                field.feedback = None;
                field.enabled = !field.input.read(cx).value().trim().is_empty();
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                if self.key_repeats.borrow().repeating() {
                    tracing::trace!("a held Enter repeats; the speed limits are not applied");
                } else {
                    self.apply(cx);
                }
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    /// Holds back a key-down in the form that repeats the key which opened the popover, before
    /// the fields' capture listeners see it, so that no field types it. The key bindings are not
    /// held back: gpui runs them before the capture listeners.
    fn hold_back_opener(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let repeats_opener = self.key_repeats.borrow().repeating()
            && self
                .opener
                .as_ref()
                .is_some_and(|opener| same_key(opener, &event.keystroke));
        if repeats_opener {
            tracing::trace!("the key that opened the speed limits repeats; held back");
            cx.stop_propagation();
        }
    }

    /// Handles a click on the switch of `direction`, which asks for `enabled`.
    fn switch(&mut self, direction: Direction, enabled: bool, cx: &mut Context<'_, Self>) {
        let field = self.field_mut(direction);
        field.enabled = enabled;
        field.feedback = None;
        cx.notify();
    }

    /// Checks both limits; hands their edits to the main view when both can be applied, and says
    /// under each field that cannot why not. Applies nothing while Apply is disabled.
    ///
    /// Enter in a field comes here only when it is a new press of the key. The Apply button needs
    /// no such check: a click is no key, and the button, pressed with the keyboard, acts when the
    /// key goes up.
    fn apply(&mut self, cx: &mut Context<'_, Self>) {
        if !self.can_apply {
            tracing::debug!("the speed limits cannot be changed now; nothing applied");
            return;
        }
        let Some(opened) = self.opened else {
            tracing::debug!("the speed limits form was never filled; nothing applied");
            return;
        };
        let down = self.down.check(opened.down, cx);
        let up = self.up.check(opened.up, cx);
        self.down.feedback = down.err();
        self.up.feedback = up.err();
        cx.notify();
        match (down, up) {
            (Ok(down), Ok(up)) => cx.emit(LimitsApplied { down, up }),
            (down, up) => tracing::debug!(
                down = ?down.err(),
                up = ?up.err(),
                "a speed limit cannot be applied; nothing sent"
            ),
        }
    }

    /// The switch, the field and the line of `field`.
    fn row(field: &LimitField, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let ids = field.ids();
        let direction = field.direction;
        let feedback = field.feedback.map(|feedback| {
            let color = match feedback {
                LimitFeedback::Invalid(_) => cx.theme().danger,
                LimitFeedback::NeedsValue => cx.theme().warning,
            };
            let text = limit_feedback_text(feedback);
            div()
                .id(ids.feedback)
                .text_xs()
                .text_color(color)
                .child(text.clone())
                .aria_label(text)
                .test_support()
        });
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        Checkbox::new(ids.switch)
                            .small()
                            .label(ids.switch_name)
                            .checked(field.enabled)
                            .on_click(cx.listener(move |form, enabled: &bool, _, cx| {
                                form.switch(direction, *enabled, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div().w(px(FIELD_WIDTH)).flex_none().child(
                            Input::new(&field.input)
                                .id(ids.field)
                                .small()
                                .aria_label(ids.field_name)
                                .suffix(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(t!("unit-kib-per-s")),
                                ),
                        ),
                    ),
            )
            .children(feedback)
    }
}

impl Focusable for SpeedLimits {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SpeedLimits {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        v_flex()
            .id("speed-limits-form")
            .key_context(SPEED_LIMITS_CONTEXT)
            .track_focus(&self.focus)
            // A press on blank space would otherwise move the focus from the field to the form,
            // where typing and Enter do nothing.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            // In the capture phase, before the fields: the key-down of a held key repeats before
            // any key-up, and any key-up ends the hold.
            .capture_key_down(cx.listener(Self::hold_back_opener))
            .capture_key_up(cx.listener(|form, _: &KeyUpEvent, _, _| form.opener = None))
            .w(px(FORM_WIDTH))
            .gap_3()
            .text_sm()
            .child(Self::row(&self.down, cx))
            .child(Self::row(&self.up, cx))
            .child(
                h_flex().justify_end().child(
                    Button::new("limits-apply")
                        .primary()
                        .small()
                        .label(t!("limit-apply"))
                        .disabled(!self.can_apply)
                        .on_click(cx.listener(|form, _, _, cx| form.apply(cx))),
                ),
            )
    }
}
