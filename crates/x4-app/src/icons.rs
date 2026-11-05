//! The icons the app draws: gpui-component's default set, and the few icons of the full Lucide
//! catalog it does not hold (the turtle of the alternative speeds, the trash can of Remove, the
//! file with a plus and the magnet of Add file and Add magnet, and the lock and the arrow of an
//! encrypted and an incoming connection in the Peers tab), embedded one by one so that the binary
//! does not carry the whole catalog. Only the icons the app draws are added.
//!
//! Both come from `gpui-kit-assets`, which ships Lucide's SVGs with Lucide's license notice
//! (ISC; the icons Lucide took from Feather, among them the trash can, are MIT).
//! [`AppAssets`] is the asset source of the application: it looks among the added icons first and
//! falls back to the default set, which reports a path it does not know as an error.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(
    AddedIcons,
    [Turtle, Trash, FilePlus, Magnet, Lock, ArrowDownLeft]
);

/// The asset source of the application: the added icons, then gpui-component's default set.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match AddedIcons.load(path)? {
            Some(icon) => Ok(Some(icon)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = AddedIcons.list(path)?;
        paths.extend(Assets.list(path)?);
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::assets::IconName;

    use super::*;

    fn loads(path: &str) -> bool {
        matches!(AppAssets.load(path), Ok(Some(bytes)) if bytes.starts_with(b"<svg"))
    }

    #[test]
    fn the_added_icons_and_the_default_ones_load() {
        for icon in [
            IconName::Turtle,
            IconName::Trash,
            IconName::FilePlus,
            IconName::Magnet,
            IconName::Lock,
            IconName::ArrowDownLeft,
            IconName::Play,
            IconName::Pause,
            IconName::Search,
            // The globe of a peer without a flag and the gear of the toolbar's Connection button
            // are among gpui-component's own.
            IconName::Globe,
            IconName::Settings,
        ] {
            assert!(loads(&icon.path()), "{icon:?}");
        }
    }

    #[test]
    fn the_added_icons_are_not_in_the_default_set() {
        for icon in [
            IconName::Turtle,
            IconName::Trash,
            IconName::FilePlus,
            IconName::Magnet,
            IconName::Lock,
            IconName::ArrowDownLeft,
        ] {
            assert!(Assets.load(&icon.path()).is_err(), "{icon:?} is a default");
        }
    }

    #[test]
    fn an_unknown_path_is_an_error_and_every_icon_is_listed() {
        assert!(AppAssets.load("icons/no-such-icon.svg").is_err());
        // Icons of the catalog that the app does not draw are not embedded.
        for icon in [IconName::Link, IconName::FileInput] {
            assert!(AppAssets.load(&icon.path()).is_err(), "{icon:?}");
        }
        let listed = AppAssets.list("icons/").expect("the icons are listed");
        assert!(listed.iter().any(|path| path == "icons/turtle.svg"));
        assert!(listed.iter().any(|path| path == "icons/play.svg"));
    }
}
