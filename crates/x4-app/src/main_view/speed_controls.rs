//! The turtle and the speed limits, as the main view carries them out: from their keys (Ctrl+T
//! and Ctrl+L, bound in [`keymap`](crate::keymap)), from the turtle and the limits button of the
//! status bar, and from the popover's Apply.
//!
//! Each goes out as an action through the same path as the actions on torrents: kept until the
//! daemon answers, not sent again while the same one waits, and its failure shown in a
//! notification of its own kind. They work only while the daemon is connected and its settings
//! are known, which the view checks again when the user acts: a click that came before its
//! button was drawn disabled sends nothing.
//!
//! The popover is filled when it opens, from the daemon's limits then. Apply sends each limit
//! whose edit changes something, then closes the popover; Escape and a click beside it close it
//! and send nothing. Either way the keys go back to the list. When the focus leaves the popover
//! (Tab, Shift+Tab, Ctrl+F), it closes too, sends nothing, and the focus stays where it went. A
//! switch to another app takes the window's focus away too, but the popover stays open as it is.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{Context, Div, Entity, Focusable as _, InteractiveElement as _, Window};
use x4_core::Direction;

use super::MainView;
use crate::actions::ActionKind;
use crate::key_repeats::{KeyRepeats, once_per_press_of};
use crate::keymap::{EditSpeedLimits, ToggleAltSpeed};
use crate::speed_limits::{LimitsApplied, SpeedLimits};

impl MainView {
    /// Switches the alternative speed limits the other way from how the daemon has them now, if
    /// they can be changed now; otherwise sends nothing.
    fn toggle_alt_speed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(speed) = self.status.changeable_speed() else {
            tracing::debug!("the speed settings cannot be changed now; the turtle sent nothing");
            return;
        };
        self.send(
            ActionKind::AltSpeed(!speed.alt_speed),
            Vec::new(),
            window,
            cx,
        );
    }

    /// Handles a click on the turtle: switches it, then hands the keys back to the table, as a
    /// toolbar button does.
    pub(crate) fn press_turtle(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.toggle_alt_speed(window, cx);
        window.focus(&self.table.focus_handle(cx), cx);
    }

    /// Opens the popover of the speed limits, filled from the daemon's limits now, unless it is
    /// open already or the limits cannot be changed now.
    pub(crate) fn edit_speed_limits(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.limits_open {
            return;
        }
        let Some(speed) = self.status.changeable_speed() else {
            tracing::debug!("the speed settings cannot be changed now; no popover opens");
            return;
        };
        tracing::debug!("editing the speed limits");
        self.speed_limits
            .update(cx, |form, cx| form.fill(speed, window, cx));
        self.limits_open = true;
        cx.notify();
    }

    /// Follows a click on the limits button, which opened the popover: fills it, or, if the limits
    /// cannot be changed now, closes it again (drawing the view closes it) and hands the keys back
    /// to the table, from the field the popover gave them to.
    pub(crate) fn limits_button_opened(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.edit_speed_limits(window, cx);
        if !self.limits_open {
            window.focus(&self.table.focus_handle(cx), cx);
            cx.notify();
        }
    }

    /// Closes the popover of the speed limits, if it is open, and hands the keys back to the table.
    pub(crate) fn close_speed_limits(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.limits_open = false;
        window.focus(&self.table.focus_handle(cx), cx);
        cx.notify();
    }

    /// The focus left the popover of the speed limits, as Tab, Shift+Tab or Ctrl+F take it out:
    /// closes the popover and sends nothing. The focus stays where it went.
    pub(super) fn limits_lost_focus(&mut self, cx: &mut Context<'_, Self>) {
        if self.limits_open {
            tracing::debug!("the focus left the speed limits; they close");
            self.limits_open = false;
            cx.notify();
        }
    }

    /// Handles Apply in the popover: sends each limit whose edit changes something, then closes
    /// the popover. While the daemon is not connected it sends nothing, and the popover stays.
    pub(super) fn on_limits_applied(
        &mut self,
        _: &Entity<SpeedLimits>,
        applied: &LimitsApplied,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.status.changeable_speed().is_none() {
            tracing::debug!("the speed settings cannot be changed now; the limits are not sent");
            return;
        }
        for (direction, edit) in [(Direction::Down, applied.down), (Direction::Up, applied.up)] {
            if let Some(kind) = ActionKind::speed_limit(direction, edit) {
                self.send(kind, Vec::new(), window, cx);
            }
        }
        self.close_speed_limits(window, cx);
    }

    /// Tells the popover whether Apply can send now.
    pub(super) fn show_speed_status(&self, cx: &mut Context<'_, Self>) {
        let can_apply = self.status.changeable_speed().is_some();
        self.speed_limits
            .update(cx, |form, cx| form.set_can_apply(can_apply, cx));
    }
}

/// `root`, the main view's root element, with the handlers of Ctrl+T and Ctrl+L, which run once
/// per press of their key, as `repeats`, the view's record of the key held, tells.
pub(super) fn on_speed_actions(
    root: Div,
    repeats: &Rc<RefCell<KeyRepeats>>,
    cx: &Context<'_, MainView>,
) -> Div {
    root.on_action(once_per_press_of::<_, ToggleAltSpeed>(
        repeats,
        cx,
        MainView::toggle_alt_speed,
    ))
    .on_action(once_per_press_of::<_, EditSpeedLimits>(
        repeats,
        cx,
        MainView::edit_speed_limits,
    ))
}
