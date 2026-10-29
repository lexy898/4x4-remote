//! The look of the window: the light or dark theme, the colors that tell torrent states and the
//! connection apart, and tabular digits for numbers.
//!
//! Every color is a field of the gpui-component [`Theme`], so it changes with the light and dark
//! mode and with a theme file; the views use no color literals. Each state maps to its color here,
//! in one place.

use std::sync::Arc;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, FontFeatures, Hsla, Window, WindowAppearance};
use x4_core::{ConnectionState, DisplayState, ThemeChoice};

use crate::status_bar::ServiceState;

/// The color of a torrent state: blue while downloading, green while seeding, the warning color
/// while queued or checking, the danger color for an error, and muted while paused.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the torrent table colors its rows in Task 6")
)]
pub(crate) fn state_color(state: DisplayState, theme: &Theme) -> Hsla {
    match state {
        DisplayState::Downloading => theme.blue,
        DisplayState::Seeding => theme.green,
        DisplayState::Checking | DisplayState::Queued(_) => theme.warning,
        DisplayState::Error => theme.danger,
        DisplayState::Paused => theme.muted_foreground,
    }
}

/// The color of the connection dot in the status bar: the success color while connected, muted
/// while connecting, the warning color while reconnecting, and the danger color when the daemon
/// is too old or the service is not running.
pub(crate) fn connection_color(
    service: ServiceState,
    connection: &ConnectionState,
    theme: &Theme,
) -> Hsla {
    match (service, connection) {
        (ServiceState::Stopped | ServiceState::Failed, _)
        | (ServiceState::Running, ConnectionState::Unsupported { .. }) => theme.danger,
        (ServiceState::Running, ConnectionState::Connected) => theme.success,
        (ServiceState::Running, ConnectionState::Connecting) => theme.muted_foreground,
        (ServiceState::Running, ConnectionState::Reconnecting { .. }) => theme.warning,
    }
}

/// Digits of one width, so that numbers that change every tick do not make the text around them
/// jump. The UI fonts of Linux desktops (Adwaita Sans, Cantarell, Noto Sans) have the feature.
pub(crate) fn tabular_digits() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".to_owned(), 1)]))
}

/// The choice after `choice` on the theme switch: system, then light, then dark, then system
/// again.
pub(crate) fn next_theme(choice: ThemeChoice) -> ThemeChoice {
    match choice {
        ThemeChoice::System => ThemeChoice::Light,
        ThemeChoice::Light => ThemeChoice::Dark,
        ThemeChoice::Dark => ThemeChoice::System,
    }
}

/// The mode `choice` shows while the system appearance is `appearance`: the system's for
/// [`ThemeChoice::System`], and the chosen one otherwise.
pub(crate) fn theme_mode(choice: ThemeChoice, appearance: WindowAppearance) -> ThemeMode {
    match choice {
        ThemeChoice::System => appearance.into(),
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::Dark => ThemeMode::Dark,
    }
}

/// Switches every window to the theme `choice` asks for, reading the system appearance from
/// `window`. On Linux the window's appearance is the reliable one (gpui-component issue #104).
pub(crate) fn apply_theme(choice: ThemeChoice, window: &mut Window, cx: &mut App) {
    let mode = theme_mode(choice, window.appearance());
    Theme::change(mode, Some(window), cx);
}

