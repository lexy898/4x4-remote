//! The memory allocator: glibc's malloc, told to give freed large buffers back to the system.
//!
//! By default glibc raises its mmap threshold to the size of every large block that is freed (up
//! to 32 MiB), and the next allocations of that size come from the arena of the thread that makes
//! them, where `free` keeps them resident. Adding a big `.torrent` makes such buffers: the file,
//! read on a short-lived reader thread, then its base64 form and the JSON body of the request on
//! the long-lived service thread, whose arena keeps them. Measured through the real add path, a
//! 16 MiB and then an 8 MiB add kept 37.6 to 39.6 MiB of private memory after they were done; with
//! the threshold fixed they keep 0.30 MiB. Fixing it also turns off the dynamic adjustment of the
//! trim threshold, so the heap's free top is returned as well.
//!
//! This is the project's only `unsafe`, approved by the owner: `mallopt` is a C function with no
//! safe wrapper. `malloc_trim` adds nothing once the threshold is fixed. `M_ARENA_MAX=1` would save
//! about 8 MiB more private memory at all times, but it costs CPU in the synthetic demo (about 0.7
//! points while it polls a busy daemon, about 17 while it scrolls), so it is not set.

use std::ffi::c_int;

/// glibc's own starting mmap threshold (`mallopt(3)`: 128 KiB), kept fixed: every allocation of
/// this size or more gets a mapping of its own, which `free` unmaps at once.
pub const MMAP_THRESHOLD: c_int = 128 * 1024;

/// Fixes glibc's mmap threshold at [`MMAP_THRESHOLD`], so that every freed allocation of that size
/// or more goes back to the system at once instead of staying in its thread's arena.
///
/// Call it first in `main`, before any thread starts: `mallopt` is not thread-safe. Returns
/// whether glibc took it.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[must_use]
pub fn pin_mmap_threshold() -> bool {
    // SAFETY: mallopt takes two integers and only sets glibc's own malloc parameters; it is called
    // before any other thread exists.
    #[expect(unsafe_code, reason = "no safe wrapper exists; the owner allowed this")]
    let taken = unsafe { libc::mallopt(libc::M_MMAP_THRESHOLD, MMAP_THRESHOLD) };
    taken == 1
}

/// Other allocators are left as they are: musl's returns large frees by itself, and other systems
/// have none of glibc's behavior to correct. Always `false`.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
#[must_use]
pub fn pin_mmap_threshold() -> bool {
    false
}

/// Logs what [`pin_mmap_threshold`] returned, once logging is up: at debug when glibc took the
/// threshold, at warn when glibc refused it (the memory of a big add then stays resident), and
/// not at all where there is no glibc to pin.
pub fn log_pin(pinned: bool) {
    if pinned {
        tracing::debug!(threshold = MMAP_THRESHOLD, "mmap threshold pinned");
    } else if cfg!(all(target_os = "linux", target_env = "gnu")) {
        tracing::warn!(
            threshold = MMAP_THRESHOLD,
            "glibc refused the mmap threshold; freed large buffers may stay resident"
        );
    }
}

#[cfg(test)]
mod tests {
    use tracing::Level;

    use super::*;
    use crate::test_log::events_of;

    #[test]
    fn a_pin_that_glibc_took_is_logged_at_debug() {
        let events = events_of(|| log_pin(true));
        assert_eq!(events, [(Level::DEBUG, "mmap threshold pinned".to_owned())]);
    }

    #[test]
    fn a_pin_that_glibc_refused_is_a_warning_and_elsewhere_silent() {
        let events = events_of(|| log_pin(false));
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            assert_eq!(events.len(), 1, "{events:?}");
            assert_eq!(events.first().map(|event| event.0), Some(Level::WARN));
        } else {
            assert!(events.is_empty(), "{events:?}");
        }
    }
}
