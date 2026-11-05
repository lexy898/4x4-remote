//! The status bar at the bottom of the main window: how the connection is doing, notices about
//! the config file and the saved state, the turtle and the speed limits, the free space and the
//! total speeds.
//!
//! [`StatusModel`] from `x4-core` decides the values, down to which state the connection
//! indicator shows ([`Indicator`]); this module words them and lays them out. A click on the
//! indicator opens the Connection dialog, as the toolbar's Connection button and Ctrl+, do.
//!
//! The turtle switches the daemon's alternative speed limits; it is drawn selected while they
//! apply, and its tooltip gives them. The limits button shows the normal limits and opens the
//! popover that sets them (the `speed_limits` module). Both are disabled until the daemon's
//! settings are known, and while the daemon is not connected; a click and their keys (Ctrl+T and
//! Ctrl+L, named in the tooltips) go through the main view, which checks that again.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, Theme, h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div, px,
};
use x4_core::{Indicator, StatusModel};

use crate::app::ConfigProblem;
use crate::i18n::{
    alt_speed_tooltip_text, config_problem_text, connected_to_text, connection_reason_detail_text,
    connection_text, download_rate_text, free_space_text, limits_button_text, t, upload_rate_text,
};
use crate::keymap::{EditSpeedLimits, MAIN_CONTEXT, ToggleAltSpeed};
use crate::main_view::MainView;
use crate::speed_limits::SpeedLimits;
use crate::theme::{STALE_OPACITY, alt_speed_color, connection_color, ghost_hover, tabular_digits};

/// The diameter of the connection dot.
const DOT_SIZE: f32 = 8.;

/// The tooltip of the connection indicator, when there is more to say than its text: what the
/// dot alone means, with the name of `server` (`host:port`) when the window knows it, or why the
/// last try to reconnect failed.
fn connection_tooltip(indicator: &Indicator, server: Option<&str>) -> Option<SharedString> {
    match indicator {
        Indicator::Connected => {
            Some(server.map_or_else(|| t!("connection-connected").into(), connected_to_text))
        }
        Indicator::Reconnecting { last_error, .. } => {
            Some(connection_reason_detail_text(last_error))
        }
        Indicator::NotConfigured => Some(t!("connection-not-configured-detail").into()),
        Indicator::ServiceFailed
        | Indicator::ServiceStopped
        | Indicator::Unsupported(_)
        | Indicator::Connecting => None,
    }
}

/// The speed limits popover as the status bar shows it.
pub(crate) struct LimitsPopover<'a> {
    /// The form inside it.
    pub(crate) form: &'a Entity<SpeedLimits>,
    /// Whether it is open.
    pub(crate) open: bool,
}

/// The status bar for `status`, of the window set to `server` (`host:port`, if one is set), whose
/// buttons act on the view of `cx`. `config_problem` adds a notice whose button calls
/// `on_dismiss`, and `state_not_saved` one that the UI state cannot be saved. `limits` is the
/// popover the limits button opens.
pub(crate) fn status_bar(
    status: &StatusModel,
    server: Option<&str>,
    config_problem: Option<ConfigProblem>,
    state_not_saved: bool,
    limits: &LimitsPopover<'_>,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &Context<'_, MainView>,
) -> StatusBar {
    let theme = cx.theme();
    // The indicator, always first on the left, pads itself by the rest of the bar's padding on
    // that side, room for its hover background: its dot stays as far in as the speeds at the
    // right end.
    let indicator = connection_indicator(&status.indicator(), server, cx);
    let mut bar = StatusBar::new().pl_1().left(indicator);
    if let Some(problem) = config_problem {
        bar = bar.left(config_notice(problem, on_dismiss));
    }
    if state_not_saved {
        bar = bar.left(state_notice());
    }
    // Both change the daemon's settings, so both need them known and the daemon there.
    let speed_enabled = status.changeable_speed().is_some();
    bar = bar
        .right(turtle(status, speed_enabled, theme, cx))
        .right(limits_button(status, speed_enabled, limits, cx));
    // The engine keeps the last numbers while the daemon does not answer; they are dimmed then,
    // like the table.
    let stale = status.stale;
    if let Some(free_space) = status.free_space {
        bar = bar.right(number("free-space", free_space_text(free_space), stale));
    }
    bar.right(number(
        "rate-down",
        download_rate_text(status.rate_down),
        stale,
    ))
    .right(number("rate-up", upload_rate_text(status.rate_up), stale))
}

