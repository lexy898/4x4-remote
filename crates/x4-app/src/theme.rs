//! The look of the window: the light or dark theme, the colors that tell torrent states, the
//! filters and the connection apart, and tabular digits for numbers.
//!
//! Every color is a field of the gpui-component [`Theme`], so it changes with the light and dark
//! mode and with a theme file; the views use no color literals. Each state maps to its color here,
//! in one place.

use std::sync::Arc;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, FontFeatures, Hsla, Window, WindowAppearance};
use x4_core::{DisplayState, Filter, Indicator, Subtitle, ThemeChoice, TrackerStatus};

/// How visible old data stays: the table and the status bar's numbers are dimmed while the
/// daemon does not answer, but stay readable.
pub(crate) const STALE_OPACITY: f32 = 0.5;

/// How visible an idle row of the table is: it recedes behind the rows where something happens,
/// and stays readable.
pub(crate) const DIMMED_OPACITY: f32 = 0.55;

/// The opacity of a row of the table: the stale opacity while the data is `stale`, the dimmed
/// one for an idle (`dimmed`) row, and none otherwise. Never both: a stale idle row would fade to
/// a quarter and become hard to read.
pub(crate) fn row_opacity(stale: bool, dimmed: bool) -> Option<f32> {
    if stale {
        Some(STALE_OPACITY)
    } else if dimmed {
        Some(DIMMED_OPACITY)
    } else {
        None
    }
}

/// How strong the track of a progress bar is, as a fraction of the bar's color, so that the
/// track is a pale shade of the state.
pub(crate) const PROGRESS_TRACK_OPACITY: f32 = 0.22;

/// The color of a torrent state: blue while downloading, green while seeding, the warning color
/// while queued or checking, the danger color for an error, and muted while paused.
pub(crate) fn state_color(state: DisplayState, theme: &Theme) -> Hsla {
    match state {
        DisplayState::Downloading => theme.blue,
        DisplayState::Seeding => theme.green,
        DisplayState::Checking | DisplayState::Queued(_) => theme.warning,
        DisplayState::Error => theme.danger,
        DisplayState::Paused => theme.muted_foreground,
    }
}

/// The color of a filter's icon in the sidebar: the color of the state it shows for downloading,
/// seeding, paused and errors, the primary color for the active torrents, and the sidebar's text
/// color for all of them.
pub(crate) fn filter_color(filter: Filter, theme: &Theme) -> Hsla {
    match filter {
        Filter::All => theme.sidebar_foreground,
        Filter::Downloading => state_color(DisplayState::Downloading, theme),
        Filter::Seeding => state_color(DisplayState::Seeding, theme),
        Filter::Paused => state_color(DisplayState::Paused, theme),
        Filter::Active => theme.primary,
        Filter::Error => state_color(DisplayState::Error, theme),
    }
}

/// The color of a count in the sidebar: the danger color for one that needs attention (see
/// `FilterCounts::needs_attention`), and muted otherwise.
pub(crate) fn count_color(needs_attention: bool, theme: &Theme) -> Hsla {
    if needs_attention {
        theme.danger
    } else {
        theme.muted_foreground
    }
}

/// The color of the line under a torrent's name: the danger color for an error of the torrent or
/// its tracker, the warning color for a tracker warning, and muted otherwise.
pub(crate) fn subtitle_color(subtitle: &Subtitle, theme: &Theme) -> Hsla {
    match subtitle {
        Subtitle::Error(_) | Subtitle::TrackerError(_) => theme.danger,
        Subtitle::TrackerWarning(_) => theme.warning,
        Subtitle::Downloading { .. }
        | Subtitle::Seeding { .. }
        | Subtitle::Paused
        | Subtitle::Queued(_)
        | Subtitle::Checking { .. }
        | Subtitle::FetchingMetadata { .. } => theme.muted_foreground,
    }
}

/// The color of a tracker's dot and of the line under its address: green while it works, the info
/// color (cyan in the default themes) while an announce is under way, the danger color for a
/// failed announce, the warning color for one that timed out, and muted for a backup and for a
/// tracker not announced to yet. The primary color would not do for an announce: it is the color
/// of plain text.
pub(crate) fn tracker_color(status: TrackerStatus, theme: &Theme) -> Hsla {
    match status {
        TrackerStatus::Working => theme.success,
        TrackerStatus::Announcing => theme.info,
        TrackerStatus::Failed => theme.danger,
        TrackerStatus::TimedOut => theme.warning,
        TrackerStatus::Backup | TrackerStatus::NotContacted => theme.muted_foreground,
    }
}

