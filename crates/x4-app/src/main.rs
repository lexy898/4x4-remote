//! The `4x4` binary: finds the app's directories, sets up logging and runs the application from
//! [`x4_app::app`].

use std::time::Instant;

use x4_app::logging::init_logging;
use x4_app::paths::AppPaths;

fn main() -> anyhow::Result<()> {
    // Taken first so the first-frame log covers the whole startup.
    let started = Instant::now();
    let paths = AppPaths::discover();
    // Kept until the end of `main`: dropping it flushes the log file.
    let _log_guard = init_logging(paths.as_ref().ok().map(|paths| paths.log_dir.as_path()))?;
    let paths = match paths {
        Ok(paths) => Some(paths),
        Err(error) => {
            tracing::warn!(%error, "running without a config file and without saved state");
            None
        }
    };
    x4_app::app::run(started, paths.as_ref())
}