/// The turtle: switches the alternative speed limits, drawn selected, its icon in the warning
/// color, while they apply; dimmed as well while it is disabled, like the stale numbers. Its
/// tooltip gives them, with the key.
fn turtle(
    status: &StatusModel,
    enabled: bool,
    theme: &Theme,
    cx: &Context<'_, MainView>,
) -> impl IntoElement + use<> {
    let on = status.alt_speed();
    // The icon's own color overrides the disabled button's, so it is dimmed here.
    let color = alt_speed_color(theme);
    let icon = Icon::new(gpui_kit::assets::IconName::Turtle).when(on, |icon| {
        icon.text_color(if enabled {
            color
        } else {
            color.opacity(STALE_OPACITY)
        })
    });
    Button::new("alt-speed")
        .ghost()
        .xsmall()
        .icon(icon)
        .selected(on)
        .toggled(on)
        .accessibility_label(SharedString::from(t!("alt-speed")))
        .tooltip_with_action(
            alt_speed_tooltip_text(status.speed),
            &ToggleAltSpeed,
            Some(MAIN_CONTEXT),
        )
        .disabled(!enabled)
        .on_click(cx.listener(|view, _, window, cx| view.press_turtle(window, cx)))
}

/// The button that shows the normal speed limits and opens their popover above it, `limits`.
/// Opening and closing it with the mouse or Escape goes through the view of `cx`, which fills the
/// form, and gives the keys back to the list when it closes.
fn limits_button(
    status: &StatusModel,
    enabled: bool,
    limits: &LimitsPopover<'_>,
    cx: &Context<'_, MainView>,
) -> impl IntoElement + use<> {
    let button = Button::new("speed-limits")
        .ghost()
        .xsmall()
        .label(limits_button_text(status.speed))
        .tooltip_with_action(
            SharedString::from(t!("speed-limits")),
            &EditSpeedLimits,
            Some(MAIN_CONTEXT),
        )
        .disabled(!enabled);
    let form = limits.form.clone();
    let first_field = form.read(cx).first_field(cx);
    let view = cx.weak_entity();
    Popover::new("speed-limits-popover")
        // The popover's own corner: it opens above the bar, ending where the button ends.
        .anchor(Anchor::BottomRight)
        .trigger(button)
        .open(limits.open)
        .track_focus(&first_field)
        .on_open_change(move |open, window, cx| {
            let open = *open;
            if let Err(error) = view.update(cx, |view, cx| {
                if open {
                    view.limits_button_opened(window, cx);
                } else {
                    view.close_speed_limits(window, cx);
                }
            }) {
                tracing::debug!(%error, "the speed limits popover changed after the window closed");
            }
        })
        .content(move |_, _, _| form.clone())
}

