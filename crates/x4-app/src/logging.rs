//! Logging: to stderr always, and to a daily log file in the app's state directory.
//!
//! The file is written on a thread of its own, so logging never waits on the disk. If the file
//! cannot be opened the app logs to stderr only and says so; it still starts. A caller that wants
//! no file at all (the synthetic demo) logs to stderr only, and nothing warns about it.

use std::env::{self, VarError};
use std::io::{self, IsTerminal as _};
use std::path::Path;

use anyhow::Context as _;
use tracing::Subscriber;
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

use crate::private_dir::create_private_dir;

/// Used when `RUST_LOG` is unset: lifecycle events from our crates, problems from the rest.
const DEFAULT_LOG_FILTER: &str = "warn,x4_app=info,x4_core=info,x4_rpc=info";

/// The log files are `4x4.<date>.log`, one per day (UTC).
const LOG_FILE_PREFIX: &str = "4x4";
const LOG_FILE_SUFFIX: &str = "log";
/// How many daily files are kept, the current one included.
const LOG_FILES_KEPT: usize = 7;
/// How many log lines wait for the writer thread before new ones are dropped.
const LOG_BUFFER_LINES: usize = 1024;

/// Starts logging to stderr and, in `log_dir`, to a daily file `4x4.<date>.log`, keeping the last
/// 7 files. The level comes from `RUST_LOG`, or from a default that shows the lifecycle of the
/// app and the problems of its dependencies.
///
/// Without a `log_dir`, or if the file cannot be opened, only stderr is used, and a warning says
/// why. The returned guard flushes the file when dropped, so `main` must keep it until the end.
///
/// # Errors
///
/// An error if `RUST_LOG` is not a valid filter, or a logger is already installed. A problem with
/// the log file is not an error.
pub fn init_logging(log_dir: Option<&Path>) -> anyhow::Result<Option<WorkerGuard>> {
    install(log_dir.map_or(LogFile::DirUnknown, LogFile::In))
}

/// Starts logging to stderr only, with the level from `RUST_LOG` as [`init_logging`] takes it.
/// For a program that writes no files on purpose, so it does not warn that there is no log file.
///
/// # Errors
///
/// An error if `RUST_LOG` is not a valid filter, or a logger is already installed.
pub fn init_stderr_logging() -> anyhow::Result<()> {
    install(LogFile::NotWanted)?;
    Ok(())
}

/// Where the log goes besides stderr.
#[derive(Debug, Clone, Copy)]
enum LogFile<'a> {
    /// Daily files in this directory.
    In(&'a Path),
    /// No file, because the app does not know its state directory: a problem to report.
    DirUnknown,
    /// No file, as the caller asked.
    NotWanted,
}

/// Installs the logger, and warns if there is no log file for a reason the user should know.
fn install(log_file: LogFile<'_>) -> anyhow::Result<Option<WorkerGuard>> {
    let filter = parse_filter(env::var(EnvFilter::DEFAULT_ENV))?;
    let (subscriber, guard, problem) = build(filter, log_file, io::stderr().is_terminal());
    subscriber
        .try_init()
        .context("cannot install the log subscriber")?;
    if let Some(problem) = problem {
        tracing::warn!(problem, "logging to stderr only");
    }
    Ok(guard)
}

/// The filter for the value of `RUST_LOG`.
fn parse_filter(value: Result<String, VarError>) -> anyhow::Result<EnvFilter> {
    match value {
        Ok(directives) => EnvFilter::try_new(directives)
            .with_context(|| format!("invalid {} value", EnvFilter::DEFAULT_ENV)),
        Err(VarError::NotPresent) => Ok(EnvFilter::new(DEFAULT_LOG_FILTER)),
        Err(error) => Err(error).with_context(|| format!("unreadable {}", EnvFilter::DEFAULT_ENV)),
    }
}

