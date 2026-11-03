//! The outcome of an add: the torrent the daemon added or already had, or why it added nothing,
//! or that it is not known whether it did.

use std::time::Instant;

use x4_rpc::RpcError;

use super::{Engine, Event, Failure, Request, Response, unexpected};
use crate::action::{ActionId, AddError, Added};

impl Engine {
    /// Takes the answer to the add `request`, run as `id`, at `now`.
    ///
    /// A torrent added brings the next tick forward, which shows it; a duplicate changed
    /// nothing. An add that got no answer within its long timeout is unconfirmed and keeps the
    /// connection; any other failure says what it says about the connection, as an action's
    /// does.
    pub(super) fn add_finished(
        &mut self,
        now: Instant,
        request: &Request,
        id: ActionId,
        result: Result<Response, RpcError>,
    ) -> Vec<Event> {
        let sent_in = self.action_batches.remove(&id);
        let error = match result {
            Ok(Response::Added(outcome)) => {
                let added = Added::from(outcome);
                if added.duplicate {
                    tracing::info!("the daemon already had the torrent it was asked to add");
                } else {
                    tracing::info!("torrent added");
                    self.pull_tick_forward(sent_in);
                }
                tracing::debug!(torrent = %added.hash, duplicate = added.duplicate, "add answered");
                return vec![Event::AddFinished {
                    id,
                    result: Ok(added),
                }];
            }
            Ok(other) => unexpected(request, &other),
            Err(error) => error,
        };
        let reason = match Failure::of(request, &error) {
            Failure::Unanswered => {
                tracing::warn!(
                    "the daemon did not confirm the add in time; the next lists tell whether it \
                     added the torrent"
                );
                AddError::Unconfirmed
            }
            Failure::Unusable | Failure::Unsupported(_) | Failure::Refused => {
                tracing::error!(%error, "adding a torrent failed");
                AddError::of_call(&error)
            }
        };
        let mut events = vec![Event::AddFinished {
            id,
            result: Err(reason),
        }];
        events.extend(self.call_failed(now, request, &error));
        events
    }
}
