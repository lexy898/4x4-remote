//! The folder field of the add dialog: a text field for the folder on the server, a button beside
//! it that opens the folders used lately, and the line under it with the free space there.
//!
//! The folders offered are x4-core's [`folder_picks`]: the session's folder, the history, then the
//! folders of the torrents in the list. They open in a menu under the button, from a click or from
//! Alt+Down in the field, inside the key context `FolderField`; the menu keeps its own Up, Down,
//! Enter and Escape. Picking one sets the field and gives the keys back to it; Escape closes only
//! the menu.
//!
//! The free space is read from the daemon only while the dialog is open, and sparingly: half a
//! second after the last edit ([`FREE_SPACE_DELAY`]), only for a folder the daemon could use, and
//! only when the dialog knows nothing about it yet. Every answer is kept for its folder while the
//! dialog is open, so going back to a folder asks nothing again; an answer for a folder the field
//! has left is dropped. An unknown answer is forgotten when the daemon comes back, and asked for
//! again. The session's own folder starts with the free space the snapshot has for it.

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, Context, DismissEvent, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Task, TestSupportExt as _, Window, div,
};
use x4_core::{FreeSpaceAnswer, Snapshot, folder_picks};
use x4_rpc::DownloadDir;

use super::dialog::{AddDialog, AddDialogEvent};
use crate::i18n::{free_space_line_text, t};
use crate::keymap::{FOLDER_FIELD_CONTEXT, OpenFolderHistory};

/// How long after the last edit of the folder its free space is asked for.
pub(super) const FREE_SPACE_DELAY: Duration = Duration::from_millis(500);

/// The folder field, its history and what is known of the free space of its folders.
pub(super) struct FolderField {
    /// The text of the field.
    pub(super) input: Entity<InputState>,
    /// The folders torrents were added to lately, the newest first, as the window remembers them.
    history: Vec<String>,
    /// The menu of the folders offered while it is open.
    menu: Option<Entity<PopupMenu>>,
    /// The menu's going away, which closes it here too; dropping it stops listening.
    menu_dismissed: Option<Subscription>,
    /// What the daemon said about each folder asked about while the dialog is open.
    answers: HashMap<DownloadDir, FreeSpaceAnswer>,
    /// The wait after the last edit, before the free space is asked for; dropping it cancels it.
    wait: Option<Task<()>>,
}

impl FolderField {
    /// A field with `input` as its text, offering `history` after the session's folder.
    pub(super) fn new(input: Entity<InputState>, history: Vec<String>) -> Self {
        Self {
            input,
            history,
            menu: None,
            menu_dismissed: None,
            answers: HashMap::new(),
            wait: None,
        }
    }

    /// Keeps the free space that `snapshot` has for the session's folder, so that it is not asked
    /// for again.
    pub(super) fn keep_session_free_space(&mut self, snapshot: &Snapshot) {
        if let (Some(session), Some(bytes)) = (&snapshot.session, snapshot.free_space_bytes)
            && let Ok(folder) = DownloadDir::new(&session.download_dir)
        {
            self.answers.insert(folder, FreeSpaceAnswer::Free { bytes });
        }
    }

    /// Offers `history` as the folders used lately.
    pub(super) fn set_history(&mut self, history: Vec<String>) {
        self.history = history;
    }

    /// What is known of the free space of `folder`.
    pub(super) fn answer(&self, folder: &DownloadDir) -> Option<&FreeSpaceAnswer> {
        self.answers.get(folder)
    }

    /// Keeps `answer` about `folder`, if the field shows that folder now (`current`).
    pub(super) fn answered(
        &mut self,
        current: Option<&DownloadDir>,
        folder: DownloadDir,
        answer: FreeSpaceAnswer,
    ) -> bool {
        if current != Some(&folder) {
            tracing::debug!("the free space of a folder the field has left; dropped");
            return false;
        }
        self.answers.insert(folder, answer);
        true
    }

    /// Forgets the answers that said nothing, as the daemon is back.
    pub(super) fn forget_unknown(&mut self) {
        self.answers
            .retain(|_, answer| *answer != FreeSpaceAnswer::Unknown);
    }
}

impl AddDialog {
    /// Follows an edit of the folder field: the form takes the text, and the free space is asked
    /// for once the user stops typing.
    pub(super) fn folder_edited(&mut self, cx: &mut Context<'_, Self>) {
        let text = self.folder.input.read(cx).value().to_string();
        if let Some(shown) = &mut self.shown {
            shown.form.set_folder(text);
        }
        self.recheck();
        self.folder.wait = Some(cx.spawn(async move |dialog, cx| {
            cx.background_executor().timer(FREE_SPACE_DELAY).await;
            if let Err(error) = dialog.update(cx, Self::ask_free_space) {
                tracing::debug!(%error, "the add dialog closed before its free space was asked");
            }
        }));
        cx.notify();
    }

