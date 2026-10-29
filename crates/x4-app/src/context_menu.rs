//! The context menu of the torrent list, which a right-click on a row opens: Start, Pause, Start
//! now, Verify and Reannounce; Copy magnet link and Copy hash; "Remove from list…" and "Remove and
//! delete files…", with separators between the groups.
//!
//! The menu takes its targets when it opens, after the right-click has selected the row: the
//! selected rows the list shows then, in the order shown. Its items act on those, whatever a
//! snapshot does to the list while it is open, as the removal dialogs do. Each item is enabled as
//! the targets' [`Availability`] says and shows the key of its action, which it looks up in the
//! torrent list (the menu's action context is the table). A click, or Enter on a highlighted item,
//! goes to the [`MenuHandler`], which asks the main view: the view reads the availability again
//! then. gpui-component lets Enter reach a disabled item that the pointer highlights, so an item
//! disabled when the menu opened does nothing at all, even if the daemon is back by then.
//!
//! What the menu offers is decided here and unit-tested ([`ITEMS`], [`MenuChoice::enabled`]); the
//! table's delegate builds it ([`torrent_menu`]).

use std::rc::Rc;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{Action, App, FocusHandle, SharedString, Window};
use x4_core::{Availability, Row, availability};
use x4_rpc::RemoveData;

use crate::actions::{
    CopyHashes, CopyMagnetLinks, PauseTorrents, ReannounceTorrents, RemoveTorrents,
    RemoveTorrentsWithData, StartTorrents, StartTorrentsNow, TorrentRequest, VerifyTorrents,
    decide,
};
use crate::i18n::t;

/// What an item of the menu does to the torrents the menu took when it opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuChoice {
    /// Start them, or queue them.
    Start,
    /// Pause them.
    Pause,
    /// Start them now, ahead of the queue.
    StartNow,
    /// Verify their data.
    Verify,
    /// Ask their trackers for more peers.
    Reannounce,
    /// Copy their magnet links.
    CopyMagnetLinks,
    /// Copy their hashes.
    CopyHashes,
    /// Ask whether to remove them, keeping or deleting their files.
    Remove(RemoveData),
}

/// The items of the menu, top to bottom; `None` is a separator.
pub(crate) const ITEMS: [Option<MenuChoice>; 11] = [
    Some(MenuChoice::Start),
    Some(MenuChoice::Pause),
    Some(MenuChoice::StartNow),
    Some(MenuChoice::Verify),
    Some(MenuChoice::Reannounce),
    None,
    Some(MenuChoice::CopyMagnetLinks),
    Some(MenuChoice::CopyHashes),
    None,
    Some(MenuChoice::Remove(RemoveData::Keep)),
    Some(MenuChoice::Remove(RemoveData::Delete)),
];

impl MenuChoice {
    /// The request the item sends, as its key does; `None` for copying and removing, which are
    /// not actions on the server or ask first.
    pub(crate) fn request(self) -> Option<TorrentRequest> {
        match self {
            Self::Start => Some(TorrentRequest::Start),
            Self::Pause => Some(TorrentRequest::Pause),
            Self::StartNow => Some(TorrentRequest::StartNow),
            Self::Verify => Some(TorrentRequest::Verify),
            Self::Reannounce => Some(TorrentRequest::Reannounce),
            Self::CopyMagnetLinks | Self::CopyHashes | Self::Remove(_) => None,
        }
    }

    /// The item's label.
    fn label(self) -> SharedString {
        match self {
            Self::Start => t!("action-start"),
            Self::Pause => t!("action-pause"),
            Self::StartNow => t!("action-start-now"),
            Self::Verify => t!("action-verify"),
            Self::Reannounce => t!("action-reannounce"),
            Self::CopyMagnetLinks => t!("action-copy-magnet"),
            Self::CopyHashes => t!("action-copy-hash"),
            Self::Remove(RemoveData::Keep) => t!("remove-from-list-menu"),
            Self::Remove(RemoveData::Delete) => t!("remove-with-data-menu"),
        }
        .into()
    }

    /// The action of the same thing, whose key the item shows.
    fn action(self) -> Box<dyn Action> {
        match self {
            Self::Start => Box::new(StartTorrents),
            Self::Pause => Box::new(PauseTorrents),
            Self::StartNow => Box::new(StartTorrentsNow),
            Self::Verify => Box::new(VerifyTorrents),
            Self::Reannounce => Box::new(ReannounceTorrents),
            Self::CopyMagnetLinks => Box::new(CopyMagnetLinks),
            Self::CopyHashes => Box::new(CopyHashes),
            Self::Remove(RemoveData::Keep) => Box::new(RemoveTorrents),
            Self::Remove(RemoveData::Delete) => Box::new(RemoveTorrentsWithData),
        }
    }

