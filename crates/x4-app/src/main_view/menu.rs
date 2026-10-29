//! The items of the torrent list's context menu, as the main view carries them out.
//!
//! An item acts on the rows the menu took when it opened, through the same paths as the keys: an
//! action through [`MainView::request_on`], which reads the availability again; a copy through the
//! `copying` module; a removal through the dialog that asks first. The keys go back to the table,
//! as after a toolbar button.

use gpui_kit::{Context, Focusable as _, Window};
use x4_core::Row;

use super::MainView;
use super::copying::write_to_clipboard;
use crate::context_menu::MenuChoice;

impl MainView {
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
}
