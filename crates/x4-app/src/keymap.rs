//! The one table of key bindings: the actions the keys run, the key contexts they run in, and
//! [`BINDINGS`], which says which key runs which action where. [`key_bindings`] turns the table
//! into gpui's bindings; `app::init` binds them after the libraries' own. The actions on torrents
//! themselves are declared in [`actions`](crate::actions).
//!
//! Where a key works is its context:
//!
//! - the torrent actions, the selection keys among them, work in the torrent list (`TorrentList`)
//!   while no menu of it is open (`!PopupMenu`): the context menu is drawn inside the list, and
//!   Delete there must not act on the torrents behind it. The search box lies outside the list, so
//!   Ctrl+A, Ctrl+C, Delete or Space there edit or copy its text and never reach the torrents;
//! - the window's own keys (Ctrl+F, `/`, and Ctrl+T and Ctrl+L for the turtle and the speed
//!   limits) work in the whole main window (`MainWindow`), the search box and the popover of the
//!   limits included: their text fields bind neither Ctrl+T nor Ctrl+L;
//! - in the footer of a removal dialog (`RemovalDialogFooter`), Enter is bound to nothing, so that
//!   it presses the button that has the focus. The dialog's own Enter would confirm the dialog
//!   whatever has the focus, and Enter on a focused Cancel must never delete files;
//! - only Ctrl+Q has no context. A binding without one also fires in a text field, before the
//!   field's own binding of the same key, so it would take the key from the search box.
//!
//! Of two bindings of a key, the one in the deeper context goes first, and at the same depth the
//! one bound later. If its action is not handled on the way to the window's root, or the handler
//! passes it on, the next binding of the key is tried. gpui-component's table (context
//! `DataTable`, inside the torrent list) binds Escape to clearing its own selected row and passes
//! the key on when it has none; Escape in the list then clears the selection here. The table also
//! binds Up, Down, Home, End, Page Up, Page Down, Tab and the side arrows, and those are bound
//! again here in its context, later, so that ours win (see [`BINDINGS`]). While a menu is open (the
//! context menu, or a menu of the toolbar), its own Up, Down, Enter and Escape go first, and Tab,
//! Shift+Tab, Ctrl+F, `/`, Ctrl+L and Ctrl+T do nothing, nor do Home, End, Page Up and Page Down
//! in the context menu: the menu keeps the keys until it closes.

use std::rc::Rc;

use gpui_kit::{Action, App, KeyBinding, KeyBindingContextPredicate, NoAction, actions};

use crate::actions::{
    CopyHashes, CopyMagnetLinks, PauseTorrents, ReannounceTorrents, RemoveTorrents,
    RemoveTorrentsWithData, StartTorrents, StartTorrentsNow, ToggleTorrents, VerifyTorrents,
};

actions!(
    x4,
    [
        /// Quits the application.
        Quit,
        /// Moves the focus to the next control, out of the torrent table.
        FocusNext,
        /// Moves the focus to the previous control, out of the torrent table.
        FocusPrevious,
        /// Moves the focus to the search box and selects its text.
        FocusSearch,
        /// Empties the search box and hands the focus back to the torrent table.
        ClearSearch,
        /// Selects every torrent the list shows, and only those.
        SelectAllTorrents,
        /// Selects no torrent.
        DeselectTorrents,
        /// Selects only the torrent above the lead (the row the keys move from).
        MoveSelectionUp,
        /// Selects only the torrent below the lead.
        MoveSelectionDown,
        /// Selects only the first torrent of the list.
        MoveSelectionToFirst,
        /// Selects only the last torrent of the list.
        MoveSelectionToLast,
        /// Selects only the torrent a page above the lead.
        MoveSelectionPageUp,
        /// Selects only the torrent a page below the lead.
        MoveSelectionPageDown,
        /// Moves the lead up and selects the range from the anchor to it.
        ExtendSelectionUp,
        /// Moves the lead down and selects the range from the anchor to it.
        ExtendSelectionDown,
        /// Moves the lead to the first torrent and selects the range from the anchor to it.
        ExtendSelectionToFirst,
        /// Moves the lead to the last torrent and selects the range from the anchor to it.
        ExtendSelectionToLast,
        /// Switches the alternative speed limits (the turtle) on or off.
        ToggleAltSpeed,
        /// Opens the popover of the normal speed limits.
        EditSpeedLimits,
    ]
);

