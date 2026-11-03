//! What the sources of adds ask the window for ([`AddRequest`]), the requests of launch arguments
//! ([`launch_requests`]), and the reading of a `.torrent` file when its turn in the queue comes
//! ([`read_torrent`]).
//!
//! A `.torrent` file is untrusted: it may be a FIFO that never ends, a device, a directory, or a
//! file of gigabytes. It is read only once `stat` says it is a regular file of at most
//! [`MAX_TORRENT_FILE_BYTES`], and never more than that is read, even from a file that grows
//! meanwhile. It is opened with `O_NONBLOCK`, so that a FIFO swapped in after the first look
//! opens at once, and is then turned away as no regular file. Reading and parsing take a while for
//! a large or hostile file, so the main view runs them on the background executor, never on the UI
//! thread. A read that never ends, as on a network file system that hangs, keeps its thread: the
//! main view lets the user skip it, and the thread is abandoned.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use x4_core::launch::{LaunchArg, RejectedArg, classify};
use x4_core::{AddItem, LoadedItem, MAX_TORRENT_FILE_BYTES, MetainfoError, QUEUE_CAP};

/// What a source of adds asks the window for: the toolbar, the keys, the file picker, the
/// clipboard, a drop, the launch arguments and a second launch all send these, through the one
/// channel the window listens to
/// ([`MainView::listen_for_adds`](crate::main_view::MainView::listen_for_adds)).
///
/// Nothing is added by a request: the items wait in the add dialog's queue for the user's Add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddRequest {
    /// `.torrent` files and magnet links to offer in the add dialog, in this order.
    Items(Vec<AddItem>),
    /// What a source that stops at the queue's cap, such as the clipboard, hands over: `items` to
    /// offer, in this order, and how many more it `left_out` unread. The window counts those with
    /// the ones a full queue drops, in one notification.
    Capped {
        /// The items to offer, in this order.
        items: Vec<AddItem>,
        /// How many more the source left out.
        left_out: usize,
    },
    /// Launch arguments 4x4 does not open; the window says so in one notification, which names
    /// why the first is refused.
    Rejected {
        /// The first of them.
        first: RejectedArg,
        /// How many there are, the first included.
        count: usize,
    },
    /// Bring the window to the front, as for a second launch that handed its arguments over.
    Raise,
}

/// The one channel of [`AddRequest`]s into the main window. The app makes it at startup
/// ([`Startup::adds`](crate::app::Startup::adds)): the sources outside the window, such as the
/// launch arguments and a second launch, send into clones of `sender`; the window listens on
/// `receiver` and keeps `sender` for the sources inside it
/// ([`MainView::listen_for_adds`](crate::main_view::MainView::listen_for_adds),
/// [`MainView::add_requests`](crate::main_view::MainView::add_requests)).
#[derive(Debug, Clone)]
pub struct AddRequests {
    /// Where the sources send their requests.
    pub sender: async_channel::Sender<AddRequest>,
    /// What the window takes them from.
    pub receiver: async_channel::Receiver<AddRequest>,
}

impl AddRequests {
    /// A new channel, with no request in it.
    #[must_use]
    pub fn new() -> Self {
        let (sender, receiver) = async_channel::unbounded();
        Self { sender, receiver }
    }
}

impl Default for AddRequests {
    fn default() -> Self {
        Self::new()
    }
}