    /// Asks the main view to read the free space of the folder of the form, unless it is not one
    /// the daemon could use or the dialog knows about it already.
    pub(super) fn ask_free_space(&mut self, cx: &mut Context<'_, Self>) {
        self.folder.wait = None;
        let Some(folder) = self.current_folder() else {
            return;
        };
        if self.folder.answer(&folder).is_none() {
            cx.emit(AddDialogEvent::AskFreeSpace(folder));
        }
    }

    /// The folder of the form, if the daemon could use it.
    pub(super) fn current_folder(&self) -> Option<DownloadDir> {
        self.shown
            .as_ref()
            .and_then(|shown| shown.form.folder().ok())
    }

    /// Opens the menu of the folders offered, from the button or Alt+Down, with the folder of the
    /// field checked; the menu takes the keys.
    pub(super) fn open_history(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let picks = self.folder_picks();
        if picks.is_empty() || self.folder.menu.is_some() {
            return;
        }
        let current = self.current_folder();
        let dialog = cx.weak_entity();
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            picks.into_iter().fold(menu, |menu, folder| {
                let checked = current
                    .as_ref()
                    .is_some_and(|current| current.as_str() == folder);
                let dialog = dialog.clone();
                let label = SharedString::from(folder);
                let picked = label.to_string();
                menu.item(PopupMenuItem::new(label).checked(checked).on_click(
                    move |_, window, cx| {
                        if let Err(error) = dialog.update(cx, |dialog, cx| {
                            dialog.pick_folder(&picked, window, cx);
                        }) {
                            tracing::debug!(%error, "a folder was picked after the dialog closed");
                        }
                    },
                ))
            })
        });
        let dismissed =
            cx.subscribe_in(&menu, window, |dialog, _, _: &DismissEvent, window, cx| {
                dialog.close_history(window, cx);
            });
        menu.focus_handle(cx).focus(window, cx);
        self.folder.menu = Some(menu);
        self.folder.menu_dismissed = Some(dismissed);
        cx.notify();
    }

    /// Closes the menu of the folders, and gives the keys back to the field.
    pub(super) fn close_history(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.folder.menu.take().is_none() {
            return;
        }
        self.folder.menu_dismissed = None;
        self.folder.input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Puts the picked `folder` into the field, as if it was typed.
    fn pick_folder(&mut self, folder: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.folder.input.update(cx, |input, cx| {
            input.set_value(folder.to_owned(), window, cx);
        });
        self.close_history(window, cx);
        self.folder_edited(cx);
    }

    /// The folders the field offers now.
    fn folder_picks(&self) -> Vec<String> {
        let snapshot = self.snapshot.as_deref();
        folder_picks(
            snapshot
                .and_then(|snapshot| snapshot.session.as_ref())
                .map(|session| session.download_dir.as_str()),
            &self.folder.history,
            snapshot.map_or(&[], |snapshot| snapshot.torrents.as_slice()),
        )
    }

    /// The field with the button of its history, in the key context `FolderField`.
    pub(super) fn folder_row(&self, cx: &mut Context<'_, Self>) -> impl IntoElement + use<> {
        let open = self.folder.menu.clone();
        // The popover moves the focus into what it tracks once it opens: the menu, which then
        // takes the keys.
        let menu_focus = open.as_ref().map(|menu| menu.focus_handle(cx));
        let has_picks = !self.folder_picks().is_empty();
        let dialog = cx.weak_entity();
        let button = Button::new("add-folder-history")
            .ghost()
            .small()
            .icon(IconName::ChevronDown)
            .tab_stop(false)
            .disabled(!has_picks)
            .tooltip(SharedString::from(t!("add-folder-history")));
        h_flex()
            .key_context(FOLDER_FIELD_CONTEXT)
            .gap_1()
            .on_action(cx.listener(|dialog, _: &OpenFolderHistory, window, cx| {
                dialog.open_history(window, cx);
            }))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.folder.input)
                        .id("add-folder")
                        .aria_label(SharedString::from(t!("add-folder-field"))),
                ),
            )
            .child(
                Popover::new("add-folder-history-popover")
                    .appearance(false)
                    .anchor(Anchor::TopRight)
                    .trigger(button)
                    .open(open.is_some())
                    .when_some(menu_focus, |popover, focus| popover.track_focus(&focus))
                    .on_open_change(move |opened, window, cx| {
                        let opened = *opened;
                        if let Err(error) = dialog.update(cx, |dialog, cx| {
                            if opened {
                                dialog.open_history(window, cx);
                            } else {
                                dialog.close_history(window, cx);
                            }
                        }) {
                            tracing::debug!(%error, "the folder history changed after the dialog closed");
                        }
                    })
                    .content(move |_, _, _| match &open {
                        Some(menu) => menu.clone().into_any_element(),
                        None => div().into_any_element(),
                    }),
            )
    }

    /// The line under the field: the free space of its folder, once known.
    pub(super) fn free_space_line(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<impl IntoElement + use<>> {
        let folder = self.current_folder()?;
        let text = free_space_line_text(self.folder.answer(&folder)?);
        Some(
            div()
                .id("add-free-space")
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .aria_label(text.clone())
                .child(text)
                .test_support(),
        )
    }
}