    /// Whether the item is enabled for `targets`, which allow `availability`. An action's item
    /// asks [`decide`], as its key and its toolbar button do.
    pub(crate) fn enabled(self, targets: &[&Row], availability: Availability) -> bool {
        match self {
            Self::CopyMagnetLinks => availability.copy_magnet,
            Self::CopyHashes => availability.copy_hash,
            Self::Remove(_) => availability.remove,
            Self::Start | Self::Pause | Self::StartNow | Self::Verify | Self::Reannounce => self
                .request()
                .is_some_and(|request| decide(request, targets, availability).is_some()),
        }
    }
}

/// What a click on an item does: the choice, with the rows the menu took when it opened.
pub(crate) type MenuHandler = Rc<dyn Fn(MenuChoice, &[Row], &mut Window, &mut App)>;

/// `menu` with the items of [`ITEMS`] for `targets`, enabled as they allow while the daemon is
/// `connected` or not. The items show the keys of their actions as the torrent list has them,
/// through the table's `focus`, and a click goes to `on_choice` with `targets`.
pub(crate) fn torrent_menu(
    menu: PopupMenu,
    targets: &Rc<[Row]>,
    connected: bool,
    focus: Option<FocusHandle>,
    on_choice: Option<&MenuHandler>,
) -> PopupMenu {
    let refs: Vec<&Row> = targets.iter().collect();
    let allowed = availability(connected, &refs);
    let menu = match focus {
        Some(focus) => menu.action_context(focus),
        None => menu,
    };
    ITEMS.iter().fold(menu, |menu, item| {
        let Some(choice) = *item else {
            return menu.separator();
        };
        let targets = Rc::clone(targets);
        let on_choice = on_choice.cloned();
        let enabled = choice.enabled(&refs, allowed);
        menu.item(
            PopupMenuItem::new(choice.label())
                // The action only shows its key; the click goes to the handler.
                .action(choice.action())
                .disabled(!enabled)
                .on_click(move |_, window, cx| {
                    if !enabled {
                        tracing::debug!(?choice, "a disabled menu item was chosen; nothing done");
                        return;
                    }
                    if let Some(on_choice) = &on_choice {
                        on_choice(choice, &targets, window, cx);
                    }
                }),
        )
    })
}

#[cfg(test)]
mod tests {
    use x4_core::test_support::torrent;
    use x4_core::{DisplayState, Torrent};

    use super::*;

    fn rows(states: &[DisplayState]) -> Vec<Row> {
        states
            .iter()
            .map(|&state| {
                Row::from(&Torrent {
                    state,
                    ..torrent("a")
                })
            })
            .collect()
    }

    /// The labels of the menu, a separator as "-".
    fn labels() -> Vec<String> {
        ITEMS
            .iter()
            .map(|item| item.map_or_else(|| "-".to_owned(), |choice| choice.label().to_string()))
            .collect()
    }

    /// Which items are enabled for rows in `states`, with the daemon `connected` or not.
    fn enabled(states: &[DisplayState], connected: bool) -> Vec<bool> {
        let rows = rows(states);
        let targets: Vec<&Row> = rows.iter().collect();
        let allowed = availability(connected, &targets);
        ITEMS
            .iter()
            .flatten()
            .map(|choice| choice.enabled(&targets, allowed))
            .collect()
    }

    #[test]
    fn the_menu_lists_every_action_in_groups() {
        assert_eq!(
            labels(),
            [
                "Start",
                "Pause",
                "Start now",
                "Verify",
                "Reannounce",
                "-",
                "Copy magnet link",
                "Copy hash",
                "-",
                "Remove from list…",
                "Remove and delete files…",
            ]
        );
    }

    #[test]
    fn items_are_enabled_as_the_targets_allow() {
        // Start, Pause, Start now, Verify, Reannounce, Copy magnet, Copy hash, Remove ×2.
        assert_eq!(
            enabled(&[DisplayState::Paused], true),
            [true, false, true, true, false, true, true, true, true]
        );
        assert_eq!(
            enabled(&[DisplayState::Seeding], true),
            [false, true, false, true, true, true, true, true, true]
        );
    }

    #[test]
    fn only_copy_hash_works_while_disconnected() {
        assert_eq!(
            enabled(&[DisplayState::Paused, DisplayState::Downloading], false),
            [false, false, false, false, false, false, true, false, false]
        );
        assert_eq!(enabled(&[], true), [false; 9], "nothing to act on");
    }

    #[test]
    fn every_item_shows_the_key_of_its_own_action() {
        let names: Vec<&str> = ITEMS
            .iter()
            .flatten()
            .map(|choice| choice.action().name())
            .collect();
        assert_eq!(
            names,
            [
                "x4::StartTorrents",
                "x4::PauseTorrents",
                "x4::StartTorrentsNow",
                "x4::VerifyTorrents",
                "x4::ReannounceTorrents",
                "x4::CopyMagnetLinks",
                "x4::CopyHashes",
                "x4::RemoveTorrents",
                "x4::RemoveTorrentsWithData",
            ]
        );
    }
}
