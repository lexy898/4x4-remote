//! The sources of adds inside the window: the toolbar's Add file and Add magnet, their keys Ctrl+O
//! and Ctrl+U, Ctrl+V in the list, the file dialog, the clipboard and files dropped on the window
//! or on the add dialog.
//!
//! Each only sends a request into the window's own channel ([`MainView::add_requests`]), so that
//! every request goes the one way, and nothing is added without the user's Add. x4-core decides
//! what each hands over: [`dropped_files`] keeps the `.torrent` files of a drop and counts the
//! others, which one notification names; [`clipboard_magnets`] takes the magnet links on the
//! clipboard, no more than the queue holds, and [`magnet_form_items`] what Add magnet queues.
//! Every local file the file dialog chose joins the queue, and one notification counts those not
//! on this computer. One file dialog is open at a time, until it is answered, however long that
//! takes: a press meanwhile says that it is open; a cancelled one says nothing, and one that
//! cannot be opened says so. A drop of torrents from another app brings the window to the front,
//! for the add dialog to have the keyboard. Each key acts once per press.
//!
//! The clipboard is read only when the user asks; on Wayland a read of another app's text waits
//! on that app, for a few seconds at most.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Context, Div, Focusable as _, InteractiveElement as _, SharedString, Window};
use x4_core::{
    AddItem, ChosenFiles, ClipboardItems, DroppedFiles, clipboard_magnets, dropped_files,
    magnet_form_items,
};

use super::MainView;
use crate::adding::{AddRequest, FilePicker, Picked};
use crate::i18n::{chosen_not_local_text, dropped_others_text, t};
use crate::key_repeats::{KeyRepeats, once_per_press_of};
use crate::keymap::{AddMagnetLink, AddTorrentFiles, PasteMagnetLinks};

/// A button of the toolbar that adds torrents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddButton {
    /// Add file: the file dialog.
    Files,
    /// Add magnet: the magnet links on the clipboard, or the form for one.
    Magnet,
}

impl MainView {
    /// Sets how the window asks for `.torrent` files: the app sets the desktop's own file dialog
    /// on Linux, and a test may set one of its own. Until then it is gpui's dialog, which the test
    /// platform lets a test answer.
    pub fn set_file_picker(&mut self, picker: Rc<dyn FilePicker>) {
        self.adds.picker = picker;
    }

    /// Handles a press of a button of the toolbar that adds torrents: hands the keyboard focus
    /// back to the table, as the other buttons of the toolbar do, then carries out `button`.
    pub(crate) fn press_add_button(
        &mut self,
        button: AddButton,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        window.focus(&self.table.focus_handle(cx), cx);
        match button {
            AddButton::Files => self.add_torrent_files(window, cx),
            AddButton::Magnet => self.add_magnet_link(cx),
        }
    }

    /// Opens the file dialog that chooses `.torrent` files over the window; the files chosen join
    /// the queue once it is answered. While one is open, however long, no other opens, and a
    /// notification says so: the one open still gives its answer.
    fn add_torrent_files(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.adds.picking.is_some() {
            tracing::debug!("the file dialog is open already; no other opens");
            let text = SharedString::from(t!("add-file-picker-open"));
            // One notice, however often the user presses.
            self.add_notice(
                |text| Notification::info(text).id::<FilePickerOpen>(),
                text,
                window,
                cx,
            );
            return;
        }
        tracing::debug!("the file dialog opens");
        let pick = self.adds.picker.pick(window, cx);
        self.adds.picking = Some(cx.spawn_in(window, async move |view, cx| {
            let picked = pick.await;
            if let Err(error) = view.update_in(cx, |view, window, cx| {
                view.files_picked(picked, window, cx);
            }) {
                tracing::debug!(%error, "the file dialog was answered after the window closed");
            }
        }));
    }

