//! Copying the selected torrents to the clipboard: their info hashes, which the list has, at once;
//! their magnet links, which only the daemon has, once it has sent them ([`MagnetCopy`] keeps the
//! read that waits for its answer).
//!
//! Both are written one torrent per line, in the order of the targets, which is the order the list
//! shows them in.
//!
//! One read of magnet links waits at a time. A newer copy takes its place, and the answer to the
//! older read is then ignored by its id, so the clipboard ends up with what the user asked for
//! last, whichever answer comes first. A copy of the same torrents, in the same order, while their
//! read waits reads nothing more: the answer on its way writes the same text. An answer without a
//! single link (the daemon no longer has any of the torrents) is a failure to show, not an empty
//! clipboard; an answer with some of them is copied, and a notice says how many are missing.
//!
//! On Wayland the copied text stays in 4x4, which hands it to the app that pastes it: it is lost
//! when 4x4 quits, unless a clipboard manager keeps it.

use gpui_kit::SharedString;
use x4_core::{ActionError, ActionId, Row};
use x4_rpc::HashString;

use crate::i18n::{magnet_failure_text, magnet_partial_text};

/// The text that copying the hashes of `targets` writes: their hashes, one per line, in order.
pub(crate) fn hashes_text(targets: &[&Row]) -> String {
    lines(targets.iter().map(|row| row.hash.as_str()))
}

/// `items`, one per line.
fn lines<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items.collect::<Vec<_>>().join("\n")
}

/// Why a copy of magnet links failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MagnetFailure {
    /// The daemon answered, and has none of the torrents any more.
    Gone,
    /// The read failed, or was refused before it went out.
    Error(ActionError),
}

/// What the answer to a read of magnet links comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MagnetAnswer {
    /// The links, one per line, for the clipboard.
    Copy {
        /// The text to write.
        text: String,
        /// How many links it has.
        count: usize,
        /// What a notice says when the daemon no longer has some of the torrents, so that their
        /// links are missing; `None` when every link came.
        partial: Option<SharedString>,
    },
    /// The copy failed; what the notification says.
    Failed(SharedString),
    /// Not the read that waits: an older one, whose place a newer copy took.
    Outdated,
}

/// The read of magnet links that waits for its answer, if any.
#[derive(Debug, Default)]
pub(crate) struct MagnetCopy {
    pending: Option<PendingRead>,
}

/// A read of magnet links sent and not answered yet.
#[derive(Debug)]
struct PendingRead {
    id: ActionId,
    /// The torrents asked for, in order.
    hashes: Vec<HashString>,
}

impl MagnetCopy {
    /// Starts a read of the magnet links of `hashes` under the id that `new_id` gives, and returns
    /// the id. The read that waited, if any, is forgotten: its answer will be ignored. Returns
    /// `None`, and takes no id, when the read that waits is of the same torrents in the same
    /// order, as its answer writes the same text.
    pub(crate) fn start(
        &mut self,
        hashes: &[HashString],
        new_id: impl FnOnce() -> ActionId,
    ) -> Option<ActionId> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.hashes == hashes)
        {
            return None;
        }
        let id = new_id();
        self.pending = Some(PendingRead {
            id,
            hashes: hashes.to_vec(),
        });
        Some(id)
    }

    /// Takes the answer `result` to the read `id`: the links to copy, the failure to show, or
    /// nothing when it is not the read that waits.
    ///
    /// Links come in the order they were asked for, without the torrents the daemon no longer has;
    /// the ones that came are copied, with a notice of how many are missing. None at all is a
    /// failure.
    pub(crate) fn finish(
        &mut self,
        id: ActionId,
        result: Result<Vec<(HashString, String)>, ActionError>,
    ) -> MagnetAnswer {
        let Some(pending) = self.pending.take_if(|pending| pending.id == id) else {
            tracing::debug!(?id, "ignored the answer to an older read of magnet links");
            return MagnetAnswer::Outdated;
        };
        let asked = pending.hashes.len();
        let failure = match result {
            Ok(links) if !links.is_empty() => {
                let count = links.len();
                let partial = (count < asked).then(|| {
                    tracing::warn!(
                        asked,
                        read = count,
                        "the daemon no longer has some of the torrents; their links are not copied"
                    );
                    magnet_partial_text(count, asked)
                });
                return MagnetAnswer::Copy {
                    text: lines(links.iter().map(|(_, link)| link.as_str())),
                    count,
                    partial,
                };
            }
            Ok(_) => {
                tracing::warn!(
                    asked,
                    "the daemon has none of the torrents any more; no magnet link copied"
                );
                MagnetFailure::Gone
            }
            Err(error) => {
                tracing::error!(asked, %error, "copying magnet links failed");
                MagnetFailure::Error(error)
            }
        };
        MagnetAnswer::Failed(magnet_failure_text(asked, &failure))
    }
}

