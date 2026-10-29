//! The two dialogs that ask before the selected torrents are removed: "Remove from list", which
//! keeps their downloaded files on the server, and "Remove and delete files", which deletes them
//! for good.
//!
//! A dialog takes what it removes when it opens: the hashes of the targets then, and what
//! `x4-core`'s [`RemovalSummary`] says about them (how many, the first names, how much data is
//! downloaded). Neither changes while it is open, whatever the list does meanwhile; the dialog's
//! confirm button sends exactly those hashes, as [`MainView::confirm_removal`]. The button is
//! disabled while the daemon is not connected, and enabled again when it is back; Cancel always
//! works.
//!
//! The dialog that deletes files is built so that no key but a deliberate one deletes anything:
//!
//! - it is a plain gpui-component `Dialog` with no `on_ok`: the dialog's own Enter (its `Confirm`)
//!   only closes it, so Enter can never reach a deletion that way;
//! - Cancel has the focus when it opens, and keeps it until the user moves it;
//! - in the footer, Enter is bound to nothing (`REMOVAL_FOOTER_CONTEXT` in
//!   [`keymap`](crate::keymap)), so it presses the focused button, as Space does: Cancel, unless
//!   the user has moved to "Delete files";
//! - Escape, Cancel and a click beside the dialog close it and send nothing.
//!
//! The light dialog is built the same way, with Remove focused, so that Enter removes from the
//! list. Closing either dialog gives the focus back to what had it when it opened, which the main
//! view makes the torrent list.
//!
//! The dialog lies beside the main view, not inside it, so the main view does not see the keys
//! pressed in it. Its footer passes the key-ups on, so that the key that opened the dialog is not
//! taken as held after it went up. It also makes the repeats of a held key press no button: an
//! Enter held on a menu item opens the dialog, and its repeats must not confirm it too.
//!
//! The names are the user's one check of what goes, so each is shown in full, wrapped over as
//! many lines as it takes: names that differ only at their end must stay apart. Every line of
//! text is a label for screen readers, as gpui's accessible text is.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, v_flex};
use gpui_kit::{
    App, ElementId, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    WeakEntity, Window, div,
};
use x4_core::RemovalSummary;
use x4_rpc::{HashString, RemoveData};

use crate::i18n::{removal_files_text, removal_more_text, removal_title_text, t};
use crate::keymap::REMOVAL_FOOTER_CONTEXT;
use crate::main_view::MainView;

/// What a removal dialog removes and says, taken when it opens.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Removal {
    /// Whether the downloaded files stay on the server.
    data: RemoveData,
    /// The torrents to remove, in the order the list showed them.
    hashes: Vec<HashString>,
    /// "Remove 3 torrents from the list?", "Delete 3 torrents and their files?".
    title: SharedString,
    /// The names of the first torrents, each on its own line.
    names: Vec<SharedString>,
    /// "and 2 more", when the names leave some out.
    more: Option<SharedString>,
    /// What becomes of the downloaded files.
    files: SharedString,
}

impl Removal {
    /// The removal of the torrents `hashes`, keeping or deleting their files as `data` says, which
    /// `summary` describes.
    pub(crate) fn new(data: RemoveData, hashes: Vec<HashString>, summary: &RemovalSummary) -> Self {
        Self {
            data,
            hashes,
            title: removal_title_text(data, summary),
            names: summary
                .names
                .iter()
                .map(|name| SharedString::from(name.clone()))
                .collect(),
            more: removal_more_text(summary),
            files: removal_files_text(data, summary),
        }
    }
}

/// The element ids of a dialog's two buttons.
struct ButtonIds {
    cancel: &'static str,
    confirm: &'static str,
}

impl ButtonIds {
    /// The ids of the buttons of the dialog that keeps (`Keep`) or deletes the files.
    const fn of(data: RemoveData) -> Self {
        match data {
            RemoveData::Keep => Self {
                cancel: "remove-from-list-cancel",
                confirm: "remove-from-list-confirm",
            },
            RemoveData::Delete => Self {
                cancel: "remove-files-cancel",
                confirm: "remove-files-confirm",
            },
        }
    }
}

/// The focus of a dialog's two buttons, made once when it opens.
struct ButtonFocus {
    cancel: FocusHandle,
    confirm: FocusHandle,
}

/// Opens the dialog of `removal` in `window`. Its confirm button asks `view` to carry the removal
/// out.
pub(crate) fn open(
    removal: Removal,
    view: WeakEntity<MainView>,
    window: &mut Window,
    cx: &mut App,
) {
    let removal = Rc::new(removal);
    let focus = Rc::new(ButtonFocus {
        cancel: cx.focus_handle(),
        confirm: cx.focus_handle(),
    });
    // Remove takes the focus, so that Enter removes from the list; in the dialog that deletes,
    // Cancel does, so that Enter cancels.
    let first_focus = match removal.data {
        RemoveData::Keep => focus.confirm.clone(),
        RemoveData::Delete => focus.cancel.clone(),
    };
    let first_build = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        // The dialog is built again at every frame it is drawn in. Only the first build moves the
        // focus, so that it stays where the user puts it.
        if first_build.replace(false) {
            first_focus.focus(window, cx);
        }
        let connected = view
            .upgrade()
            .is_some_and(|view| view.read(cx).status().connected());
        dialog
            .close_button(false)
            .title(text_line("removal-title", &removal.title).test_support())
            .child(body(&removal, cx))
            .footer(footer(&removal, &focus, connected, &view))
    });
}