/// The dot and the text that tell how the connection to `server` is doing; a click opens the
/// Connection dialog of the view of `cx`. Under the pointer it is drawn as the bar's ghost buttons
/// are, and it is otherwise as it was before it could be clicked: its padding, where the hover
/// background shows, is taken from the bar's own ([`status_bar`]).
///
/// For screen readers it stays a status, which says how the connection is doing as that changes,
/// and Tab passes it by: the toolbar's Connection button and Ctrl+, open the dialog from the
/// keyboard.
fn connection_indicator(
    indicator: &Indicator,
    server: Option<&str>,
    cx: &Context<'_, MainView>,
) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let label = connection_text(indicator);
    let tooltip = connection_tooltip(indicator, server);
    // The dot alone says nothing to a screen reader, so the indicator is named either way.
    let name = if label.is_empty() {
        tooltip.clone().unwrap_or_default()
    } else {
        label.clone()
    };
    let hover = ghost_hover(theme);
    h_flex()
        .id("status-connection")
        .gap_1p5()
        .px_1()
        .rounded(theme.radius)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(|view, _, window, cx| view.open_connection(window, cx)))
        .child(
            div()
                .id("status-connection-dot")
                .size(px(DOT_SIZE))
                .flex_none()
                .rounded_full()
                .bg(connection_color(indicator, theme))
                .test_support(),
        )
        .when(!label.is_empty(), |indicator| indicator.child(label))
        .when_some(tooltip, |indicator, tooltip| {
            indicator.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        })
        .role(Role::Status)
        .aria_label(name)
        .test_support()
}

/// The notice that `config.toml` is not used, with the reason in its tooltip and a button that
/// dismisses it.
fn config_notice(
    problem: ConfigProblem,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let detail = config_problem_text(problem);
    h_flex()
        .gap_1()
        .child(
            div()
                .id("config-problem")
                .child(t!("status-config-problem"))
                .tooltip({
                    let detail = detail.clone();
                    move |window, cx| Tooltip::new(detail.clone()).build(window, cx)
                })
                .role(Role::Status)
                .aria_label(SharedString::from(t!("status-config-problem")))
                .aria_description(detail)
                .test_support(),
        )
        .child(
            Button::new("config-problem-dismiss")
                .ghost()
                .xsmall()
                .label(t!("status-dismiss"))
                .on_click(on_dismiss),
        )
}

/// The notice that the UI state cannot be saved, with what that means in its tooltip. It stays
/// until a save works again; the log has the error.
fn state_notice() -> impl IntoElement {
    let detail = SharedString::from(t!("status-state-not-saved-detail"));
    div()
        .id("state-not-saved")
        .child(t!("status-state-not-saved"))
        .tooltip({
            let detail = detail.clone();
            move |window, cx| Tooltip::new(detail.clone()).build(window, cx)
        })
        .role(Role::Status)
        .aria_label(SharedString::from(t!("status-state-not-saved")))
        .aria_description(detail)
        .test_support()
}

/// A number that changes every tick, in tabular digits so the bar does not jitter, and dimmed
/// while `stale`.
fn number(id: &'static str, text: SharedString, stale: bool) -> impl IntoElement {
    div()
        .id(id)
        .font_features(tabular_digits())
        .when(stale, |number| number.opacity(STALE_OPACITY))
        .child(text.clone())
        .aria_label(text)
        .test_support()
}

#[cfg(test)]
mod tests {
    use x4_core::{ConnectionError, UnsupportedReason};

    use super::*;

    #[test]
    fn the_tooltip_explains_the_dot_and_the_reconnect_reason() {
        assert_eq!(
            connection_tooltip(&Indicator::Connected, None),
            Some("Connected".into()),
            "a window that knows no server"
        );
        assert_eq!(
            connection_tooltip(&Indicator::Connected, Some("nas:9091")),
            Some("Connected to nas:9091".into())
        );
        assert_eq!(
            connection_tooltip(
                &Indicator::Reconnecting {
                    attempt: 1,
                    last_error: ConnectionError::Unauthorized,
                },
                Some("nas:9091")
            ),
            Some("The server rejected the credentials".into())
        );
        for indicator in [
            Indicator::Connecting,
            Indicator::ServiceStopped,
            Indicator::ServiceFailed,
            Indicator::Unsupported(UnsupportedReason::Legacy),
        ] {
            assert_eq!(
                connection_tooltip(&indicator, Some("nas:9091")),
                None,
                "the text says it all: {indicator:?}"
            );
        }
    }

    #[test]
    fn the_tooltip_without_a_server_says_where_to_set_one() {
        assert_eq!(
            connection_tooltip(&Indicator::NotConfigured, None),
            Some("Set the server in Connection (Ctrl+,)".into())
        );
    }
}