/// The requests of the launch arguments `args` of a process started in `cwd`, told apart by
/// [`classify`]: the `.torrent` files and magnet links, in their order, then one
/// [`AddRequest::Rejected`] for the arguments 4x4 does not open; each only when there are any. The
/// first `--`, which a shell or `cargo run` may pass, is skipped; an argument after it that starts
/// with `-` is still refused as an option.
///
/// The work is bounded by the add dialog's queue, not by the arguments, of which a second launch
/// may hand over a million: once [`QUEUE_CAP`] items are told apart, which is all the queue holds,
/// the arguments after them are only counted, as the `left_out` of an [`AddRequest::Capped`]
/// (an [`AddRequest::Items`] when there are none). A refusal keeps the first and a count.
pub fn launch_requests<A: AsRef<OsStr>>(
    args: impl IntoIterator<Item = A>,
    cwd: &Path,
) -> Vec<AddRequest> {
    let mut items = Vec::new();
    let mut left_out = 0;
    let mut first_rejected = None;
    let mut rejected = 0;
    let mut options_ended = false;
    for arg in args {
        let arg = arg.as_ref();
        if !options_ended && arg == "--" {
            options_ended = true;
            continue;
        }
        // The queue would leave them out all the same; each would join `cwd` to be told apart.
        if items.len() == QUEUE_CAP {
            left_out += 1;
            continue;
        }
        match classify(arg, cwd) {
            LaunchArg::Magnet(link) => items.push(AddItem::Magnet(Some(link))),
            LaunchArg::File(path) => items.push(AddItem::File(path)),
            LaunchArg::Rejected(refused) => {
                first_rejected.get_or_insert(refused);
                rejected += 1;
            }
        }
    }
    // Only a full queue leaves any out.
    let items = match (items.is_empty(), left_out) {
        (true, _) => None,
        (false, 0) => Some(AddRequest::Items(items)),
        (false, left_out) => Some(AddRequest::Capped { items, left_out }),
    };
    [
        items,
        first_rejected.map(|first| AddRequest::Rejected {
            first,
            count: rejected,
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Why a `.torrent` file cannot be offered in the add dialog.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LoadError {
    /// It is not there.
    #[error("the file does not exist")]
    Missing,
    /// 4x4 may not read it.
    #[error("the file may not be read")]
    Denied,
    /// It could not be read for another reason.
    #[error("the file could not be read: {0}")]
    Unreadable(io::ErrorKind),
    /// It is a directory, a FIFO, a socket or a device.
    #[error("not a regular file")]
    NotAFile,
    /// It is larger than [`MAX_TORRENT_FILE_BYTES`].
    #[error("the file has {size} bytes, more than the {MAX_TORRENT_FILE_BYTES} that are read")]
    TooLarge {
        /// Its size in bytes, as far as it is known.
        size: u64,
    },
    /// It is not a torrent the daemon can add.
    #[error(transparent)]
    Invalid(MetainfoError),
}

impl From<io::Error> for LoadError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::Missing,
            io::ErrorKind::PermissionDenied => Self::Denied,
            kind => Self::Unreadable(kind),
        }
    }
}

/// The item `item` as the add dialog shows it: a magnet link as it is, a `.torrent` file read and
/// parsed by [`read_torrent`]. It blocks while a file is read, so it runs off the UI thread.
///
/// # Errors
///
/// The [`LoadError`] of [`read_torrent`] for a file that cannot be offered.
pub(crate) fn load(item: AddItem) -> Result<LoadedItem, LoadError> {
    match item {
        AddItem::File(path) => read_torrent(path),
        AddItem::Magnet(link) => Ok(LoadedItem::Magnet(link)),
    }
}

/// Reads the `.torrent` file at `path` and parses it, once `stat` says it is a regular file of at
/// most [`MAX_TORRENT_FILE_BYTES`]. The file is checked again once it is open, and no more than one
/// byte past the limit is ever read from it.
///
/// # Errors
///
/// [`LoadError::NotAFile`] for anything but a regular file, [`LoadError::TooLarge`] for a file
/// larger than [`MAX_TORRENT_FILE_BYTES`], the [`LoadError`] of an I/O error for a file that
/// cannot be read, and [`LoadError::Invalid`] for one that is not a torrent the daemon can add.
pub(crate) fn read_torrent(path: PathBuf) -> Result<LoadedItem, LoadError> {
    // A FIFO would block the open itself, so it is turned away before.
    check_metadata(&std::fs::metadata(&path)?)?;
    let bytes = read_at_most(&path)?;
    LoadedItem::torrent(path, bytes).map_err(LoadError::Invalid)
}

/// Whether the file of `metadata` may be read: a regular file of at most
/// [`MAX_TORRENT_FILE_BYTES`].
fn check_metadata(metadata: &std::fs::Metadata) -> Result<(), LoadError> {
    if !metadata.is_file() {
        return Err(LoadError::NotAFile);
    }
    if metadata.len() > MAX_TORRENT_FILE_BYTES {
        return Err(LoadError::TooLarge {
            size: metadata.len(),
        });
    }
    Ok(())
}

/// The content of the regular file at `path`, of at most [`MAX_TORRENT_FILE_BYTES`].
fn read_at_most(path: &Path) -> Result<Arc<[u8]>, LoadError> {
    let file = open_without_waiting(path)?;
    // It may have been replaced since the first look.
    let metadata = file.metadata()?;
    check_metadata(&metadata)?;
    // Room for the whole file and the one byte that would tell it grew: one allocation.
    let room = usize::try_from(metadata.len())
        .unwrap_or(0)
        .saturating_add(1);
    let mut bytes = Vec::with_capacity(room);
    file.take(MAX_TORRENT_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > MAX_TORRENT_FILE_BYTES {
        return Err(LoadError::TooLarge { size });
    }
    Ok(Arc::from(bytes))
}

/// Opens the file at `path` for reading, without waiting on a FIFO for a writer.
fn open_without_waiting(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
    options.open(path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use x4_core::QUEUE_CAP;
    use x4_core::launch::RejectKind;
    use x4_rpc::MagnetLink;

    use super::*;

    /// "a": one file of 16 KiB, in one piece.
    const ONE_FILE: &[u8] =
        b"d4:infod6:lengthi16384e4:name1:a12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";

    #[test]
    fn a_regular_torrent_file_is_read_and_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.torrent");
        fs::write(&path, ONE_FILE).unwrap();

        let Ok(LoadedItem::Torrent(torrent)) = read_torrent(path.clone()) else {
            panic!("a torrent");
        };
        assert_eq!(torrent.path(), path);
        assert_eq!(torrent.bytes(), ONE_FILE);
        assert_eq!(torrent.parsed().name, "a");
    }

    #[test]
    fn only_a_regular_file_within_the_limit_is_read() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_torrent(dir.path().join("missing.torrent")).err(),
            Some(LoadError::Missing)
        );
        assert_eq!(
            read_torrent(dir.path().to_path_buf()).err(),
            Some(LoadError::NotAFile)
        );
        let huge = dir.path().join("huge.torrent");
        File::create(&huge)
            .unwrap()
            .set_len(MAX_TORRENT_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(
            read_torrent(huge).err(),
            Some(LoadError::TooLarge {
                size: MAX_TORRENT_FILE_BYTES + 1
            })
        );
        let at_the_limit = dir.path().join("limit.torrent");
        File::create(&at_the_limit)
            .unwrap()
            .set_len(MAX_TORRENT_FILE_BYTES)
            .unwrap();
        assert!(
            matches!(read_torrent(at_the_limit), Err(LoadError::Invalid(_))),
            "a file at the limit is read, and parsed"
        );
        let text = dir.path().join("text.torrent");
        fs::write(&text, b"not bencode").unwrap();
        assert!(matches!(
            read_torrent(text),
            Err(LoadError::Invalid(MetainfoError::NotBencode { .. }))
        ));
    }

    #[test]
    fn a_fifo_is_refused_without_being_opened() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo.torrent");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());

        // Opening it would wait for a writer forever.
        assert_eq!(read_torrent(fifo).err(), Some(LoadError::NotAFile));
    }

    #[test]
    fn a_fifo_opens_without_waiting_and_is_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo.torrent");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());

        // As if it had been swapped in after the first look: opened, and turned away.
        assert_eq!(read_at_most(&fifo).err(), Some(LoadError::NotAFile));
    }

    #[test]
    fn a_file_that_may_not_be_read_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("closed.torrent");
        fs::write(&path, ONE_FILE).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads it anyway; the test means something only for a user.
        if File::open(&path).is_err() {
            assert_eq!(read_torrent(path).err(), Some(LoadError::Denied));
        }
    }

    #[test]
    fn launch_arguments_make_one_request_of_items_and_one_of_refusals() {
        let link = format!("magnet:?xt=urn:btih:{}", "b".repeat(40));
        let args = [
            "show.torrent",
            "https://example.org/a.torrent",
            link.as_str(),
            "-x",
        ];

        assert_eq!(
            launch_requests(args, Path::new("/home/user")),
            [
                AddRequest::Items(vec![
                    AddItem::File("/home/user/show.torrent".into()),
                    AddItem::Magnet(Some(MagnetLink::parse(&link).unwrap())),
                ]),
                AddRequest::Rejected {
                    first: RejectedArg {
                        kind: RejectKind::Url
                    },
                    count: 2,
                },
            ]
        );
    }

    #[test]
    fn launch_arguments_of_one_kind_make_one_request_and_none_make_none() {
        let cwd = Path::new("/home/user");

        assert_eq!(
            launch_requests(["a.torrent"], cwd),
            [AddRequest::Items(vec![AddItem::File(
                "/home/user/a.torrent".into()
            )])]
        );
        assert_eq!(
            launch_requests(["-v"], cwd),
            [AddRequest::Rejected {
                first: RejectedArg {
                    kind: RejectKind::Option
                },
                count: 1,
            }]
        );
        assert_eq!(launch_requests::<&str>([], cwd), []);
    }

    #[test]
    fn the_first_double_dash_ends_the_options_and_is_skipped() {
        let cwd = Path::new("/home/user");

        assert_eq!(launch_requests(["--"], cwd), []);
        assert_eq!(
            launch_requests(["--", "a.torrent", "--"], cwd),
            [
                AddRequest::Items(vec![AddItem::File("/home/user/a.torrent".into())]),
                AddRequest::Rejected {
                    first: RejectedArg {
                        kind: RejectKind::Option
                    },
                    count: 1,
                },
            ],
            "only the first"
        );
    }

    #[test]
    fn a_million_launch_arguments_give_a_queueful_and_count_the_rest() {
        let args = (0..1_000_000).map(|n| format!("{n}.torrent"));

        let requests = launch_requests(args, Path::new("/home/user"));

        let [AddRequest::Capped { items, left_out }] = requests.as_slice() else {
            panic!("one capped request, not {} requests", requests.len());
        };
        assert_eq!(items.len(), QUEUE_CAP);
        assert_eq!(*left_out, 1_000_000 - QUEUE_CAP);
        assert_eq!(
            items.first(),
            Some(&AddItem::File("/home/user/0.torrent".into()))
        );
        assert_eq!(
            items.last(),
            Some(&AddItem::File("/home/user/99.torrent".into()))
        );
    }

    #[test]
    fn what_launch_arguments_build_is_bounded_by_the_queue_and_the_cwd() {
        // A working directory as long as Linux allows, and many short relative names: each item
        // joins the two.
        let cwd = PathBuf::from(format!("/{}", "d".repeat(4000)));
        let refused = std::iter::repeat_n("-x", 2_000);
        let files = std::iter::repeat_n("a", 10_000);

        let requests = launch_requests(refused.chain(files), &cwd);

        let [
            AddRequest::Capped { items, left_out },
            AddRequest::Rejected { first, count },
        ] = requests.as_slice()
        else {
            panic!(
                "a capped request and a refusal, not {} requests",
                requests.len()
            );
        };
        assert_eq!((items.len(), *left_out), (QUEUE_CAP, 10_000 - QUEUE_CAP));
        assert_eq!(
            (first.kind, *count),
            (RejectKind::Option, 2_000),
            "one refusal and a count"
        );
        let built: usize = items
            .iter()
            .map(|item| match item {
                AddItem::File(path) => path.as_os_str().len(),
                AddItem::Magnet(_) => 0,
            })
            .sum();
        assert_eq!(built, QUEUE_CAP * (cwd.as_os_str().len() + 2));
    }

    #[test]
    fn arguments_after_a_full_queue_are_only_counted() {
        let names: Vec<String> = (0..QUEUE_CAP).map(|n| format!("{n}.torrent")).collect();
        let args = names.iter().map(String::as_str).chain(["-x", "b.torrent"]);

        let requests = launch_requests(args, Path::new("/home/user"));

        assert!(
            matches!(
                requests.as_slice(),
                [AddRequest::Capped { items, left_out: 2 }] if items.len() == QUEUE_CAP
            ),
            "not told apart, so not refused either: {requests:?}"
        );
    }

    #[test]
    fn a_magnet_link_is_offered_as_it_is() {
        assert!(matches!(
            load(AddItem::Magnet(None)),
            Ok(LoadedItem::Magnet(None))
        ));
    }
}
