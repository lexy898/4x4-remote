//! The add dialog: a gpui-component `Dialog` in the main window, whose content is one
//! [`AddDialog`] for the head of the queue.
//!
//! For the torrent it is about, the dialog shows:
//! - its title, with the place in the queue when there are several ("Add torrent (2 of 5)");
//! - the torrent's name and total size; for a magnet link, the field of the link, checked as it
//!   is typed, and the link's display name or its hash;
//! - the folder on the server, with the folders used lately and the free space there (the
//!   `folder_field` module);
//! - for a `.torrent` with a choice, its files with checkboxes (the `file_choice` module), or why
//!   all of them are added;
//! - "Start immediately";
//! - Add, enabled only when x4-core's [`AddForm::check`] is [`AddCheck::Ready`], with the reason
//!   on one line when it is not, and a Show button for a torrent in the list already; and Cancel.
//!
//! The folder and "Start immediately" start as the session's, and the next torrent of the same
//! run keeps what the user chose for the one before. A dialog that opens before the session is
//! known takes them once it is, unless the user has changed them already.
//!
//! Add hands the request of the check to the main view ([`AddDialogEvent::Add`]), which sends it;
//! Add then says "Adding…" and sends nothing more until the main view hears back. The dialog
//! sends nothing itself. Enter in a text field adds, once per press. The dialog's own Enter is
//! bound to nothing in its key context `AddTorrent`, so it cannot close the dialog from a field.
//! Escape and Cancel skip the torrent, through the main view, which moves on to the next one or
//! closes the dialog. A click beside the dialog does nothing, so that it never throws a choice of
//! files away. Files dropped on the dialog join the queue.
//!
//! The dialog lies beside the main view, so the main view does not see its key-ups: the dialog
//! passes them on to the record of the key held. The repeats of a held key press none of its
//! buttons.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    KeyUpEvent, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, WeakEntity,
    Window, div, px,
};
use x4_core::{
    ActionId, AddCheck, AddForm, AddItem, FreeSpaceAnswer, LoadedItem, Snapshot, human_size,
};
use x4_rpc::{AddSource, DownloadDir, HashString, MagnetLink, TorrentAdd};

use super::AddRequest;
use super::file_choice::FileChoiceList;
use super::folder_field::FolderField;
use crate::i18n::{add_reason_text, add_title_text, no_choice_text, size_text, t};
use crate::key_repeats::KeyRepeats;
use crate::keymap::ADD_DIALOG_CONTEXT;
use crate::main_view::MainView;

/// The width of the dialog.
const DIALOG_WIDTH: f32 = 640.;

/// What the dialog asks the main view for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AddDialogEvent {
    /// Send this add of the torrent shown.
    Add(Outgoing),
    /// The torrent is in the list already: select it, and move on.
    Show(HashString),
    /// Read the free space of this folder.
    AskFreeSpace(DownloadDir),
}

/// An add to send, with what the main view keeps of it until the daemon answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outgoing {
    /// The request, as x4-core's check made it.
    pub(crate) add: TorrentAdd,
    /// The torrent's info hash, as it is known before the daemon answers.
    pub(crate) hash: HashString,
    /// The torrent's name, for what the window says of it.
    pub(crate) name: String,
}

/// The torrent the dialog shows: the form and what is shown of it besides.
pub(super) struct Shown {
    /// The item of the queue it is.
    item: AddItem,
    /// The form, which decides what Add sends.
    pub(super) form: AddForm,
    /// The total size of a `.torrent`'s files, when it is known.
    total_size: Option<u64>,
    /// The info hash of a `.torrent`.
    hash: Option<HashString>,
    /// The link of a magnet form as it is typed, while it is valid.
    link: Option<MagnetLink>,
    /// The name of the `.torrent` file, for a torrent without a name of its own.
    file_name: Option<String>,
}