/// The color of a note about a tracker's trouble, in the General tab and after a tracker's status
/// line in the Trackers tab: muted for a `mild` kind (see `TrackerProblemKind::is_mild`), such as a
/// scrape that many trackers never answer while their announces work, and the warning color
/// otherwise. Both tabs take it from here, so that they say the same trouble in the same color.
pub(crate) fn problem_note_color(mild: bool, theme: &Theme) -> Hsla {
    if mild {
        theme.muted_foreground
    } else {
        theme.warning
    }
}

/// The color of a download or upload rate in the table: the color of the torrent's state while
/// data moves that way, so that what is active stands out, and muted otherwise.
pub(crate) fn rate_color(state: DisplayState, active: bool, theme: &Theme) -> Hsla {
    if active {
        state_color(state, theme)
    } else {
        theme.muted_foreground
    }
}

/// The color of the connection dot in the status bar: the success color while connected, muted
/// while connecting, the warning color while reconnecting, and the danger color when the daemon
/// is too old or the service is not running.
pub(crate) fn connection_color(indicator: &Indicator, theme: &Theme) -> Hsla {
    match indicator {
        Indicator::ServiceFailed | Indicator::ServiceStopped | Indicator::Unsupported(_) => {
            theme.danger
        }
        Indicator::Reconnecting { .. } => theme.warning,
        Indicator::Connecting => theme.muted_foreground,
        Indicator::Connected => theme.success,
    }
}

