//! The `4x4` binary: pins the allocator ([`x4_app::allocator`]), finds the app's directories and
//! sets up logging; then hands its arguments to the 4x4 the user runs already
//! ([`x4_app::instance`]) and exits, or runs the application from [`x4_app::app`] with them.

use std::env;
use std::path::PathBuf;
use std::time::Instant;

use x4_app::adding::launch_requests;
use x4_app::allocator::{log_pin, pin_mmap_threshold};
use x4_app::app::app_id;
use x4_app::instance::{self, Claim};
use x4_app::logging::init_logging;
use x4_app::paths::{AppPaths, runtime_dir};

fn main() -> anyhow::Result<()> {
    // First of all: no thread exists yet (the log writer is started below), and mallopt is not
    // thread-safe. Logged once logging is up.
    let mmap_pinned = pin_mmap_threshold();
    // Taken right after, so the first-frame log covers the whole startup.
    let started = Instant::now();
    let paths = AppPaths::discover();
    // Kept until the end of `main`: dropping it flushes the log file.
    let _log_guard = init_logging(paths.as_ref().ok().map(|paths| paths.log_dir.as_path()))?;
    log_pin(mmap_pinned);
    // Before the config, the service and the window: a later launch only hands its arguments over.
    let instance = match instance::claim(app_id(), runtime_dir().as_deref()) {
        Claim::HandedOff => return Ok(()),
        Claim::Primary(listener) => Some(listener),
        Claim::Standalone(_) => None,
    };
    let paths = match paths {
        Ok(paths) => Some(paths),
        Err(error) => {
            tracing::warn!(%error, "running without a config file and without saved state");
            None
        }
    };
    let cwd = env::current_dir().unwrap_or_else(|error| {
        tracing::warn!(%error, "the working directory is unknown; relative paths given are refused");
        PathBuf::new()
    });
    let requests = launch_requests(env::args_os().skip(1), &cwd);
    x4_app::app::run(started, paths.as_ref(), requests, instance)
}