/// The content of the add dialog.
pub(crate) struct AddDialog {
    /// The torrent shown; `None` while its file is read.
    pub(super) shown: Option<Shown>,
    /// Where it is in the queue: its number from 1, and how many the run has.
    position: Option<(usize, usize)>,
    /// The folder field.
    pub(super) folder: FolderField,
    /// The text of the magnet link field.
    link: Entity<InputState>,
    /// The list of files.
    pub(super) files: FileChoiceList,
    /// What the user made of "Start immediately", which the session's default then leaves alone.
    chosen_start: Option<bool>,
    /// The session's `start_added_torrents`, once the session's folder and it have been taken.
    session_start: Option<bool>,
    /// The newest snapshot, which the check and the folders offered follow.
    pub(super) snapshot: Option<Arc<Snapshot>>,
    /// Whether the daemon is connected.
    connected: bool,
    /// Whether Add may send, and what.
    check: AddCheck,
    /// The add of the torrent shown that is out, until the main view hears back.
    waiting: Option<ActionId>,
    /// What gets the focus at the next build of the dialog, after a torrent was shown.
    pending_focus: Option<FocusHandle>,
    /// The focus of the Add button.
    confirm_focus: FocusHandle,
    /// The key held down now, shared with the main view, so that a held key acts once.
    pub(super) key_repeats: Rc<RefCell<KeyRepeats>>,
    /// The fields' events.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AddDialogEvent> for AddDialog {}

impl AddDialog {
    /// An empty dialog, which [`show`](Self::show) fills. `history` is the window's list of the
    /// folders torrents were added to lately; `snapshot` and `connected` are the main view's now.
    pub(crate) fn new(
        key_repeats: Rc<RefCell<KeyRepeats>>,
        history: Vec<String>,
        snapshot: Option<Arc<Snapshot>>,
        connected: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let folder = cx.new(|cx| InputState::new(window, cx));
        let link = cx.new(|cx| {
            InputState::new(window, cx).placeholder(SharedString::from(t!("add-link-placeholder")))
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &folder,
                window,
                |dialog, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => dialog.folder_edited(cx),
                    InputEvent::PressEnter { .. } => dialog.enter_in_a_field(cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ),
            cx.subscribe_in(
                &link,
                window,
                |dialog, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => dialog.link_edited(cx),
                    InputEvent::PressEnter { .. } => dialog.enter_in_a_field(cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ),
        ];
        let mut dialog = Self {
            shown: None,
            position: None,
            folder: FolderField::new(folder, history),
            link,
            files: FileChoiceList::new(cx),
            chosen_start: None,
            session_start: None,
            snapshot,
            connected,
            check: AddCheck::NotConnected,
            waiting: None,
            pending_focus: None,
            confirm_focus: cx.focus_handle(),
            key_repeats,
            _subscriptions: subscriptions,
        };
        dialog.take_defaults(window, cx);
        dialog
    }

    /// Shows `loaded`, the item `item` of the queue, at `position` in it, with the folder and
    /// "Start immediately" as they are. A `.torrent` and a valid link put the focus on Add, an
    /// empty or invalid link on its field.
    pub(crate) fn show(
        &mut self,
        item: AddItem,
        loaded: LoadedItem,
        position: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let folder = self.folder.input.read(cx).value().to_string();
        let (total_size, hash, link, file_name) = match &loaded {
            LoadedItem::Torrent(torrent) => (
                torrent.parsed().total_size,
                Some(torrent.parsed().hash.clone()),
                None,
                torrent
                    .path()
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
            ),
            LoadedItem::Magnet(link) => {
                let text = link.as_ref().map_or("", MagnetLink::as_str).to_owned();
                self.link
                    .update(cx, |input, cx| input.set_value(text, window, cx));
                (None, None, link.clone(), None)
            }
        };
        let form = AddForm::new(loaded, folder, self.start());
        let link_wanted = form.is_magnet() && link.is_none();
        self.files.show(form.tree());
        self.shown = Some(Shown {
            item,
            form,
            total_size,
            hash,
            link,
            file_name,
        });
        self.position = position;
        self.waiting = None;
        self.recheck();
        self.pending_focus = Some(if link_wanted {
            self.link.focus_handle(cx)
        } else {
            self.confirm_focus.clone()
        });
        self.ask_free_space(cx);
        cx.notify();
    }

    /// Shows that the item at `position` is being read; nothing can be added meanwhile.
    pub(crate) fn reading(&mut self, position: Option<(usize, usize)>, cx: &mut Context<'_, Self>) {
        self.shown = None;
        self.position = position;
        self.waiting = None;
        self.files.show(None);
        self.recheck();
        self.pending_focus = Some(self.confirm_focus.clone());
        cx.notify();
    }

    /// Moves the torrent shown to `position` in the queue, as when more items joined it.
    pub(crate) fn set_position(
        &mut self,
        position: Option<(usize, usize)>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.position != position {
            self.position = position;
            cx.notify();
        }
    }

    /// Whether the dialog shows the item `item` of the queue, read.
    pub(crate) fn shows(&self, item: &AddItem) -> bool {
        self.shown.as_ref().is_some_and(|shown| &shown.item == item)
    }

    /// The title of the dialog.
    pub(crate) fn title(&self) -> SharedString {
        add_title_text(self.position)
    }

    /// The add of the torrent shown that is out, if any.
    pub(crate) fn waiting(&self) -> Option<ActionId> {
        self.waiting
    }

    /// Marks the add of the torrent shown as out with the id `id`, or as back (`None`) after it
    /// failed: the form is kept, and can be sent again.
    pub(crate) fn set_waiting(&mut self, id: Option<ActionId>, cx: &mut Context<'_, Self>) {
        self.waiting = id;
        cx.notify();
    }

    /// Offers `history` as the folders used lately, as after an add.
    pub(crate) fn set_history(&mut self, history: Vec<String>, cx: &mut Context<'_, Self>) {
        self.folder.set_history(history);
        cx.notify();
    }

    /// Takes the newest `snapshot`, with the daemon `connected` or not. The first session that is
    /// known gives the folder and "Start immediately" the user has not set yet. When the daemon is
    /// back, the free space that was not known is asked for again.
    pub(crate) fn follow(
        &mut self,
        snapshot: Option<Arc<Snapshot>>,
        connected: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let back = connected && !self.connected;
        self.snapshot = snapshot;
        self.connected = connected;
        self.take_defaults(window, cx);
        if back {
            self.folder.forget_unknown();
            self.ask_free_space(cx);
        }
        self.recheck();
        cx.notify();
    }

    /// Takes the daemon's `answer` about the free space of `path`, if the field shows that folder.
    pub(crate) fn free_space_read(
        &mut self,
        path: DownloadDir,
        answer: FreeSpaceAnswer,
        cx: &mut Context<'_, Self>,
    ) {
        let current = self.current_folder();
        if self.folder.answered(current.as_ref(), path, answer) {
            cx.notify();
        }
    }

    /// The focus to give at the next build of the dialog, once.
    fn take_pending_focus(&mut self, _: &mut Context<'_, Self>) -> Option<FocusHandle> {
        self.pending_focus.take()
    }

    /// Fills in the session's folder, unless the field has a folder already, and its "Start
    /// immediately", unless the user has chosen; once, as soon as the session is known. The free
    /// space the snapshot has for the session's folder is kept too.
    fn take_defaults(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.session_start.is_some() {
            return;
        }
        let Some(snapshot) = self.snapshot.clone() else {
            return;
        };
        let Some(session) = &snapshot.session else {
            return;
        };
        self.session_start = Some(session.start_added_torrents);
        self.folder.keep_session_free_space(&snapshot);
        let start = self.start();
        if let Some(shown) = &mut self.shown {
            shown.form.set_start(start);
        }
        if self.folder.input.read(cx).value().is_empty() {
            let folder = session.download_dir.clone();
            self.folder
                .input
                .update(cx, |input, cx| input.set_value(folder.clone(), window, cx));
            if let Some(shown) = &mut self.shown {
                shown.form.set_folder(folder);
            }
        }
    }

    /// Whether "Start immediately" is checked: as the user chose, or as the session starts added
    /// torrents; before the session is known, as the daemon does by default.
    fn start(&self) -> bool {
        self.chosen_start.or(self.session_start).unwrap_or(true)
    }

    /// Checks the form again.
    pub(super) fn recheck(&mut self) {
        self.check = match (&self.shown, &self.snapshot) {
            (Some(shown), Some(snapshot)) => shown.form.check(snapshot, self.connected),
            _ => AddCheck::NotConnected,
        };
    }

    /// Follows an edit of the link field: the form checks the new text.
    fn link_edited(&mut self, cx: &mut Context<'_, Self>) {
        let text = self.link.read(cx).value().to_string();
        if let Some(shown) = &mut self.shown {
            shown.form.set_link(&text);
            shown.link = MagnetLink::parse(&text).ok();
        }
        self.recheck();
        cx.notify();
    }

    /// Follows a click on "Start immediately".
    fn start_clicked(&mut self, start: bool, cx: &mut Context<'_, Self>) {
        self.chosen_start = Some(start);
        if let Some(shown) = &mut self.shown {
            shown.form.set_start(start);
        }
        self.recheck();
        cx.notify();
    }

    /// Enter in a text field: adds, for a new press of the key.
    fn enter_in_a_field(&mut self, cx: &mut Context<'_, Self>) {
        if self.key_repeats.borrow().repeating() {
            tracing::trace!("a held Enter repeats; nothing added again");
            return;
        }
        self.add(cx);
    }

    /// Hands the request of the check to the main view, unless one is out already or the check
    /// does not allow it.
    fn add(&mut self, cx: &mut Context<'_, Self>) {
        if self.waiting.is_some() {
            tracing::debug!("an add of this torrent is out already; nothing sent");
            return;
        }
        let AddCheck::Ready(add) = &self.check else {
            tracing::debug!("the form cannot be sent now; nothing sent");
            return;
        };
        let Some(hash) = self.hash() else {
            tracing::error!("a form ready to send has no info hash; nothing sent");
            return;
        };
        let name = self.name().unwrap_or_else(|| hash.as_str().to_owned());
        cx.emit(AddDialogEvent::Add(Outgoing {
            add: add.clone(),
            hash,
            name,
        }));
    }

    /// The info hash of the torrent shown: the `.torrent`'s, or the link's as it is typed.
    fn hash(&self) -> Option<HashString> {
        let shown = self.shown.as_ref()?;
        shown
            .hash
            .clone()
            .or_else(|| shown.link.as_ref().map(|link| link.hash().clone()))
    }

    /// The torrent's name: its own, the `.torrent` file's, or a magnet link's hash.
    fn name(&self) -> Option<String> {
        let shown = self.shown.as_ref()?;
        shown
            .form
            .name()
            .map(str::to_owned)
            .or_else(|| shown.file_name.clone())
            .or_else(|| {
                shown
                    .link
                    .as_ref()
                    .map(|link| link.hash().as_str().to_owned())
            })
    }

    /// The line of the torrent's name and size, or that its file is being read.
    fn heading(&self, cx: &App) -> Div {
        let muted = cx.theme().muted_foreground;
        let Some(shown) = &self.shown else {
            let reading = SharedString::from(t!("add-reading"));
            return h_flex().child(
                text_line("add-reading", &reading)
                    .text_color(muted)
                    .test_support(),
            );
        };
        let name = self.name().map(SharedString::from);
        let size = shown.total_size.map(|size| size_text(human_size(size)));
        h_flex()
            .gap_3()
            .items_baseline()
            .children(name.map(|name| {
                text_line("add-name", &name)
                    .flex_1()
                    .min_w_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .test_support()
            }))
            .children(size.map(|size| {
                text_line("add-size", &size)
                    .flex_none()
                    .text_color(muted)
                    .test_support()
            }))
    }

    /// The field of a magnet link and the note that its files come later.
    fn link_part(&self, cx: &App) -> Div {
        let note = SharedString::from(t!("add-magnet-note"));
        v_flex()
            .gap_1()
            .child(
                Input::new(&self.link)
                    .id("add-link")
                    .aria_label(SharedString::from(t!("add-link-field"))),
            )
            .child(
                text_line("add-magnet-note", &note)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .test_support(),
            )
    }

    /// The files of the `.torrent` shown to choose from, or why all of them are added.
    fn files_part(&self, window: &Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let shown = self.shown.as_ref()?;
        if shown.form.is_magnet() {
            return None;
        }
        Some(match shown.form.choice_unavailable() {
            Some(reason) => text_line("add-no-choice", &no_choice_text(reason))
                .text_color(cx.theme().muted_foreground)
                .test_support()
                .into_any_element(),
            None => self.file_choice(window, cx).into_any_element(),
        })
    }

    /// The line that says why Add is disabled, with the Show button of a torrent in the list.
    fn reason_line(&self, cx: &mut Context<'_, Self>) -> Option<Div> {
        if self.waiting.is_some() || self.shown.is_none() {
            return None;
        }
        let text = add_reason_text(&self.check)?;
        let show = match &self.check {
            AddCheck::AlreadyInList(hash) => Some(hash.clone()),
            _ => None,
        };
        Some(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    text_line("add-reason", &text)
                        .flex_1()
                        .text_color(cx.theme().warning)
                        .test_support(),
                )
                .children(show.map(|hash| {
                    Button::new("add-show")
                        .small()
                        .label(t!("add-show"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(AddDialogEvent::Show(hash.clone()));
                        }))
                })),
        )
    }
}

/// A line of the dialog's text, `id`, which screen readers read as its name.
fn text_line(id: &'static str, text: &SharedString) -> Stateful<Div> {
    div().id(id).aria_label(text.clone()).child(text.clone())
}

impl Render for AddDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let repeats = Rc::clone(&self.key_repeats);
        let content = v_flex()
            .id("add-dialog")
            .key_context(ADD_DIALOG_CONTEXT)
            .gap_3()
            .text_sm()
            // The dialog lies beside the main view, which would not see the key go up.
            .capture_key_up(move |_: &KeyUpEvent, _, _| repeats.borrow_mut().release())
            .child(self.heading(cx));
        let Some(shown) = &self.shown else {
            return content;
        };
        let link = shown.form.is_magnet().then(|| self.link_part(cx));
        let files = self.files_part(window, cx);
        let folder_label = SharedString::from(t!("add-folder-field"));
        content
            .children(link)
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(folder_label),
                    )
                    .child(self.folder_row(cx))
                    .children(self.free_space_line(cx)),
            )
            .children(files)
            .child(
                Checkbox::new("add-start")
                    .label(t!("add-start"))
                    .checked(self.start())
                    .on_click(cx.listener(|dialog, start: &bool, _, cx| {
                        dialog.start_clicked(*start, cx);
                    })),
            )
            .children(self.reason_line(cx))
    }
}