/// The subscriber, the guard of the file writer, and why there is no file if that is a problem.
/// Nothing is installed, so tests can use the subscriber on its own.
fn build(
    filter: EnvFilter,
    log_file: LogFile<'_>,
    stderr_ansi: bool,
) -> (
    impl Subscriber + Send + Sync + 'static,
    Option<WorkerGuard>,
    Option<String>,
) {
    let (file_writer, guard, problem) = match log_file {
        LogFile::In(dir) => match open_log_file(dir) {
            Ok(appender) => {
                let (writer, guard) = file_writer_builder().finish(appender);
                (Some(writer), Some(guard), None)
            }
            Err(error) => (None, None, Some(error)),
        },
        LogFile::DirUnknown => (None, None, Some("the log directory is unknown".to_owned())),
        LogFile::NotWanted => (None, None, None),
    };
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(io::stderr).with_ansi(stderr_ansi))
        .with(file_writer.map(|writer| fmt::layer().with_writer(writer).with_ansi(false)));
    (subscriber, guard, problem)
}

/// The settings of the thread that writes the log file.
///
/// The default buffer is 128 000 lines, and the channel behind it is allocated and touched up
/// front, which costs about 4 MiB of private memory: a quarter of the app's 16 MiB budget. 1024
/// lines is far more than the app logs between two wakeups of the writer. If the disk stalls
/// for longer, dropping lines is better than blocking the thread that logs, which may be the UI.
fn file_writer_builder() -> NonBlockingBuilder {
    NonBlockingBuilder::default()
        .buffered_lines_limit(LOG_BUFFER_LINES)
        .lossy(true)
}