#[cfg(test)]
mod tests {
    use x4_core::ConnectionError;
    use x4_core::test_support::torrent;

    use super::*;

    fn hash(name: &str) -> HashString {
        torrent(name).hash
    }

    fn link(name: &str) -> (HashString, String) {
        (hash(name), format!("magnet:?xt=urn:btih:{}", hash(name)))
    }

    #[test]
    fn hashes_are_copied_one_per_line_in_order() {
        let rows = [Row::from(&torrent("b")), Row::from(&torrent("a"))];
        let targets: Vec<&Row> = rows.iter().collect();

        assert_eq!(
            hashes_text(&targets),
            format!("{}\n{}", hash("b"), hash("a"))
        );
        assert_eq!(hashes_text(&targets[..1]), hash("b").to_string());
    }

    #[test]
    fn the_links_that_arrive_are_copied_in_their_order() {
        let mut copy = MagnetCopy::default();
        let id = copy.start(&[hash("b"), hash("a"), hash("c")], || ActionId(4));
        assert_eq!(id, Some(ActionId(4)));

        // c is gone from the daemon.
        let answer = copy.finish(ActionId(4), Ok(vec![link("b"), link("a")]));

        assert_eq!(
            answer,
            MagnetAnswer::Copy {
                text: format!("{}\n{}", link("b").1, link("a").1),
                count: 2,
                partial: Some(
                    "Copied 2 of 3 magnet links; 1 torrent is no longer on the server".into()
                ),
            }
        );
        assert_eq!(
            copy.finish(ActionId(4), Ok(vec![link("b")])),
            MagnetAnswer::Outdated,
            "answered once"
        );
    }

    #[test]
    fn a_newer_read_makes_the_older_answer_outdated() {
        let mut copy = MagnetCopy::default();
        let older = copy.start(&[hash("a")], || ActionId(1));
        let newer = copy.start(&[hash("b")], || ActionId(2));
        assert_eq!((older, newer), (Some(ActionId(1)), Some(ActionId(2))));

        assert_eq!(
            copy.finish(ActionId(1), Ok(vec![link("a")])),
            MagnetAnswer::Outdated
        );
        assert_eq!(
            copy.finish(ActionId(1), Err(ActionError::NotConnected)),
            MagnetAnswer::Outdated,
            "an older failure is not shown either"
        );
        assert!(matches!(
            copy.finish(ActionId(2), Ok(vec![link("b")])),
            MagnetAnswer::Copy {
                count: 1,
                partial: None,
                ..
            }
        ));
    }

    #[test]
    fn the_same_read_is_not_sent_again_while_it_waits() {
        let mut copy = MagnetCopy::default();
        let both = [hash("a"), hash("b")];
        assert_eq!(copy.start(&both, || ActionId(1)), Some(ActionId(1)));

        assert_eq!(
            copy.start(&both, || panic!("no new id for the same read")),
            None
        );
        // In another order the text differs, so it is read again.
        assert_eq!(
            copy.start(&[hash("b"), hash("a")], || ActionId(2)),
            Some(ActionId(2))
        );

        // Answered, the same torrents can be read again.
        copy.finish(ActionId(2), Ok(vec![link("b"), link("a")]));
        assert_eq!(copy.start(&both, || ActionId(3)), Some(ActionId(3)));
    }

    #[test]
    fn no_link_at_all_or_an_error_is_a_failure() {
        let mut copy = MagnetCopy::default();
        copy.start(&[hash("a"), hash("b")], || ActionId(1));
        assert_eq!(
            copy.finish(ActionId(1), Ok(Vec::new())),
            MagnetAnswer::Failed(
                "Could not copy the magnet links of 2 torrents: they are no longer on the server"
                    .into()
            )
        );

        copy.start(&[hash("a")], || ActionId(2));
        assert_eq!(
            copy.finish(
                ActionId(2),
                Err(ActionError::Rpc(ConnectionError::Forbidden))
            ),
            MagnetAnswer::Failed("Could not copy the magnet link: access denied".into())
        );
    }
}
