//! The add dialog: a gpui-component `Dialog` in the main window, whose whole surface is one
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
//! While the file of a torrent is read, the dialog says so, and Cancel becomes Skip, so that a file
//! that takes long to read never holds the queue.
//!
//! x4-core decides the rest: [`FormDefaults`] gives the folder and "Start immediately" (the
//! session's, then what the user chose, carried on to the next torrent of the run), and
//! [`FreeSpaceCache`] when the free space is asked for and what is kept of the answers.
//!
//! Add checks the form again and hands its request to the main view ([`AddDialogEvent::Add`]),
//! which sends it; Add then says "Adding…" and sends nothing more until the main view hears back.
//! The dialog sends nothing itself. Enter in a text field adds, once per press. The dialog's own
//! Enter is bound to nothing in its key context `AddTorrent`, and its confirm closes nothing, so
//! Enter never closes the dialog from a field. Escape, Cancel and Skip skip the torrent, through
//! the main view, which moves on to the next one or closes the dialog. A click beside the dialog
//! does nothing, so that it never throws a choice of files away. Files dropped anywhere on the
//! dialog join the queue.
//!
//! The dialog lies beside the main view, so the main view does not see its key-ups: the dialog
//! passes them on to the record of the key held. The repeats of a held key press none of its
//! buttons.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogFooter, DialogTitle};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    KeyUpEvent, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
    px,
};
use x4_core::{
    ActionId, AddCheck, AddForm, AddItem, FormDefaults, FreeSpaceAnswer, FreeSpaceCache,
    LoadedItem, SentAdd, Snapshot, folder_picks, human_size,
};
use x4_rpc::{DownloadDir, HashString, MagnetLink, TorrentAdd};

use super::file_choice::FileChoiceList;
use super::folder_field::FolderField;
use crate::i18n::{add_reason_text, add_title_text, no_choice_text, size_text, t};
use crate::key_repeats::KeyRepeats;
use crate::keymap::ADD_DIALOG_CONTEXT;

/// The width of the dialog.
const DIALOG_WIDTH: f32 = 640.;

/// What the dialog asks the main view for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AddDialogEvent {
    /// Send `add`, the request of the torrent shown, which the window keeps as `sent` until the
    /// daemon answers.
    Add {
        /// The request, boxed: it is much larger than the other events.
        add: Box<TorrentAdd>,
        /// What the window keeps of it.
        sent: SentAdd,
    },
    /// The torrent is in the list already: select it, and move on.
    Show(HashString),
    /// Skip the torrent shown, or the file being read.
    Skip,
    /// Queue these, dropped on the dialog.
    Dropped(Vec<AddItem>),
    /// Read the free space of this folder.
    AskFreeSpace(DownloadDir),
}