/// The names of the torrents, each on its own line, how many more there are, and what becomes of
/// the files.
fn body(removal: &Removal, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let files_color = match removal.data {
        RemoveData::Keep => theme.foreground,
        RemoveData::Delete => theme.danger,
    };
    v_flex()
        .gap_1()
        .children(
            removal
                .names
                .iter()
                .enumerate()
                .map(|(index, name)| text_line(("removal-name", index), name).test_support()),
        )
        .children(removal.more.as_ref().map(|more| {
            text_line("removal-more", more)
                .text_color(theme.muted_foreground)
                .test_support()
        }))
        .child(
            text_line("removal-files", &removal.files)
                .pt_2()
                .text_color(files_color)
                .test_support(),
        )
}

/// A line of the dialog's text, `id`, shown in full however long: it wraps instead of being cut.
/// Screen readers read it as a label whose value is the text.
fn text_line(id: impl Into<ElementId>, text: &SharedString) -> Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .w_full()
        .role(Role::Label)
        .aria_value(text.clone())
        .child(text.clone())
}

/// Cancel and the confirm button, in the key context where Enter presses the focused one. The
/// confirm button is enabled only while the daemon is `connected`.
fn footer(
    removal: &Rc<Removal>,
    focus: &ButtonFocus,
    connected: bool,
    view: &WeakEntity<MainView>,
) -> impl IntoElement + use<> {
    let ids = ButtonIds::of(removal.data);
    let cancel = Button::new(ids.cancel)
        .label(t!("remove-cancel"))
        .track_focus(&focus.cancel)
        .on_click(|_, window, cx| window.close_dialog(cx));
    let confirm = Button::new(ids.confirm)
        .track_focus(&focus.confirm)
        .disabled(!connected)
        .on_click({
            let removal = Rc::clone(removal);
            let view = view.clone();
            move |_, window, cx| confirm(&removal, &view, window, cx)
        });
    let confirm = match removal.data {
        RemoveData::Keep => confirm.primary().label(t!("remove-from-list-confirm")),
        RemoveData::Delete => confirm.danger().label(t!("remove-with-data-confirm")),
    };
    let (on_key_up, on_key_down) = (view.clone(), view.clone());
    div()
        .key_context(REMOVAL_FOOTER_CONTEXT)
        .capture_key_up(move |_, _, cx| {
            if let Some(view) = on_key_up.upgrade() {
                view.read(cx).release_held_key();
            }
        })
        // Before the focused button takes the key-down that its key-up would turn into a press.
        .capture_key_down(move |event, window, cx| {
            let repeat = event.is_held
                || on_key_down
                    .upgrade()
                    .is_some_and(|view| view.read(cx).key_held());
            if repeat {
                window.prevent_default();
            }
        })
        .child(DialogFooter::new().child(cancel).child(confirm))
}

/// Handles a press of the confirm button: asks `view` to carry `removal` out and closes the
/// dialog. While the daemon is not connected the view sends nothing, and the dialog stays open.
fn confirm(removal: &Removal, view: &WeakEntity<MainView>, window: &mut Window, cx: &mut App) {
    let hashes = removal.hashes.clone();
    match view.update(cx, |view, cx| {
        view.confirm_removal(removal.data, hashes, window, cx)
    }) {
        Ok(true) => window.close_dialog(cx),
        Ok(false) => {}
        Err(error) => {
            tracing::debug!(%error, "a removal was confirmed after the window closed");
            window.close_dialog(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use x4_core::human_size;

    use super::*;

    fn summary(count: usize, names: &[&str]) -> RemovalSummary {
        RemovalSummary {
            count,
            downloaded: human_size(3 << 30),
            names: names.iter().map(|name| (*name).to_owned()).collect(),
            more: count - names.len(),
        }
    }

    #[test]
    fn a_removal_says_what_its_summary_says_and_keeps_the_hashes() {
        let hashes = vec![
            x4_core::test_support::torrent("b").hash,
            x4_core::test_support::torrent("a").hash,
        ];
        let removal = Removal::new(
            RemoveData::Delete,
            hashes.clone(),
            &summary(7, &["b", "a", "c", "d", "e"]),
        );

        assert_eq!(removal.hashes, hashes);
        assert_eq!(removal.title, "Delete 7 torrents and their files?");
        assert_eq!(removal.names, ["b", "a", "c", "d", "e"]);
        assert_eq!(removal.more.as_deref(), Some("and 2 more"));
        assert_eq!(
            removal.files,
            "3.0 GiB of downloaded data will be deleted from the server. This cannot be undone."
        );

        let keep = Removal::new(RemoveData::Keep, hashes, &summary(2, &["b", "a"]));
        assert_eq!(keep.title, "Remove 2 torrents from the list?");
        assert_eq!(keep.more, None);
        assert_eq!(keep.files, "The downloaded files stay on the server.");
    }

    #[test]
    fn each_dialog_has_its_own_buttons() {
        let keep = ButtonIds::of(RemoveData::Keep);
        let delete = ButtonIds::of(RemoveData::Delete);
        let ids: std::collections::HashSet<&str> =
            [keep.cancel, keep.confirm, delete.cancel, delete.confirm].into();
        assert_eq!(ids.len(), 4, "no id is used twice: {ids:?}");
    }
}
