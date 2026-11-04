//! The event pump: the events of the service, taken on the UI thread's executor in batches.
//!
//! Awaiting the channel yields until the service sends something, so the pump never blocks the
//! UI thread. Whatever has queued up while the thread was busy is taken in one go and reduced to
//! the newest snapshot, so a slow frame never makes the window replay old states, and the view
//! draws again only when what it shows has changed. The pump runs in the window
//! (`spawn_in`), because the answer to an action or to a read of magnet links may be a failure to
//! show as a notification, which needs the window.

use gpui_kit::{AsyncWindowContext, Context, WeakEntity, Window};
use x4_core::Event;

use super::MainView;

impl MainView {
    /// Takes a batch of events, oldest first, and draws again if what the window shows changed.
    fn take_events(&mut self, events: Vec<Event>, window: &mut Window, cx: &mut Context<'_, Self>) {
        let mut changed = false;
        for event in events {
            match event {
                Event::Snapshot(snapshot) => changed |= self.show(snapshot, cx),
                Event::Details(details) => self.details_arrived(details, cx),
                Event::Peers(peers) => self.peers_arrived(peers, cx),
                Event::PeersCleared => self.peers_cleared(cx),
                Event::ActionFinished { id, result } => {
                    self.action_finished(id, &result, window, cx);
                }
                Event::MagnetLinks { id, result } => {
                    self.magnet_links_read(id, result, window, cx);
                }
                Event::AddFinished { id, result } => self.add_finished(id, result, window, cx),
                Event::FreeSpace { path, answer } => self.free_space_read(path, answer, cx),
            }
        }
        // After the batch, so that the dialog and the selection see its newest snapshot.
        self.follow_adds(window, cx);
        if changed {
            cx.notify();
        }
    }
}

/// Feeds the view the events of the service until the view is gone or the channel closes.
pub(super) async fn pump(
    events: async_channel::Receiver<Event>,
    view: WeakEntity<MainView>,
    cx: &mut AsyncWindowContext,
) {
    while let Ok(first) = events.recv().await {
        let mut batch = vec![first];
        // Only what is queued now, so a busy service cannot keep the loop from yielding.
        for _ in 0..events.len() {
            match events.try_recv() {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }
        let batch = newest_snapshot_only(batch);
        if view
            .update_in(cx, |view, window, cx| view.take_events(batch, window, cx))
            .is_err()
        {
            // The window was closed; dropping `events` tells the service so.
            return;
        }
    }
    if let Err(error) = view.update_in(cx, |view, window, cx| {
        view.service_stopped(cx);
        view.adds_service_stopped(window, cx);
        view.follow_adds(window, cx);
    }) {
        tracing::debug!(%error, "the service stopped after the window closed");
    }
}

/// `events` without the snapshots that a later one in it replaces; the other events keep their
/// order. Each snapshot holds the whole state, so only the newest needs to be shown.
fn newest_snapshot_only(mut events: Vec<Event>) -> Vec<Event> {
    let newest = events
        .iter()
        .rposition(|event| matches!(event, Event::Snapshot(_)));
    let mut index = 0;
    events.retain(|event| {
        let keep = !matches!(event, Event::Snapshot(_)) || Some(index) == newest;
        index += 1;
        keep
    });
    events
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use x4_core::{ActionError, ActionId, ConnectionState, Snapshot};

    use super::*;

    fn snapshot(rate_down: u64) -> Event {
        Event::Snapshot(Arc::new(Snapshot {
            torrents: Vec::new(),
            listed: true,
            session: None,
            free_space_bytes: None,
            rate_down,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }))
    }

    fn finished(id: u64) -> Event {
        Event::ActionFinished {
            id: ActionId(id),
            result: Err(ActionError::NotConnected),
        }
    }

    /// The download rates of the snapshots and the ids of the finished actions, in order.
    fn summary(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .map(|event| match event {
                Event::Snapshot(snapshot) => format!("snapshot {}", snapshot.rate_down),
                Event::ActionFinished { id, .. } => format!("action {}", id.0),
                Event::MagnetLinks { id, .. } => format!("magnet links {}", id.0),
                Event::AddFinished { id, .. } => format!("add {}", id.0),
                Event::FreeSpace { path, .. } => format!("free space {}", path.as_str()),
                Event::Details(_) => "details".to_owned(),
                Event::Peers(_) => "peers".to_owned(),
                Event::PeersCleared => "peers cleared".to_owned(),
            })
            .collect()
    }

    #[test]
    fn only_the_newest_snapshot_is_kept() {
        let events = newest_snapshot_only(vec![snapshot(1), snapshot(2), snapshot(3)]);

        assert_eq!(summary(&events), ["snapshot 3"]);
    }

    #[test]
    fn other_events_keep_their_order_around_the_newest_snapshot() {
        let events = newest_snapshot_only(vec![
            snapshot(1),
            finished(7),
            snapshot(2),
            finished(8),
            snapshot(3),
            finished(9),
        ]);

        assert_eq!(
            summary(&events),
            ["action 7", "action 8", "snapshot 3", "action 9"]
        );
    }

    #[test]
    fn a_batch_without_snapshots_is_left_alone() {
        assert_eq!(
            summary(&newest_snapshot_only(vec![finished(1), finished(2)])),
            ["action 1", "action 2"]
        );
        assert_eq!(
            summary(&newest_snapshot_only(Vec::new())),
            [] as [String; 0]
        );
    }
}
