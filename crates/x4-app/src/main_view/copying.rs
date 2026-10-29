//! Copying the selected torrents, as the main view carries it out, and the items of the torrent
//! list's context menu.
//!
//! Ctrl+Shift+C and Copy hash write the hashes of the targets at once, from the list, also while
//! the daemon is not connected. Ctrl+C and Copy magnet link ask the daemon for the magnet links of
//! the targets (`Command::ReadMagnetLinks`) and write them when they arrive; the `clipboard`
//! module keeps the read that waits, so that only the newest copy writes. A failed read is shown
//! as a failed action is. A held key copies once.
//!
//! An item of the context menu acts on the rows the menu took when it opened, through the same
//! paths as the keys: an action through [`MainView::request_on`], which reads the availability
//! again, a removal through the dialog that asks first. Then the keys go back to the table, as
//! after a toolbar button.

use gpui_kit::{App, ClipboardItem, Context, Focusable as _, Window};
use x4_core::{ActionError, ActionId, Command, Row, availability};
use x4_rpc::HashString;

use super::MainView;
use crate::clipboard::{MagnetAnswer, hashes_text};
use crate::context_menu::MenuChoice;

/// The name of copying magnet links in logs and for its failure notification, which a newer
/// failure of a copy replaces.
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

    /// Takes the item `choice` of the list's context menu, for `targets`, the rows the menu took
    /// when it opened. The keys go back to the table first: a removal dialog then gives them back
    /// to it when it closes, and the menu, seeing the focus gone elsewhere, does not take them back
    /// to where they were before it opened.
    pub(crate) fn choose_from_menu(
        &mut self,
        choice: MenuChoice,
        targets: &[Row],
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        window.focus(&self.table.focus_handle(cx), cx);
        let targets: Vec<&Row> = targets.iter().collect();
        tracing::debug!(?choice, count = targets.len(), "chosen in the context menu");
        match choice {
            MenuChoice::CopyHashes => {
                if let Some((text, count)) = self.hashes_to_copy(&targets) {
                    write_to_clipboard(text, count, "hashes", cx);
                }
            }
            MenuChoice::CopyMagnetLinks => {
                if let Some(hashes) = self.links_to_read(&targets) {
                    self.read_magnet_links(hashes, window, cx);
                }
            }
            MenuChoice::Remove(data) => self.ask_removal_of(data, &targets, window, cx),
            MenuChoice::Start
            | MenuChoice::Pause
            | MenuChoice::StartNow
            | MenuChoice::Verify
            | MenuChoice::Reannounce => {
                if let Some(request) = choice.request() {
                    self.request_on(request, &targets, window, cx);
                }
            }
        }
    }

    /// The text that copies the hashes of `targets`, and how many there are; `None` (logged) when
    /// they do not allow it.
    fn hashes_to_copy(&self, targets: &[&Row]) -> Option<(String, usize)> {
        if !availability(self.status.connected(), targets).copy_hash {
            tracing::debug!("no torrent to copy the hash of");
            return None;
        }
        Some((hashes_text(targets), targets.len()))
    }

    /// The hashes whose magnet links to read for `targets`, in order; `None` (logged) when they
    /// do not allow it.
    fn links_to_read(&self, targets: &[&Row]) -> Option<Vec<HashString>> {
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
    fn read_magnet_links(
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

    /// Takes the answer to the read of magnet links `id`: copies the links, shows the failure, or
    /// does nothing when a newer copy has taken the read's place.
    pub(super) fn magnet_links_read(
        &mut self,
        id: ActionId,
        result: Result<Vec<(HashString, String)>, ActionError>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self.magnet.finish(id, result) {
            MagnetAnswer::Copy { text, count } => {
                write_to_clipboard(text, count, "magnet links", cx);
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
fn write_to_clipboard(text: String, count: usize, what: &'static str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    tracing::info!(count, what, "copied to the clipboard");
}