/// Cancel and Add, in the dialog's key context, where Enter presses the focused button. Cancel
/// skips the torrent, through `view`; Add is enabled while the check is ready and no add is out.
fn footer(dialog: &Entity<AddDialog>, view: &WeakEntity<MainView>, cx: &App) -> Div {
    let state = dialog.read(cx);
    let ready = matches!(state.check, AddCheck::Ready(_)) && state.waiting.is_none();
    let label = if state.waiting.is_some() {
        t!("add-adding")
    } else {
        t!("add-confirm")
    };
    let released = Rc::clone(&state.key_repeats);
    let held = Rc::clone(&state.key_repeats);
    let cancel_view = view.clone();
    let add_dialog = dialog.downgrade();
    let cancel =
        Button::new("add-cancel")
            .label(t!("add-cancel"))
            .on_click(move |_, window, cx| {
                if let Err(error) = cancel_view.update(cx, |view, cx| view.cancel_add(window, cx)) {
                    tracing::debug!(%error, "the add dialog was cancelled after the window closed");
                    window.close_dialog(cx);
                }
            });
    let confirm = Button::new("add-confirm")
        .primary()
        .label(label)
        .track_focus(&state.confirm_focus)
        .disabled(!ready)
        .on_click(move |_, _, cx| {
            if let Err(error) = add_dialog.update(cx, AddDialog::add) {
                tracing::debug!(%error, "Add was pressed after the dialog closed");
            }
        });
    div()
        .key_context(ADD_DIALOG_CONTEXT)
        .capture_key_up(move |_: &KeyUpEvent, _, _| released.borrow_mut().release())
        // Before the focused button takes the key-down that its key-up would turn into a press.
        .capture_key_down(move |event: &KeyDownEvent, window, _| {
            if event.is_held || held.borrow().repeating() {
                window.prevent_default();
            }
        })
        .child(DialogFooter::new().child(cancel).child(confirm))
}