/// The color of the turtle's icon while the alternative speed limits apply: the warning color, so
/// that slowed-down transfers show at a glance. While they do not apply, the icon keeps the
/// button's own muted look.
pub(crate) fn alt_speed_color(theme: &Theme) -> Hsla {
    theme.warning
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
    use x4_core::{
        ConnectionError, QueueKind, TrackerProblemKind, TrackerStatus, UnsupportedReason,
    };

    use super::*;

    #[test]
    fn a_row_has_one_opacity_and_stale_wins() {
        assert_eq!(row_opacity(false, false), None);
        assert_eq!(row_opacity(false, true), Some(DIMMED_OPACITY));
        assert_eq!(row_opacity(true, false), Some(STALE_OPACITY));
        assert_eq!(
            row_opacity(true, true),
            Some(STALE_OPACITY),
            "never the product of the two"
        );
    }

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
    fn the_turtle_is_on_in_the_warning_color(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = theme(mode, cx);
            assert_eq!(alt_speed_color(&theme), theme.warning, "{mode:?}");
            assert_ne!(alt_speed_color(&theme), theme.muted_foreground, "{mode:?}");
        }
    }

    #[gpui_kit::test]
    fn subtitles_of_errors_stand_out(cx: &mut TestAppContext) {
        let theme = theme(ThemeMode::Dark, cx);
        let color = |subtitle: Subtitle| subtitle_color(&subtitle, &theme);

        assert_eq!(color(Subtitle::Error("disk full".to_owned())), theme.danger);
        assert_eq!(color(Subtitle::TrackerError(String::new())), theme.danger);
        assert_eq!(
            color(Subtitle::TrackerWarning(String::new())),
            theme.warning
        );
        for quiet in [
            Subtitle::Downloading {
                from_peers: 1,
                connected: 2,
            },
            Subtitle::Seeding { to_peers: 3 },
            Subtitle::Paused,
            Subtitle::Queued(QueueKind::Seed),
            Subtitle::Checking { percent: 5 },
            Subtitle::FetchingMetadata { percent: 6 },
        ] {
            assert_eq!(color(quiet), theme.muted_foreground);
        }
    }

    #[gpui_kit::test]
    fn trackers_take_the_colors_of_what_they_are_doing(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = theme(mode, cx);
            let color = |status| tracker_color(status, &theme);

            assert_eq!(color(TrackerStatus::Working), theme.success, "{mode:?}");
            // The info color, cyan in the default themes: the primary color is the plain text's.
            assert_eq!(color(TrackerStatus::Announcing), theme.info, "{mode:?}");
            assert_ne!(
                color(TrackerStatus::Announcing),
                theme.foreground,
                "{mode:?}"
            );
            assert_eq!(color(TrackerStatus::Failed), theme.danger);
            assert_eq!(color(TrackerStatus::TimedOut), theme.warning);
            // A tracker that is not in use, or not announced to yet, recedes.
            for quiet in [TrackerStatus::Backup, TrackerStatus::NotContacted] {
                assert_eq!(color(quiet), theme.muted_foreground, "{quiet:?}");
            }
            // Every color that means something differs from the others.
            let distinct = [
                color(TrackerStatus::Working),
                color(TrackerStatus::Announcing),
                color(TrackerStatus::Failed),
                color(TrackerStatus::TimedOut),
                color(TrackerStatus::Backup),
            ];
            for (index, first) in distinct.iter().enumerate() {
                for second in &distinct[index + 1..] {
                    assert_ne!(first, second, "{mode:?}");
                }
            }
        }
    }

    #[gpui_kit::test]
    fn a_scrape_problem_is_a_muted_note_beside_a_dot_that_stays_green(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = theme(mode, cx);
            // The Trackers tab's note: a scrape's trouble is always mild.
            let scrape_note = problem_note_color(true, &theme);

            assert_eq!(scrape_note, theme.muted_foreground, "{mode:?}");
            // The General tab says a scrape's trouble in the same color.
            for kind in [
                TrackerProblemKind::ScrapeFailed,
                TrackerProblemKind::ScrapeTimedOut,
            ] {
                assert_eq!(
                    problem_note_color(kind.is_mild(), &theme),
                    scrape_note,
                    "{kind:?}"
                );
            }
            // An announce's trouble is a warning.
            for kind in [
                TrackerProblemKind::AnnounceFailed,
                TrackerProblemKind::AnnounceTimedOut,
            ] {
                assert_eq!(
                    problem_note_color(kind.is_mild(), &theme),
                    theme.warning,
                    "{kind:?}"
                );
            }
            assert_ne!(
                scrape_note,
                tracker_color(TrackerStatus::Working, &theme),
                "{mode:?}"
            );
        }
    }

    #[gpui_kit::test]
    fn rates_take_the_state_color_only_while_moving(cx: &mut TestAppContext) {
        let theme = theme(ThemeMode::Light, cx);

        assert_eq!(rate_color(DisplayState::Seeding, true, &theme), theme.green);
        assert_eq!(
            rate_color(DisplayState::Downloading, true, &theme),
            theme.blue
        );
        for state in [DisplayState::Seeding, DisplayState::Downloading] {
            assert_eq!(rate_color(state, false, &theme), theme.muted_foreground);
        }
    }

    #[gpui_kit::test]
    fn connection_colors_tell_the_states_apart(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = theme(mode, cx);
            let color = |indicator: Indicator| connection_color(&indicator, &theme);
            let reconnecting = Indicator::Reconnecting {
                attempt: 1,
                last_error: ConnectionError::Unreachable,
            };

            let distinct = [
                color(Indicator::Connected),
                color(Indicator::Connecting),
                color(reconnecting),
                color(Indicator::Unsupported(UnsupportedReason::Legacy)),
            ];
            for (index, first) in distinct.iter().enumerate() {
                for second in &distinct[index + 1..] {
                    assert_ne!(first, second, "{mode:?}");
                }
            }
            assert_eq!(color(Indicator::Connected), theme.success);
            for gone in [Indicator::ServiceStopped, Indicator::ServiceFailed] {
                assert_eq!(color(gone), theme.danger, "{mode:?}");
            }
        }
    }

    #[gpui_kit::test]
    fn filters_take_the_colors_of_their_states(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = theme(mode, cx);
            let color = |filter| filter_color(filter, &theme);

            assert_eq!(color(Filter::All), theme.sidebar_foreground, "{mode:?}");
            assert_eq!(
                color(Filter::Downloading),
                state_color(DisplayState::Downloading, &theme)
            );
            assert_eq!(color(Filter::Downloading), theme.blue);
            assert_eq!(
                color(Filter::Seeding),
                state_color(DisplayState::Seeding, &theme)
            );
            assert_eq!(color(Filter::Seeding), theme.green);
            assert_eq!(color(Filter::Paused), theme.muted_foreground);
            assert_eq!(color(Filter::Active), theme.primary);
            assert_eq!(color(Filter::Error), theme.danger);
        }
    }

    #[gpui_kit::test]
    fn only_a_count_that_needs_attention_is_in_the_danger_color(cx: &mut TestAppContext) {
        let theme = theme(ThemeMode::Dark, cx);

        assert_eq!(count_color(true, &theme), theme.danger);
        assert_eq!(count_color(false, &theme), theme.muted_foreground);
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