    /// Takes the file dialog's answer `picked`: the local files chosen join the queue, in order,
    /// and one notification counts the others; a cancelled dialog says nothing; one that could
    /// not be opened says so.
    fn files_picked(&mut self, picked: Picked, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.adds.picking = None;
        match picked {
            Ok(Some(ChosenFiles { paths, not_local })) => {
                tracing::info!(
                    count = paths.len(),
                    not_local,
                    "torrent files were chosen to add"
                );
                if not_local > 0 {
                    let text = chosen_not_local_text(not_local);
                    self.add_notice(Notification::warning, text, window, cx);
                }
                if !paths.is_empty() {
                    self.request_adds(AddRequest::Items(
                        paths.into_iter().map(AddItem::File).collect(),
                    ));
                }
            }
            Ok(None) => tracing::debug!("the file dialog was cancelled"),
            Err(error) => {
                tracing::warn!(%error, "the file dialog could not be opened");
                let text = SharedString::from(t!("add-file-picker-failed"));
                self.add_notice(Notification::error, text, window, cx);
            }
        }
    }

    /// Queues each magnet link on the clipboard, or, without one, the form to paste a link into.
    fn add_magnet_link(&self, cx: &App) {
        let ClipboardItems { items, left_out } = magnet_form_items(clipboard_text(cx).as_deref());
        tracing::info!(
            count = items.len(),
            left_out,
            "magnet links to add were asked for"
        );
        self.request_adds(AddRequest::Capped { items, left_out });
    }

    /// Queues each magnet link on the clipboard, or says that it holds none.
    fn paste_magnet_links(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let ClipboardItems { items, left_out } = clipboard_text(cx)
            .as_deref()
            .map(clipboard_magnets)
            .unwrap_or_default();
        if items.is_empty() {
            tracing::debug!("the clipboard holds no magnet link");
            let text = SharedString::from(t!("add-no-magnet-on-clipboard"));
            self.add_notice(Notification::info, text, window, cx);
            return;
        }
        tracing::info!(
            count = items.len(),
            left_out,
            "magnet links were pasted to add"
        );
        self.request_adds(AddRequest::Capped {
            items: items
                .into_iter()
                .map(|link| AddItem::Magnet(Some(link)))
                .collect(),
            left_out,
        });
    }

    /// Takes the files `paths` dropped on the window or on the add dialog: the `.torrent` files
    /// join the queue, and one notification counts the others. Torrents dropped from another app
    /// bring the window to the front, so that the keys go to the add dialog.
    pub(crate) fn files_dropped(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let DroppedFiles { torrents, others } = dropped_files(paths.iter().cloned());
        tracing::info!(torrents = torrents.len(), others, "files were dropped");
        if others > 0 {
            self.add_notice(
                Notification::warning,
                dropped_others_text(others),
                window,
                cx,
            );
        }
        if !torrents.is_empty() {
            self.request_adds(AddRequest::Items(
                torrents.into_iter().map(AddItem::File).collect(),
            ));
            if !window.is_window_active() {
                self.raise("torrent files were dropped from another app", window, cx);
            }
        }
    }

    /// Sends `request` into the window's own channel of requests, which the window takes on the
    /// UI thread a moment later, as it takes those of the sources outside it.
    ///
    /// The send cannot fail, so the user is told nothing: the view listens to a channel from the
    /// start ([`MainView::listen_for_adds`]) and keeps its sender, and the listener, which holds
    /// the receiver, lives as long as the view. Should that change, the error is logged.
    fn request_adds(&self, request: AddRequest) {
        if let Err(error) = self.adds.requests.try_send(request) {
            tracing::error!(%error, "the window no longer takes requests to add torrents");
        }
    }
}

/// The id of the notification that the file dialog is open already, so that another press
/// replaces it rather than stacking another.
struct FilePickerOpen;

/// The text on the clipboard, if it holds text.
fn clipboard_text(cx: &App) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

/// `root`, the main view's root element, with the handlers of Ctrl+O, Ctrl+U and Ctrl+V, which run
/// once per press of their key, as `repeats`, the view's record of the key held, tells.
pub(in crate::main_view) fn on_add_actions(
    root: Div,
    repeats: &Rc<RefCell<KeyRepeats>>,
    cx: &Context<'_, MainView>,
) -> Div {
    root.on_action(once_per_press_of::<_, AddTorrentFiles>(
        repeats,
        cx,
        MainView::add_torrent_files,
    ))
    .on_action(once_per_press_of::<_, AddMagnetLink>(
        repeats,
        cx,
        |view, _, cx| view.add_magnet_link(cx),
    ))
    .on_action(once_per_press_of::<_, PasteMagnetLinks>(
        repeats,
        cx,
        MainView::paste_magnet_links,
    ))
}
