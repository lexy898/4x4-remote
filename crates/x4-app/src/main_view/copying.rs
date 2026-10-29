//! Copying the selected torrents to the clipboard, as the main view carries it out, from the keys
//! and from the list's context menu (the `menu` module).
//!
//! Ctrl+Shift+C and Copy hash write the hashes of the targets at once, from the list, also while
//! the daemon is not connected. Ctrl+C and Copy magnet link ask the daemon for the magnet links of
//! the targets (`Command::ReadMagnetLinks`) and write them when they arrive; the `clipboard`
//! module keeps the read that waits, so that only the newest copy writes. A failed read is shown
//! as a failed action is, and so is an answer that lacks some of the links (the ones that came are
//! copied). A held key copies once.
//!
//! On Wayland only the window in focus may set the clipboard, and gpui drops the text silently
//! otherwise. The links arrive a moment after the key press, so if 4x4 is no longer the active
//! window by then, nothing is written and a notice asks to copy them again. A later copy that
//! succeeds in full takes down that notice, and the ones about missing links and failed reads.

use gpui_kit::{App, ClipboardItem, Context, Window};
use x4_core::{ActionError, ActionId, Command, Row, availability};
use x4_rpc::HashString;

use super::MainView;
use crate::clipboard::{MagnetAnswer, hashes_text};
use crate::i18n::t;

/// The name of copying magnet links for its notifications: a newer one replaces the older.
const COPY_MAGNET_LINKS: &str = "copy magnet links";

impl MainView {
    /// Copies the hashes of the selected torrents that the list shows now; nothing when none is.
    pub(super) fn copy_selected_hashes(&self, cx: &mut App) {
        let text = {
            let table = self.table.read(cx).delegate();
            self.hashes_to_copy(&table.selection().targets(table.rows()))
        };
        if let Some((text, count)) = text {
            write_to_clipboard(text, count, "hashes", cx);
        }
    }

    /// Asks the daemon for the magnet links of the selected torrents that the list shows now, to
    /// copy them when they arrive; nothing while the daemon is not connected.
    pub(super) fn copy_selected_magnet_links(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let hashes = {
            let table = self.table.read(cx).delegate();
            self.links_to_read(&table.selection().targets(table.rows()))
        };
        if let Some(hashes) = hashes {
            self.read_magnet_links(hashes, window, cx);
        }
    }

    /// The text that copies the hashes of `targets`, and how many there are; `None` (logged) when
    /// they do not allow it.
    pub(super) fn hashes_to_copy(&self, targets: &[&Row]) -> Option<(String, usize)> {
        if !availability(self.status.connected(), targets).copy_hash {
            tracing::debug!("no torrent to copy the hash of");
            return None;
        }
        Some((hashes_text(targets), targets.len()))
    }

    /// The hashes whose magnet links to read for `targets`, in order; `None` (logged) when they
    /// do not allow it.
    pub(super) fn links_to_read(&self, targets: &[&Row]) -> Option<Vec<HashString>> {
        if !availability(self.status.connected(), targets).copy_magnet {
            tracing::debug!(
                count = targets.len(),
                "no magnet link can be read now; nothing asked"
            );
            return None;
        }
        Some(targets.iter().map(|row| row.hash.clone()).collect())
    }

    /// Asks the daemon for the magnet links of `hashes`, which then take the place of any read
    /// that waits, unless that read is of the same torrents.
    pub(super) fn read_magnet_links(
        &mut self,
        hashes: Vec<HashString>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let count = hashes.len();
        let pending = &mut self.pending;
        let Some(id) = self.magnet.start(&hashes, || pending.next_id()) else {
            tracing::debug!(
                count,
                "the same magnet links are on their way; nothing asked"
            );
            return;
        };
        tracing::info!(count, "reading magnet links to copy");
        if self
            .commands
            .send(Command::ReadMagnetLinks(id, hashes))
            .is_err()
        {
            tracing::warn!("the service has stopped; the magnet links were not asked for");
            self.magnet_links_read(id, Err(ActionError::NotConnected), window, cx);
        }
    }

    /// Takes the answer to the read of magnet links `id`: copies the links (with a notice when
    /// some are missing), shows the failure, or does nothing when a newer copy has taken the read's
    /// place. Links that arrive while 4x4 is not the active window are not copied; a notice asks
    /// to copy them again.
    pub(super) fn magnet_links_read(
        &mut self,
        id: ActionId,
        result: Result<Vec<(HashString, String)>, ActionError>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self.magnet.finish(id, result) {
            MagnetAnswer::Copy { count, .. } if !window.is_window_active() => {
                tracing::warn!(
                    count,
                    "the magnet links came while 4x4 was not in focus; they were not copied"
                );
                self.show_warning(
                    COPY_MAGNET_LINKS,
                    t!("copy-magnet-not-in-focus").into(),
                    window,
                    cx,
                );
            }
            MagnetAnswer::Copy {
                text,
                count,
                partial,
            } => {
                write_to_clipboard(text, count, "magnet links", cx);
                match partial {
                    Some(partial) => self.show_warning(COPY_MAGNET_LINKS, partial, window, cx),
                    // Whatever an earlier copy said ("copy them again", "Copied 2 of 3") no
                    // longer holds, and the notice would otherwise stay until dismissed.
                    None => Self::hide_notice(COPY_MAGNET_LINKS, window, cx),
                }
            }
            MagnetAnswer::Failed(text) => self.show_failure(COPY_MAGNET_LINKS, text, window, cx),
            MagnetAnswer::Outdated => {}
        }
    }
}

/// Writes `text`, the copy of `count` torrents, to the clipboard, and logs `what` was copied
/// ("hashes", "magnet links").
///
/// gpui says nothing when the system does not take it: on Wayland only a window with the keyboard
/// focus may set the clipboard.
pub(super) fn write_to_clipboard(text: String, count: usize, what: &'static str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    tracing::info!(count, what, "copied to the clipboard");
}
