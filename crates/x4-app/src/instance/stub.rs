//! The single instance where there are no Unix sockets: every launch runs on its own.

use std::convert::Infallible;
use std::io;
use std::path::Path;

use super::{Claim, Frame, Reply, StandaloneReason, Timeouts};
use crate::adding::AddRequest;

/// The first 4x4 of the user. There is none on this system: no value of this type exists.
#[derive(Debug)]
pub struct Listener(Infallible);

/// The first 4x4 of the user, while it answers later launches. There is none on this system.
#[derive(Debug)]
pub struct Serving(Infallible);

// The app drops it on every system, as the Unix one lets the next launch take over when dropped.
impl Drop for Serving {
    fn drop(&mut self) {
        match self.0 {}
    }
}

impl Listener {
    /// Answers later launches by sending their requests into `adds`; never called here.
    ///
    /// # Errors
    ///
    /// None: no listener exists on this system.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "same signature as the Unix listener's, which keeps the sender"
    )]
    pub fn serve(self, adds: async_channel::Sender<AddRequest>) -> io::Result<Serving> {
        let _ = adds;
        match self.0 {}
    }

    /// Answers the next launch with what `deliver` makes of its frame; never called here.
    ///
    /// # Errors
    ///
    /// None: no listener exists on this system.
    pub fn answer_next(&self, deliver: impl FnOnce(Frame) -> bool) -> io::Result<Reply> {
        let _ = deliver;
        match self.0 {}
    }

    /// Answers the next launch by sending its requests into `adds`; never called here.
    ///
    /// # Errors
    ///
    /// None: no listener exists on this system.
    pub fn answer_next_into(&self, adds: &async_channel::Sender<AddRequest>) -> io::Result<Reply> {
        let _ = adds;
        match self.0 {}
    }
}

/// [`claim_with`](super::claim_with) here: always on its own.
pub(super) fn claim(
    _app_id: &str,
    _runtime_dir: Option<&Path>,
    _frame: &Frame,
    _timeouts: Timeouts,
) -> Result<Claim, StandaloneReason> {
    Err(StandaloneReason::Unsupported)
}