/// The key context of the main window's content.
pub(crate) const MAIN_CONTEXT: &str = "MainWindow";
/// The main window while no text field has the focus: a key bound here can still be typed in one.
const MAIN_OUTSIDE_TEXT_CONTEXT: &str = "MainWindow && !Input";
/// The key context of the search box, around gpui-component's `Input` context.
pub(crate) const SEARCH_CONTEXT: &str = "SearchBox";
/// The key context of the torrent list, around gpui-component's table.
pub(crate) const TORRENT_LIST_CONTEXT: &str = "TorrentList";
/// The key context of gpui-component's table.
const TABLE_CONTEXT: &str = "DataTable";
/// The table while no menu of it is open: the context menu is drawn inside the table, so without
/// `!PopupMenu` its keys would move the selection behind the menu.
const TABLE_NO_MENU: &str = "DataTable && !PopupMenu";
/// An open menu of the table: its context menu.
const TABLE_MENU: &str = "DataTable > PopupMenu";
/// Any open menu: the table's context menu, or a menu of the toolbar (Remove, Columns).
const ANY_MENU: &str = "PopupMenu";
/// Where the torrent actions work: in the torrent list, while no menu of it is open. An open
/// context menu is drawn inside the list, so without `!PopupMenu` Delete would act on the
/// torrents behind it.
const TORRENTS: &str = "TorrentList && !PopupMenu";
/// The key context around the buttons of a removal dialog, inside gpui-component's `Dialog`.
pub(crate) const REMOVAL_FOOTER_CONTEXT: &str = "RemovalDialogFooter";

/// One row of the key table: a key, the action it runs and where.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// The keystroke, in gpui's spelling: modifiers first, joined by `-`, such as `ctrl-shift-a`.
    pub keys: &'static str,
    /// Makes the action the key runs.
    pub action: fn() -> Box<dyn Action>,
    /// The key context predicate the key works in; `None` is everywhere, text fields included.
    pub context: Option<&'static str>,
}

