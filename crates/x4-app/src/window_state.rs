//! The main window's size between runs: the bounds it opens with, from the remembered
//! [`WindowState`], and the state to remember, from the bounds the platform reports.
//!
//! Only the size and whether the window is maximized are kept, never the position: a Wayland
//! client can neither read nor set where its window is, so the position is the compositor's
//! business.

use gpui_kit::{Bounds, Pixels, WindowBounds};
use x4_core::WindowState;

/// The bounds to open the window with: `bounds` (the remembered size, placed by the caller), as
/// the size of a normal window, or as the size it goes back to when it is no longer maximized.
pub(crate) fn opening_bounds(window: WindowState, bounds: Bounds<Pixels>) -> WindowBounds {
    if window.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}

/// The state to remember for a window whose bounds are `bounds` (`Window::window_bounds`): the
/// size of the normal window, which a maximized or full-screen window reports as the size it goes
/// back to, and whether it is maximized. A full-screen window is remembered as a normal one, as
/// 4x4 does not open full screen. An unusable size takes the default ([`WindowState::normalized`]).
pub(crate) fn remembered_window(bounds: WindowBounds) -> WindowState {
    let (normal, maximized) = match bounds {
        WindowBounds::Windowed(normal) | WindowBounds::Fullscreen(normal) => (normal, false),
        WindowBounds::Maximized(normal) => (normal, true),
    };
    WindowState {
        width: f32::from(normal.size.width),
        height: f32::from(normal.size.height),
        maximized,
    }
    .normalized()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{point, px, size};

    use super::*;

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    #[test]
    fn a_maximized_window_opens_maximized_over_its_normal_size() {
        let normal = bounds(10., 20., 1000., 650.);
        let window = |maximized| WindowState {
            width: 1000.,
            height: 650.,
            maximized,
        };

        assert_eq!(
            opening_bounds(window(true), normal),
            WindowBounds::Maximized(normal)
        );
        assert_eq!(
            opening_bounds(window(false), normal),
            WindowBounds::Windowed(normal)
        );
    }

    #[test]
    fn only_the_size_and_the_maximized_flag_are_remembered() {
        let expected = |maximized| WindowState {
            width: 1000.,
            height: 650.,
            maximized,
        };

        assert_eq!(
            remembered_window(WindowBounds::Windowed(bounds(300., 200., 1000., 650.))),
            expected(false)
        );
        assert_eq!(
            remembered_window(WindowBounds::Windowed(bounds(0., 0., 1000., 650.))),
            expected(false),
            "the position does not count"
        );
        assert_eq!(
            remembered_window(WindowBounds::Maximized(bounds(0., 0., 1000., 650.))),
            expected(true)
        );
        assert_eq!(
            remembered_window(WindowBounds::Fullscreen(bounds(0., 0., 1000., 650.))),
            expected(false)
        );
    }

    #[test]
    fn an_unusable_size_is_remembered_as_the_default() {
        for (width, height) in [(20000., 700.), (100., 100.)] {
            assert_eq!(
                remembered_window(WindowBounds::Maximized(bounds(0., 0., width, height))),
                WindowState {
                    maximized: true,
                    ..WindowState::default()
                },
                "{width} x {height}"
            );
        }
    }
}
