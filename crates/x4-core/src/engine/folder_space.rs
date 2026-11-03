//! The free space of a folder the user is choosing ([`Command::FreeSpace`]), read once per
//! question: of the folder itself or, when it does not exist yet, of the nearest parent that
//! does. The daemon creates a missing folder with the first file it writes into it, so a missing
//! folder is a valid choice, and the room in its parent is the room it will have.
//!
//! The daemon answers one request at a time, on a Raspberry Pi whose disk may sleep, so this
//! asks little: at most one call is out, a newer question replaces an older one that has not
//! been answered yet, and one question makes at most [`MAX_CALLS`] calls.
//!
//! [`Command::FreeSpace`]: super::Command::FreeSpace

use std::time::Instant;

use x4_rpc::{DownloadDir, RpcError};

use super::{Engine, Event, Request, Response, unexpected};

/// How many `free_space` calls one question may make: the folder and up to two of its parents.
pub(super) const MAX_CALLS: usize = 3;

/// The JSON-RPC error code of `free_space` for a folder it cannot look at, with the system's
/// message as its detail: in practice "No such file or directory" (T/rpcimpl.cc:2753-2760).
const SYSTEM_ERROR: i64 = 5;

impl Engine {
    /// Takes what the call about the free space of `folder` brought at `now`. A folder the
    /// daemon cannot look at counts as missing, and the question walks on to its parent; any
    /// other failure says what it says about the connection, as a poll's does.
    pub(super) fn folder_space_read(
        &mut self,
        now: Instant,
        request: &Request,
        folder: &DownloadDir,
        result: Result<Response, RpcError>,
    ) -> Vec<Event> {
        let (reading, failure) = match result {
            // The daemon reports -1 for a size it could not read.
            Ok(Response::FreeSpace(space)) => {
                (Reading::Exists(u64::try_from(space.size_bytes).ok()), None)
            }
            Ok(other) => (Reading::Failed, Some(unexpected(request, &other))),
            Err(RpcError::Server {
                code: SYSTEM_ERROR, ..
            }) => (Reading::Missing, None),
            Err(error) => (Reading::Failed, Some(error)),
        };
        let mut events: Vec<Event> = self
            .folder_space
            .answered(folder, reading)
            .into_iter()
            .collect();
        if let Some(error) = failure {
            tracing::warn!(%error, "could not read the free space of a folder");
            events.extend(self.call_failed(now, request, &error));
        }
        events
    }
}

/// What the daemon said about the room in a folder, the answer to a
/// [`Command::FreeSpace`](super::Command::FreeSpace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreeSpaceAnswer {
    /// The folder exists, with this much free in it.
    Free {
        /// The free space, in bytes.
        bytes: u64,
    },
    /// The folder does not exist yet; the daemon creates it with the first file it writes into
    /// it.
    ///
    /// The daemon reports every system error about a folder with one code, so a folder it cannot
    /// look into for another reason, or a file of that name, counts as missing too.
    Missing {
        /// The nearest parent that exists, found within the calls one question may make; `None`
        /// when none was.
        parent: Option<DownloadDir>,
        /// The free space in `parent`, in bytes; `None` when there is no parent, or when the
        /// daemon could not read its size.
        free: Option<u64>,
    },
    /// The free space is not known: the daemon is not connected, refused to tell, or did not
    /// answer.
    Unknown,
}

/// What one `free_space` call said about the folder it asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reading {
    /// The folder exists; its free space, `None` when the daemon could not read it.
    Exists(Option<u64>),
    /// The daemon could not look at the folder: in practice, it does not exist.
    Missing,
    /// The call failed otherwise.
    Failed,
}

/// The newest free-space question not answered yet, and the call out, if any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct FolderSpace {
    question: Option<Question>,
    /// The folder of the call handed out and not answered yet. It may belong to a question that
    /// a newer one has replaced since: the newer one waits for it.
    out: Option<DownloadDir>,
}

/// One question and how far up it has walked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Question {
    /// The folder the user asked about.
    path: DownloadDir,
    /// The folder to ask the daemon about next, or being asked about: `path`, then its parents.
    asking: DownloadDir,
    /// The calls handed out for the question so far.
    calls: usize,
    /// Whether the call about `asking` has been handed out.
    sent: bool,
}

impl FolderSpace {
    /// Takes a question about `path`, which replaces one not answered yet; that one is never
    /// answered. A question about the folder already asked about changes nothing.
    pub(super) fn ask(&mut self, path: DownloadDir) {
        if self
            .question
            .as_ref()
            .is_some_and(|question| question.path == path)
        {
            return;
        }
        self.question = Some(Question {
            asking: path.clone(),
            path,
            calls: 0,
            sent: false,
        });
    }

    /// Whether a call is due: a question waits for one, and no call is out.
    pub(super) fn is_due(&self) -> bool {
        self.out.is_none()
            && self
                .question
                .as_ref()
                .is_some_and(|question| !question.sent)
    }

    /// The folder to ask the daemon about now, if a call is due; it counts as out from now on.
    pub(super) fn hand_out(&mut self) -> Option<DownloadDir> {
        if !self.is_due() {
            return None;
        }
        let question = self.question.as_mut()?;
        question.sent = true;
        question.calls += 1;
        self.out = Some(question.asking.clone());
        Some(question.asking.clone())
    }

    /// Takes what the call about `folder` read; returns the answer to the question when this
    /// ends it, `None` when the question walks on to a parent, when a newer question replaced
    /// the one it was for, or when no such call is out (it was dropped with the connection).
    fn answered(&mut self, folder: &DownloadDir, reading: Reading) -> Option<Event> {
        if self.out.as_ref() != Some(folder) {
            tracing::debug!("ignored the free space of a folder no call is out for");
            return None;
        }
        self.out = None;
        let Some(mut question) = self.question.take_if(|question| question.sent) else {
            tracing::debug!("dropped the free space of a folder asked about before the newest");
            return None;
        };
        let asked_itself = question.asking == question.path;
        let answer = match reading {
            Reading::Exists(Some(bytes)) if asked_itself => FreeSpaceAnswer::Free { bytes },
            Reading::Exists(None) if asked_itself => FreeSpaceAnswer::Unknown,
            Reading::Exists(free) => FreeSpaceAnswer::Missing {
                parent: Some(question.asking),
                free,
            },
            Reading::Missing => match question.asking.parent() {
                Some(parent) if question.calls < MAX_CALLS => {
                    question.asking = parent;
                    question.sent = false;
                    self.question = Some(question);
                    return None;
                }
                Some(_) | None => FreeSpaceAnswer::Missing {
                    parent: None,
                    free: None,
                },
            },
            // Known to be missing, though no parent could be read.
            Reading::Failed if !asked_itself => FreeSpaceAnswer::Missing {
                parent: None,
                free: None,
            },
            Reading::Failed => FreeSpaceAnswer::Unknown,
        };
        Some(Event::FreeSpace {
            path: question.path,
            answer,
        })
    }

    /// The daemon is no longer connected: the call out is forgotten, since the worker no longer
    /// sends it, and the question not answered yet is answered as unknown.
    pub(super) fn disconnected(&mut self) -> Option<Event> {
        self.out = None;
        self.question.take().map(|question| Event::FreeSpace {
            path: question.path,
            answer: FreeSpaceAnswer::Unknown,
        })
    }
}