/// Queues the files `paths` dropped on the dialog, through `view`, as a drop on the window does.
fn dropped(paths: &ExternalPaths, view: &WeakEntity<MainView>, window: &mut Window, cx: &mut App) {
    let items = paths
        .paths()
        .iter()
        .map(|path| AddItem::File(path.clone()))
        .collect();
    if let Err(error) = view.update(cx, |view, cx| {
        view.take_add_request(AddRequest::Items(items), window, cx);
    }) {
        tracing::debug!(%error, "files were dropped on the add dialog after the window closed");
    }
}

/// Opens the add dialog of `dialog` in `window`, for the main view `view`, which moves the queue
/// on when the user skips a torrent. The builder runs at every frame, so it only lays the entity
/// out, and gives the focus where [`AddDialog::show`] asked, once.
pub(crate) fn open(
    dialog: Entity<AddDialog>,
    view: WeakEntity<MainView>,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |surface, window, cx| {
        if let Some(focus) = dialog.update(cx, AddDialog::take_pending_focus) {
            focus.focus(window, cx);
        }
        let title = dialog.read(cx).title();
        let cancel_view = view.clone();
        let drop_view = view.clone();
        surface
            .w(px(DIALOG_WIDTH))
            .close_button(false)
            .overlay_closable(false)
            .title(text_line("add-title", &title).test_support())
            .on_cancel(move |_, window, cx| {
                // The view closes the dialog itself once nothing is left to add.
                cancel_view
                    .update(cx, |view, cx| view.cancel_add(window, cx))
                    .is_err()
            })
            .child(
                div()
                    .on_drop(move |paths: &ExternalPaths, window, cx| {
                        dropped(paths, &drop_view, window, cx);
                    })
                    .child(dialog.clone()),
            )
            .footer(footer(&dialog, &view, cx))
    });
}

/// Whether `add` names a magnet link, for the log, which never shows the link itself.
pub(crate) fn is_magnet(add: &TorrentAdd) -> bool {
    matches!(add.source, AddSource::Magnet(_))
}