/// Every key binding of the app, in the order they are bound: of two bindings of a key in the
/// same context, the later one wins.
pub const BINDINGS: &[Binding] = &[
    Binding {
        keys: "ctrl-q",
        action: || Box::new(Quit),
        context: None,
    },
    Binding {
        keys: "ctrl-f",
        action: || Box::new(FocusSearch),
        context: Some(MAIN_CONTEXT),
    },
    Binding {
        keys: "/",
        action: || Box::new(FocusSearch),
        context: Some(MAIN_OUTSIDE_TEXT_CONTEXT),
    },
    Binding {
        keys: "ctrl-t",
        action: || Box::new(ToggleAltSpeed),
        context: Some(MAIN_CONTEXT),
    },
    Binding {
        keys: "ctrl-l",
        action: || Box::new(EditSpeedLimits),
        context: Some(MAIN_CONTEXT),
    },
    // Shallower than the text field's own Escape, which goes first: it closes the field's menu or
    // drops extra cursors, and passes the key on when there is nothing of that kind.
    Binding {
        keys: "escape",
        action: || Box::new(ClearSearch),
        context: Some(SEARCH_CONTEXT),
    },
    // The table binds Tab and the side arrows to moving between its columns. A list of torrents
    // has rows only: Tab leaves it, and the side arrows do nothing.
    Binding {
        keys: "tab",
        action: || Box::new(FocusNext),
        context: Some(TABLE_CONTEXT),
    },
    Binding {
        keys: "shift-tab",
        action: || Box::new(FocusPrevious),
        context: Some(TABLE_CONTEXT),
    },
    Binding {
        keys: "left",
        action: || Box::new(NoAction),
        context: Some(TABLE_CONTEXT),
    },
    Binding {
        keys: "right",
        action: || Box::new(NoAction),
        context: Some(TABLE_CONTEXT),
    },
    // The table moves its own selected row from that row, and starts from the top when it has
    // none. Its row is the lead only while the lead is selected, so its moving keys move the lead
    // here instead, from wherever it is. A key that lands on the lead also selects only the lead,
    // which the table, seeing its row unchanged, would not.
    Binding {
        keys: "up",
        action: || Box::new(MoveSelectionUp),
        context: Some(TABLE_CONTEXT),
    },
    Binding {
        keys: "down",
        action: || Box::new(MoveSelectionDown),
        context: Some(TABLE_CONTEXT),
    },
    // A menu is modal. Up and Down in it move its highlight (the menu's own bindings are deeper).
    // Neither Tab, Shift+Tab, Ctrl+F, `/` nor Ctrl+L (which opens the speed limits) may take the
    // focus out of an open menu, which does not close when it loses the focus: the keys of the
    // list or the search box would then act behind it. Ctrl+T does nothing behind it either.
    Binding {
        keys: "tab",
        action: || Box::new(NoAction),
        context: Some(ANY_MENU),
    },
    Binding {
        keys: "shift-tab",
        action: || Box::new(NoAction),
        context: Some(ANY_MENU),
    },
    Binding {
        keys: "ctrl-f",
        action: || Box::new(NoAction),
        context: Some(ANY_MENU),
    },
    Binding {
        keys: "/",
        action: || Box::new(NoAction),
        context: Some(ANY_MENU),
    },
    Binding {
        keys: "ctrl-l",
        action: || Box::new(NoAction),
        context: Some(ANY_MENU),
    },
    Binding {
        keys: "ctrl-t",
        action: || Box::new(NoAction),
        context: Some(ANY_MENU),
    },
    // The context menu has no Home, End, Page Up or Page Down: ours do not work behind it, and the
    // table's own are bound to nothing in it, which is deeper than the table, so that they do
    // nothing either.
    Binding {
        keys: "home",
        action: || Box::new(NoAction),
        context: Some(TABLE_MENU),
    },
    Binding {
        keys: "end",
        action: || Box::new(NoAction),
        context: Some(TABLE_MENU),
    },
    Binding {
        keys: "pageup",
        action: || Box::new(NoAction),
        context: Some(TABLE_MENU),
    },
    Binding {
        keys: "pagedown",
        action: || Box::new(NoAction),
        context: Some(TABLE_MENU),
    },
    Binding {
        keys: "home",
        action: || Box::new(MoveSelectionToFirst),
        context: Some(TABLE_NO_MENU),
    },
    Binding {
        keys: "end",
        action: || Box::new(MoveSelectionToLast),
        context: Some(TABLE_NO_MENU),
    },
    Binding {
        keys: "pageup",
        action: || Box::new(MoveSelectionPageUp),
        context: Some(TABLE_NO_MENU),
    },
    Binding {
        keys: "pagedown",
        action: || Box::new(MoveSelectionPageDown),
        context: Some(TABLE_NO_MENU),
    },
    Binding {
        keys: "ctrl-a",
        action: || Box::new(SelectAllTorrents),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "ctrl-shift-a",
        action: || Box::new(DeselectTorrents),
        context: Some(TORRENTS),
    },
    // The table's own Escape goes first and clears the selection through its selected row; when
    // the table has no row (the lead is not selected), it passes the key on to this one.
    Binding {
        keys: "escape",
        action: || Box::new(DeselectTorrents),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "shift-up",
        action: || Box::new(ExtendSelectionUp),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "shift-down",
        action: || Box::new(ExtendSelectionDown),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "shift-home",
        action: || Box::new(ExtendSelectionToFirst),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "shift-end",
        action: || Box::new(ExtendSelectionToLast),
        context: Some(TORRENTS),
    },
    // The actions on the selected torrents, with Transmission's keys where it has one. Verify is
    // Ctrl+Shift+R rather than Transmission's Ctrl+V, which stays free for pasting a magnet link:
    // a check reads every byte of the data, hours on a small server.
    Binding {
        keys: "ctrl-s",
        action: || Box::new(StartTorrents),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "ctrl-p",
        action: || Box::new(PauseTorrents),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "ctrl-shift-s",
        action: || Box::new(StartTorrentsNow),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "space",
        action: || Box::new(ToggleTorrents),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "ctrl-shift-r",
        action: || Box::new(VerifyTorrents),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "ctrl-r",
        action: || Box::new(ReannounceTorrents),
        context: Some(TORRENTS),
    },
    // Both only open a dialog that asks first; Shift+Delete never deletes by itself.
    Binding {
        keys: "delete",
        action: || Box::new(RemoveTorrents),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "shift-delete",
        action: || Box::new(RemoveTorrentsWithData),
        context: Some(TORRENTS),
    },
    // The search box lies outside the list and keeps its own Ctrl+C, which copies its text.
    Binding {
        keys: "ctrl-c",
        action: || Box::new(CopyMagnetLinks),
        context: Some(TORRENTS),
    },
    Binding {
        keys: "ctrl-shift-c",
        action: || Box::new(CopyHashes),
        context: Some(TORRENTS),
    },
    // Deeper than the dialog's own Enter, which confirms the dialog whatever has the focus: bound
    // to nothing here, Enter goes on to the focused button and presses it, as Space does.
    Binding {
        keys: "enter",
        action: || Box::new(NoAction),
        context: Some(REMOVAL_FOOTER_CONTEXT),
    },
];

/// gpui's bindings for [`BINDINGS`], in the same order. An entry whose keys or context do not
/// parse is logged and left out; the rest still work.
///
/// gpui takes any word as the name of a key, so a misspelled key parses and never fires; the
/// tests check every key name.
#[must_use]
pub fn key_bindings(cx: &App) -> Vec<KeyBinding> {
    BINDINGS
        .iter()
        .filter_map(|binding| {
            let context = binding
                .context
                .map(KeyBindingContextPredicate::parse)
                .transpose()
                .inspect_err(|error| {
                    tracing::error!(
                        keys = binding.keys,
                        context = binding.context,
                        %error,
                        "a key binding has a context that does not parse; it is left out"
                    );
                })
                .ok()?;
            KeyBinding::load(
                binding.keys,
                (binding.action)(),
                context.map(Rc::new),
                false,
                None,
                cx,
            )
            .inspect_err(|error| {
                tracing::error!(
                    keys = binding.keys,
                    %error,
                    "a key binding has keys that do not parse; it is left out"
                );
            })
            .ok()
        })
        .collect()
}
