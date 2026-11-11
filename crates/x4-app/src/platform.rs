//! What differs between the operating systems the app runs on, as far as the app itself needs to
//! know it.

use gpui_kit::OperatingSystem;

/// The operating system this build runs on, in gpui's terms. Where a window is at hand, prefer
/// `cx.platform_info().os`, which tests can pin; this is for code that has no context.
#[must_use]
pub const fn host_os() -> OperatingSystem {
    if cfg!(target_os = "macos") {
        OperatingSystem::Mac
    } else if cfg!(target_os = "windows") {
        OperatingSystem::Windows
    } else {
        OperatingSystem::Linux
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_os_is_the_one_the_build_targets() {
        let expected = if cfg!(target_os = "macos") {
            OperatingSystem::Mac
        } else if cfg!(target_os = "windows") {
            OperatingSystem::Windows
        } else {
            OperatingSystem::Linux
        };
        assert_eq!(host_os(), expected);
    }
}