/// The torrent the dialog shows: the item of the queue and its form.
pub(super) struct Shown {
    /// The item of the queue it is.
    item: AddItem,
    /// The form, which decides what Add sends and what the torrent is called.
    pub(super) form: AddForm,
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
    /// The folder and "Start immediately" of the run.
    defaults: FormDefaults,
    /// What is known of the free space of the folders the field has shown.
    pub(super) space: FreeSpaceCache,
    /// The newest snapshot, which the check and the folders offered follow.
    snapshot: Option<Arc<Snapshot>>,
    /// Whether the daemon is connected.
    connected: bool,
    /// Whether Add may send, as of the last change.
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
            defaults: FormDefaults::default(),
            space: FreeSpaceCache::new(connected),
            snapshot: None,
            connected,
            check: AddCheck::NotConnected,
            waiting: None,
            pending_focus: None,
            confirm_focus: cx.focus_handle(),
            key_repeats,
            _subscriptions: subscriptions,
        };
        dialog.follow(snapshot, connected, window, cx);
        dialog
    }

    /// Shows `loaded`, the item `item` of the queue, at `position` in it, with the folder and
    /// "Start immediately" of the run. A `.torrent` and a valid link put the focus on Add, an
    /// empty or invalid link on its field.
    pub(crate) fn show(
        &mut self,
        item: AddItem,
        loaded: LoadedItem,
        position: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let LoadedItem::Magnet(link) = &loaded {
            let text = link.as_ref().map_or("", MagnetLink::as_str).to_owned();
            self.link
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
        let form = AddForm::new(
            loaded,
            self.defaults.folder().to_owned(),
            self.defaults.start(),
        );
        let link_wanted = form.is_magnet() && form.hash().is_none();
        self.files.show(form.tree());
        self.shown = Some(Shown { item, form });
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

    /// Shows that the item at `position` is being read; nothing can be added meanwhile, and
    /// Skip moves on.
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
        self.folder.history = history;
        if self.refresh_picks() {
            cx.notify();
        }
    }

    /// Takes the newest `snapshot`, with the daemon `connected` or not. The first session that is
    /// known gives the folder and "Start immediately" the user has not set yet; when the daemon is
    /// back, the free space that was not known is asked for again at once. Draws again only if
    /// what the dialog shows changed.
    pub(crate) fn follow(
        &mut self,
        snapshot: Option<Arc<Snapshot>>,
        connected: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let mut changed = self.connected != connected;
        self.connected = connected;
        self.snapshot = snapshot;
        if let Some(snapshot) = self.snapshot.clone() {
            if let Some(taken) = self.defaults.take_session(&snapshot) {
                if let Some(folder) = taken.folder {
                    self.folder
                        .input
                        .update(cx, |input, cx| input.set_value(folder.clone(), window, cx));
                    if let Some(shown) = &mut self.shown {
                        shown.form.set_folder(folder);
                    }
                }
                if let Some(shown) = &mut self.shown {
                    shown.form.set_start(taken.start);
                }
                changed = true;
            }
            let space = self.space.follow(&snapshot, connected);
            // Only answers forgotten are asked for at once; the delay after an edit holds.
            if space.ask_again {
                self.ask_free_space(cx);
            }
            changed |= space.changed;
        }
        changed |= self.refresh_picks();
        changed |= self.recheck();
        if changed {
            cx.notify();
        }
    }

    /// Takes the daemon's `answer` about the free space of `path`, if the field shows that folder.
    pub(crate) fn free_space_read(
        &mut self,
        path: DownloadDir,
        answer: FreeSpaceAnswer,
        cx: &mut Context<'_, Self>,
    ) {
        let current = self.current_folder();
        if self.space.answered(current.as_ref(), path, answer) {
            cx.notify();
        }
    }

    /// Asks the main view to skip the torrent shown, as Escape does.
    pub(crate) fn skip(cx: &mut Context<'_, Self>) {
        cx.emit(AddDialogEvent::Skip);
    }

    /// The focus to give at the next build of the dialog, once.
    fn take_pending_focus(&mut self, _: &mut Context<'_, Self>) -> Option<FocusHandle> {
        self.pending_focus.take()
    }

    /// Makes the folders the field offers again, from the session, the history and the torrents
    /// of the newest snapshot; returns whether they changed.
    fn refresh_picks(&mut self) -> bool {
        let snapshot = self.snapshot.as_deref();
        let picks = folder_picks(
            snapshot
                .and_then(|snapshot| snapshot.session.as_ref())
                .map(|session| session.download_dir.as_str()),
            &self.folder.history,
            snapshot.map_or(&[], |snapshot| snapshot.torrents.as_slice()),
        );
        if picks == self.folder.picks {
            return false;
        }
        self.folder.picks = picks;
        true
    }

    /// Checks the form again; returns whether the check changed.
    pub(super) fn recheck(&mut self) -> bool {
        let check = match (&self.shown, &self.snapshot) {
            (Some(shown), Some(snapshot)) => shown.form.check(snapshot, self.connected),
            _ => AddCheck::NotConnected,
        };
        if check == self.check {
            return false;
        }
        self.check = check;
        true
    }

    /// Takes the folder `text` of the field, for the form and the next ones of the run.
    pub(super) fn take_folder(&mut self, text: String) {
        self.defaults.set_folder(text.clone());
        if let Some(shown) = &mut self.shown {
            shown.form.set_folder(text);
        }
    }

    /// Follows an edit of the link field: the form checks the new text.
    fn link_edited(&mut self, cx: &mut Context<'_, Self>) {
        let text = self.link.read(cx).value().to_string();
        if let Some(shown) = &mut self.shown {
            shown.form.set_link(&text);
        }
        self.recheck();
        cx.notify();
    }

    /// Follows a click on "Start immediately", for the form and the next ones of the run.
    fn start_clicked(&mut self, start: bool, cx: &mut Context<'_, Self>) {
        self.defaults.choose_start(start);
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

    /// Checks the form afresh and hands its request to the main view, unless one is out already
    /// or the check does not allow it.
    fn add(&mut self, cx: &mut Context<'_, Self>) {
        if self.waiting.is_some() {
            tracing::debug!("an add of this torrent is out already; nothing sent");
            return;
        }
        if self.recheck() {
            cx.notify();
        }
        let AddCheck::Ready(add) = &self.check else {
            tracing::debug!("the form cannot be sent now; nothing sent");
            return;
        };
        let Some(form) = self.shown.as_ref().map(|shown| &shown.form) else {
            return;
        };
        let Some(hash) = form.hash().cloned() else {
            tracing::error!("a form ready to send has no info hash; nothing sent");
            return;
        };
        let name = form
            .display_name()
            .map_or_else(|| hash.as_str().to_owned(), Cow::into_owned);
        let sent = SentAdd {
            hash,
            name,
            download_dir: add.download_dir.clone(),
        };
        cx.emit(AddDialogEvent::Add {
            add: Box::new(add.clone()),
            sent,
        });
    }

    /// The title line, with the place in the queue.
    fn title(&self) -> impl IntoElement + use<> {
        let title = add_title_text(self.position);
        DialogTitle::new().child(text_line("add-title", &title).test_support())
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
        let name = shown
            .form
            .display_name()
            .map(|name| SharedString::from(name.into_owned()));
        let size = shown
            .form
            .total_size()
            .map(|size| size_text(human_size(size)));
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

    /// The form of the torrent shown: the link, the folder, the files, "Start immediately" and why
    /// Add is disabled.
    fn form_part(&self, window: &Window, cx: &mut Context<'_, Self>) -> Option<Div> {
        let shown = self.shown.as_ref()?;
        let link = shown.form.is_magnet().then(|| self.link_part(cx));
        let files = self.files_part(window, cx);
        let folder_label = SharedString::from(t!("add-folder-field"));
        Some(
            v_flex()
                .gap_3()
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
                        .checked(self.defaults.start())
                        .on_click(cx.listener(|dialog, start: &bool, _, cx| {
                            dialog.start_clicked(*start, cx);
                        })),
                )
                .children(self.reason_line(cx)),
        )
    }

    /// Cancel, or Skip while a file is read, and Add, which is enabled while the check is ready
    /// and no add is out. The repeats of a held key press neither.
    fn buttons(&self, cx: &mut Context<'_, Self>) -> Div {
        let ready = matches!(self.check, AddCheck::Ready(_)) && self.waiting.is_none();
        let confirm = if self.waiting.is_some() {
            t!("add-adding")
        } else {
            t!("add-confirm")
        };
        let cancel = if self.shown.is_some() {
            t!("add-cancel")
        } else {
            t!("add-skip")
        };
        let held = Rc::clone(&self.key_repeats);
        div()
            // Before the focused button takes the key-down that its key-up would turn into a press.
            .capture_key_down(move |event: &KeyDownEvent, window, _| {
                if event.is_held || held.borrow().repeating() {
                    window.prevent_default();
                }
            })
            .child(
                DialogFooter::new()
                    .child(
                        Button::new("add-cancel")
                            .label(cancel)
                            .on_click(cx.listener(|_, _, _, cx| Self::skip(cx))),
                    )
                    .child(
                        Button::new("add-confirm")
                            .primary()
                            .label(confirm)
                            .track_focus(&self.confirm_focus)
                            .disabled(!ready)
                            .on_click(cx.listener(|dialog, _, _, cx| dialog.add(cx))),
                    ),
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
        let form = self.form_part(window, cx);
        v_flex()
            .id("add-dialog")
            .key_context(ADD_DIALOG_CONTEXT)
            .w_full()
            .p_4()
            .gap_3()
            .text_sm()
            // The dialog lies beside the main view, which would not see the key go up.
            .capture_key_up(move |_: &KeyUpEvent, _, _| repeats.borrow_mut().release())
            // The content is the whole surface of the dialog, so a drop anywhere on it lands here.
            .on_drop(cx.listener(|_, paths: &ExternalPaths, _, cx| {
                let items = paths
                    .paths()
                    .iter()
                    .map(|path| AddItem::File(path.clone()))
                    .collect();
                cx.emit(AddDialogEvent::Dropped(items));
            }))
            .child(self.title())
            .child(self.heading(cx))
            .children(form)
            .child(self.buttons(cx))
    }
}

/// Opens the add dialog of `dialog` in `window`. Its content fills the whole surface, without the
/// surface's own title, padding or footer, so that a drop anywhere on it reaches the content. The
/// builder runs at every frame, so it only lays the entity out, and gives the focus where
/// [`AddDialog::show`] asked, once.
pub(crate) fn open(dialog: Entity<AddDialog>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |surface, window, cx| {
        if let Some(focus) = dialog.update(cx, AddDialog::take_pending_focus) {
            focus.focus(window, cx);
        }
        let escape = dialog.downgrade();
        surface
            .w(px(DIALOG_WIDTH))
            .p_0()
            .close_button(false)
            .overlay_closable(false)
            // The content binds Enter to nothing; should the dialog's confirm come anyway, it
            // closes nothing.
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, _, cx| {
                // The view closes the dialog itself once nothing is left to add.
                escape.update(cx, |_, cx| AddDialog::skip(cx)).is_err()
            })
            .child(dialog.clone())
    });
}
