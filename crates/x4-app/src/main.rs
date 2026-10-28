//! The `4x4` binary: sets up logging and runs the application from [`x4_app::app`].

use std::env::{self, VarError};
use std::io::{self, IsTerminal as _};
use std::time::Instant;

use anyhow::Context as _;
use tracing_subscriber::EnvFilter;

/// Used when `RUST_LOG` is unset: lifecycle events from our crates, problems from the rest.
const DEFAULT_LOG_FILTER: &str = "warn,x4_app=info,x4_core=info,x4_rpc=info";

fn main() -> anyhow::Result<()> {
    // Taken first so the first-frame log covers the whole startup.
    let started = Instant::now();
    init_tracing()?;
    x4_app::app::run(started)
}

fn init_tracing() -> anyhow::Result<()> {
    let filter = match env::var(EnvFilter::DEFAULT_ENV) {
        Ok(directives) => EnvFilter::try_new(directives)
            .with_context(|| format!("invalid {} value", EnvFilter::DEFAULT_ENV))?,
        Err(VarError::NotPresent) => EnvFilter::new(DEFAULT_LOG_FILTER),
        Err(error) => {
            return Err(error).with_context(|| format!("unreadable {}", EnvFilter::DEFAULT_ENV));
        }
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .try_init()
        .map_err(anyhow::Error::from_boxed)
}
