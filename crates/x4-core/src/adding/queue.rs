//! What is waiting to be added, in the order it came, one dialog at a time.

use std::collections::VecDeque;
use std::path::PathBuf;

use x4_rpc::MagnetLink;

/// The most items an [`AddQueue`] holds. More are dropped, and the app says how many.
pub const QUEUE_CAP: usize = 100;

/// Something the user asked to add, before it is read.
///
/// A `.torrent` is only named by its path here. It is read when its turn comes, so the queue never
/// holds the contents of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddItem {
    /// A `.torrent` file on this computer.
    File(PathBuf),
    /// A magnet link; `None` for the magnet form with an empty link, which the user fills in.
    Magnet(Option<MagnetLink>),
}

/// The items waiting to be added. The head is shown in the add dialog; Add or Cancel moves on to
/// the next one.
///
/// It holds at most [`QUEUE_CAP`] items, and each item once while it waits.
#[derive(Debug, Clone, Default)]
pub struct AddQueue {
    waiting: VecDeque<AddItem>,
    /// How many items of the current run were added or skipped; 0 again once the queue is empty.
    done: usize,
}

impl AddQueue {
    /// Appends `items`, in their order, and returns how many of them were dropped because the
    /// queue already held [`QUEUE_CAP`] items.
    ///
    /// An item equal to one that is waiting already is not queued twice, and is not counted as
    /// dropped: the user sees its dialog once.
    pub fn push(&mut self, items: Vec<AddItem>) -> usize {
        let mut dropped = 0;
        for item in items {
            if self.waiting.contains(&item) {
                continue;
            }
            if self.waiting.len() < QUEUE_CAP {
                self.waiting.push_back(item);
            } else {
                dropped += 1;
            }
        }
        dropped
    }

    /// The item shown now; `None` when nothing waits.
    #[must_use]
    pub fn head(&self) -> Option<&AddItem> {
        self.waiting.front()
    }

    /// Moves on to the next item, after the head was added or the user skipped it. Nothing
    /// happens when nothing waits.
    pub fn skip(&mut self) {
        if self.waiting.pop_front().is_some() {
            self.done += 1;
        }
        if self.waiting.is_empty() {
            self.done = 0;
        }
    }

    /// Where the head is in the current run, for a title such as "Add torrent (2 of 5)": its
    /// number, counted from 1, and how many items the run has, those done and those waiting.
    /// `None` when nothing waits; the next item then starts a new run.
    #[must_use]
    pub fn position(&self) -> Option<(usize, usize)> {
        (!self.waiting.is_empty()).then(|| (self.done + 1, self.done + self.waiting.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(number: usize) -> AddItem {
        AddItem::File(PathBuf::from(format!("/home/user/{number}.torrent")))
    }

    fn magnet(hash_digit: char) -> AddItem {
        let hash: String = std::iter::repeat_n(hash_digit, 40).collect();
        AddItem::Magnet(Some(
            MagnetLink::parse(&format!("magnet:?xt=urn:btih:{hash}")).unwrap(),
        ))
    }

    /// Skips every item and returns them in the order they were shown.
    fn drain(queue: &mut AddQueue) -> Vec<AddItem> {
        let mut shown = Vec::new();
        while let Some(head) = queue.head() {
            shown.push(head.clone());
            queue.skip();
        }
        shown
    }

    #[test]
    fn the_queue_keeps_order_caps_at_100_and_reports_the_overflow() {
        assert_eq!(QUEUE_CAP, 100);
        let mut queue = AddQueue::default();
        assert_eq!(queue.head(), None);
        assert_eq!(queue.position(), None);

        let first = vec![file(0), magnet('a'), AddItem::Magnet(None)];
        assert_eq!(queue.push(first.clone()), 0);
        assert_eq!(queue.head(), Some(&file(0)));
        assert_eq!(queue.position(), Some((1, 3)));
        queue.skip();
        assert_eq!(queue.head(), Some(&magnet('a')));
        assert_eq!(queue.position(), Some((2, 3)), "the one done still counts");

        // Two wait; 98 more fit, and the other 22 are dropped.
        let more: Vec<AddItem> = (1..=120).map(file).collect();
        assert_eq!(queue.push(more.clone()), 22);
        assert_eq!(queue.position(), Some((2, 101)));
        assert_eq!(queue.push(vec![file(500)]), 1, "full");

        let shown = drain(&mut queue);
        let expected: Vec<AddItem> = first
            .into_iter()
            .skip(1)
            .chain(more.into_iter().take(98))
            .collect();
        assert_eq!(
            shown, expected,
            "in the order they came, the first ones kept"
        );
        assert_eq!(queue.head(), None);
        assert_eq!(queue.position(), None);

        // An empty queue starts a new run, and skipping it changes nothing.
        queue.skip();
        assert_eq!(queue.position(), None);
        assert_eq!(queue.push(vec![magnet('b'), magnet('c')]), 0);
        assert_eq!(queue.position(), Some((1, 2)));
        queue.skip();
        assert_eq!(queue.position(), Some((2, 2)));
        queue.skip();
        assert_eq!(queue.push(vec![file(7)]), 0);
        assert_eq!(queue.position(), Some((1, 1)));
    }

    #[test]
    fn an_item_that_waits_already_is_not_queued_twice() {
        let mut queue = AddQueue::default();

        assert_eq!(queue.push(vec![file(1), file(1), magnet('a'), file(2)]), 0);
        assert_eq!(queue.push(vec![magnet('a'), file(2), file(3)]), 0);
        assert_eq!(queue.position(), Some((1, 4)));

        // Once shown and moved on from, it can be asked for again.
        queue.skip();
        assert_eq!(queue.push(vec![file(1)]), 0);
        assert_eq!(drain(&mut queue), [magnet('a'), file(2), file(3), file(1)]);

        // A duplicate is not counted as dropped, even when the queue is full.
        let full: Vec<AddItem> = (0..QUEUE_CAP).map(file).collect();
        assert_eq!(queue.push(full), 0);
        assert_eq!(
            queue.push(vec![file(0), file(QUEUE_CAP - 1), file(QUEUE_CAP)]),
            1
        );
    }
}
