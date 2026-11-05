//! The showcase: a small daemon of torrents with names of freely distributable content, for the
//! screenshots, in place of 2000 named "Synthetic torrent NNNN", which look made up.

use std::sync::PoisonError;

use super::{Activity, SyntheticDaemon};

/// The names of the showcase's torrents, in the order the main window lists them by default (by
/// name, ignoring case), so that row `N` is the torrent `N + 1`. They are made up from distributions
/// of free software, films and texts that anyone may share, never from a real daemon.
pub(crate) const SHOWCASE_NAMES: [&str; 24] = [
    "archlinux-2026.11.01-x86_64.iso",
    "Big Buck Bunny (2008) 1080p",
    "Blender 5.0 Linux x64",
    "Charge (2022) Blender Studio",
    "Cosmos Laundromat (2015)",
    "debian-13.2.0-amd64-netinst.iso",
    "Elephants Dream (2006)",
    "Fedora-Workstation-Live-x86_64-43",
    "Free Music Archive — 2025 picks",
    "kdenlive-25.08-sample-projects",
    "LibreOffice 25.8 all platforms",
    "linuxmint-22.2-cinnamon-64bit.iso",
    "Metropolis (1927)",
    "NASA Apollo 11 mission audio",
    "Night of the Living Dead (1968)",
    "openSUSE-Tumbleweed-DVD-x86_64-Current.iso",
    "Project Gutenberg — top 100 ebooks",
    "Sintel (2010) 4K",
    "Spring (2019) Blender Open Movie",
    "Tears of Steel (2012) 1080p",
    "The General (1926)",
    "ubuntu-26.04-desktop-amd64.iso",
    "Wikimedia Commons picture of the year 2025",
    "Wikipedia enwiki dump 2026-10",
];

impl SyntheticDaemon {
    /// A daemon of the torrents of [`SHOWCASE_NAMES`], named in that order, made as
    /// [`SyntheticDaemon::new`] makes torrents and behaving as `activity` says.
    ///
    /// The mix of states is that of the torrents of a ticking daemon, whichever `activity` is: a
    /// static daemon draws less for each torrent and so picks other kinds, and among 24 of those
    /// none is an error. A static showcase is the ticking one at rest.
    pub(crate) fn showcase(activity: Activity) -> Self {
        let count = u32::try_from(SHOWCASE_NAMES.len()).unwrap_or(u32::MAX);
        let mut daemon = Self::new(count, Activity::Ticking);
        let state = daemon
            .state
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        state.activity = activity;
        for (torrent, name) in state.torrents.iter_mut().zip(SHOWCASE_NAMES) {
            name.clone_into(&mut torrent.name);
            if activity == Activity::Static {
                torrent.rate_down = 0;
                torrent.rate_up = 0;
                torrent.peers_sending_to_us = 0;
                torrent.peers_getting_from_us = 0;
            }
        }
        daemon
    }
}