/// Opens today's log file in `dir`, creating the directory if needed and deleting the oldest
/// files beyond the last 7.
///
/// The builder is used on purpose: the helpers of `tracing_appender::rolling` panic when the
/// file cannot be opened.
fn open_log_file(dir: &Path) -> Result<RollingFileAppender, String> {
    create_private_dir(dir)
        .map_err(|error| format!("cannot create the log directory {}: {error}", dir.display()))?;
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(LOG_FILES_KEPT)
        .build(dir)
        .map_err(|error: InitError| {
            format!("cannot open the log file in {}: {error}", dir.display())
        })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// The log files in `dir`, sorted by name.
    fn log_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("a readable directory")
            .map(|entry| {
                entry
                    .expect("a directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| {
                name.starts_with("4x4.")
                    && Path::new(name).extension().is_some_and(|ext| ext == "log")
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn events_reach_the_daily_log_file() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // Not yet there: the logger creates the directory.
        let log_dir = dir.path().join("state").join("4x4");
        let (subscriber, guard, problem) =
            build(EnvFilter::new("info"), LogFile::In(&log_dir), false);

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(torrent = "abc", "marker event");
            tracing::debug!("filtered out");
        });
        drop(guard); // Flushes the writer thread.

        assert_eq!(problem, None);
        let files = log_files(&log_dir);
        assert_eq!(files.len(), 1, "{files:?}");
        let name = files.first().expect("one file");
        // 4x4.2026-10-29.log
        assert_eq!(name.len(), "4x4.0000-00-00.log".len(), "{name}");
        let text = fs::read_to_string(log_dir.join(name)).expect("readable");
        assert!(text.contains("marker event"), "{text}");
        assert!(text.contains("torrent=\"abc\""), "{text}");
        assert!(!text.contains("filtered out"), "{text}");
        assert!(!text.contains('\u{1b}'), "no colors in the file: {text:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for created in [log_dir.as_path(), log_dir.parent().expect("a parent")] {
                let mode = fs::metadata(created)
                    .expect("metadata")
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o700, "{}", created.display());
            }
        }
    }

    #[test]
    fn only_the_last_seven_log_files_are_kept() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        for day in 1..=10 {
            fs::write(
                dir.path().join(format!("4x4.2026-01-{day:02}.log")),
                "old\n",
            )
            .expect("written");
        }
        fs::write(dir.path().join("state.toml"), "x").expect("written");

        let (_subscriber, guard, problem) =
            build(EnvFilter::new("info"), LogFile::In(dir.path()), false);

        assert_eq!(problem, None);
        assert_eq!(log_files(dir.path()).len(), 7);
        assert!(dir.path().join("state.toml").exists(), "other files stay");
        drop(guard);
    }

    #[test]
    fn unusable_log_dir_falls_back_to_stderr_only() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let file = dir.path().join("not-a-directory");
        fs::write(&file, "x").expect("written");
        let log_dir = file.join("logs"); // A directory cannot be made inside a file.

        let (subscriber, guard, problem) =
            build(EnvFilter::new("info"), LogFile::In(&log_dir), false);

        assert!(guard.is_none());
        let problem = problem.expect("the problem is reported");
        assert!(problem.contains(&*log_dir.to_string_lossy()), "{problem}");
        // The subscriber still works.
        tracing::subscriber::with_default(subscriber, || tracing::info!("still logging"));
    }

    #[test]
    fn missing_log_dir_falls_back_to_stderr_only() {
        let (subscriber, guard, problem) =
            build(EnvFilter::new("info"), LogFile::DirUnknown, false);

        assert!(guard.is_none());
        assert!(problem.is_some());
        tracing::subscriber::with_default(subscriber, || tracing::info!("still logging"));
    }

    #[test]
    fn stderr_only_by_choice_is_no_problem() {
        let (subscriber, guard, problem) = build(EnvFilter::new("info"), LogFile::NotWanted, false);

        assert!(guard.is_none());
        assert_eq!(problem, None, "nothing to warn about");
        tracing::subscriber::with_default(subscriber, || tracing::info!("still logging"));
    }

    #[test]
    fn rust_log_unset_gives_the_default_filter() {
        let filter = parse_filter(Err(VarError::NotPresent)).expect("the default is valid");

        assert_eq!(
            filter.to_string(),
            EnvFilter::new(DEFAULT_LOG_FILTER).to_string()
        );
    }

    #[test]
    fn rust_log_sets_the_filter() {
        let filter = parse_filter(Ok("x4_app=trace".to_owned())).expect("valid");

        assert_eq!(filter.to_string(), "x4_app=trace");
    }

    #[test]
    fn invalid_rust_log_is_an_error() {
        let error = parse_filter(Ok("x4_app=loud".to_owned())).expect_err("not a level");

        assert!(error.to_string().contains("RUST_LOG"), "{error}");
    }

    #[test]
    fn unreadable_rust_log_is_an_error() {
        let value = Err(VarError::NotUnicode("\u{fffd}".into()));

        let error = parse_filter(value).expect_err("not unicode");

        assert!(error.to_string().contains("RUST_LOG"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn log_dir_that_cannot_be_written_falls_back_to_stderr_only() {
        let root = tempfile::tempdir().expect("a temporary directory");
        // The directory exists, so creating it succeeds and opening the file inside it fails.
        let log_dir = root.path().join("logs");
        fs::create_dir(&log_dir).expect("a directory");
        let Some(_locked) = crate::test_dirs::make_read_only(&log_dir) else {
            return;
        };

        let (subscriber, guard, problem) =
            build(EnvFilter::new("info"), LogFile::In(&log_dir), false);

        assert!(guard.is_none());
        let problem = problem.expect("the problem is reported");
        assert!(
            problem.starts_with("cannot open the log file in "),
            "{problem}"
        );
        assert!(problem.contains(&*log_dir.to_string_lossy()), "{problem}");
        // The subscriber still works.
        tracing::subscriber::with_default(subscriber, || tracing::info!("still logging"));
    }

    #[test]
    fn the_file_writer_buffers_a_thousand_lines_and_drops_the_rest() {
        // The buffer is allocated up front: the default of 128 000 lines costs about 4 MiB of
        // private memory. The builder has no getters, so its debug output pins the settings.
        let builder = format!("{:?}", file_writer_builder());

        assert!(builder.contains("buffered_lines_limit: 1024"), "{builder}");
        assert!(builder.contains("is_lossy: true"), "{builder}");
        assert_eq!(LOG_BUFFER_LINES, 1024);
    }
}