/// Follows a change of the system appearance when `choice` is [`ThemeChoice::System`]; a fixed
/// light or dark theme stays as it is.
pub(crate) fn follow_appearance(choice: ThemeChoice, window: &mut Window, cx: &mut App) {
    if choice == ThemeChoice::System {
        apply_theme(choice, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::component::ActiveTheme as _;
    use x4_core::{ConnectionError, QueueKind, UnsupportedReason};

    use super::*;

    /// The theme of `mode`, as gpui-component loads it.
    fn theme(mode: ThemeMode, cx: &mut TestAppContext) -> Theme {
        cx.update(|cx| {
            gpui_kit::init(cx);
            Theme::change(mode, None, cx);
            cx.theme().clone()
        })
    }

    #[gpui_kit::test]
    fn state_colors_differ_by_state(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = theme(mode, cx);
            let color = |state| state_color(state, &theme);

            let distinct = [
                color(DisplayState::Downloading),
                color(DisplayState::Seeding),
                color(DisplayState::Error),
                color(DisplayState::Checking),
                color(DisplayState::Paused),
            ];
            for (index, first) in distinct.iter().enumerate() {
                for second in &distinct[index + 1..] {
                    assert_ne!(first, second, "{mode:?}");
                }
            }
            assert_eq!(
                color(DisplayState::Queued(QueueKind::Download)),
                color(DisplayState::Checking)
            );
            assert_eq!(
                color(DisplayState::Queued(QueueKind::Seed)),
                color(DisplayState::Checking)
            );
        }
    }

    #[gpui_kit::test]
    fn state_colors_follow_the_mode(cx: &mut TestAppContext) {
        let light = theme(ThemeMode::Light, cx);
        let dark = theme(ThemeMode::Dark, cx);

        assert_eq!(state_color(DisplayState::Seeding, &light), light.green);
        assert_eq!(state_color(DisplayState::Seeding, &dark), dark.green);
        assert_ne!(
            state_color(DisplayState::Seeding, &light),
            state_color(DisplayState::Seeding, &dark)
        );
        assert_eq!(state_color(DisplayState::Downloading, &dark), dark.blue);
        assert_eq!(state_color(DisplayState::Error, &dark), dark.danger);
        assert_eq!(state_color(DisplayState::Checking, &dark), dark.warning);
        assert_eq!(
            state_color(DisplayState::Paused, &dark),
            dark.muted_foreground
        );
    }

    #[gpui_kit::test]
    fn connection_colors_tell_the_states_apart(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = theme(mode, cx);
            let running = |connection: &ConnectionState| {
                connection_color(ServiceState::Running, connection, &theme)
            };
            let reconnecting = ConnectionState::Reconnecting {
                attempt: 1,
                last_error: ConnectionError::Unreachable,
            };
            let unsupported = ConnectionState::Unsupported {
                reason: UnsupportedReason::Legacy,
            };

            let distinct = [
                running(&ConnectionState::Connected),
                running(&ConnectionState::Connecting),
                running(&reconnecting),
                running(&unsupported),
            ];
            for (index, first) in distinct.iter().enumerate() {
                for second in &distinct[index + 1..] {
                    assert_ne!(first, second, "{mode:?}");
                }
            }
            for service in [ServiceState::Stopped, ServiceState::Failed] {
                assert_eq!(
                    connection_color(service, &ConnectionState::Connected, &theme),
                    theme.danger,
                    "a stopped service is never shown as connected"
                );
            }
        }
    }

    #[test]
    fn tabular_digits_turn_on_tnum() {
        assert_eq!(tabular_digits().tag_value_list(), [("tnum".to_owned(), 1)]);
    }

    #[test]
    fn the_theme_switch_cycles_system_light_dark() {
        assert_eq!(next_theme(ThemeChoice::System), ThemeChoice::Light);
        assert_eq!(next_theme(ThemeChoice::Light), ThemeChoice::Dark);
        assert_eq!(next_theme(ThemeChoice::Dark), ThemeChoice::System);
    }

    #[test]
    fn system_follows_the_appearance_and_a_fixed_choice_does_not() {
        use WindowAppearance::{Dark, Light, VibrantDark, VibrantLight};

        for appearance in [Light, VibrantLight] {
            assert_eq!(
                theme_mode(ThemeChoice::System, appearance),
                ThemeMode::Light
            );
        }
        for appearance in [Dark, VibrantDark] {
            assert_eq!(theme_mode(ThemeChoice::System, appearance), ThemeMode::Dark);
        }
        for appearance in [Light, VibrantLight, Dark, VibrantDark] {
            assert_eq!(theme_mode(ThemeChoice::Light, appearance), ThemeMode::Light);
            assert_eq!(theme_mode(ThemeChoice::Dark, appearance), ThemeMode::Dark);
        }
    }

    #[gpui_kit::test]
    fn apply_theme_switches_the_mode(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let window = cx.add_window(|_, _| gpui_kit::Empty);
        let mode_after = |choice, cx: &mut TestAppContext| {
            window
                .update(cx, |_, window, cx| {
                    apply_theme(choice, window, cx);
                    cx.theme().mode
                })
                .expect("the window is open")
        };

        assert_eq!(mode_after(ThemeChoice::Dark, cx), ThemeMode::Dark);
        // The test platform's appearance is light.
        assert_eq!(mode_after(ThemeChoice::System, cx), ThemeMode::Light);
        assert_eq!(mode_after(ThemeChoice::Dark, cx), ThemeMode::Dark);
        assert_eq!(mode_after(ThemeChoice::Light, cx), ThemeMode::Light);
    }

    #[gpui_kit::test]
    fn an_appearance_change_leaves_a_fixed_theme_alone(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let window = cx.add_window(|_, _| gpui_kit::Empty);

        let mode = window
            .update(cx, |_, window, cx| {
                apply_theme(ThemeChoice::Dark, window, cx);
                // The window reports a light appearance; a dark choice stays dark.
                follow_appearance(ThemeChoice::Dark, window, cx);
                let fixed = cx.theme().mode;
                follow_appearance(ThemeChoice::System, window, cx);
                (fixed, cx.theme().mode)
            })
            .expect("the window is open");

        assert_eq!(mode, (ThemeMode::Dark, ThemeMode::Light));
    }
}
