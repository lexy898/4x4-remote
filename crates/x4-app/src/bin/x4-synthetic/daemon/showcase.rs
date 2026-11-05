//! The showcase: a small daemon of torrents with names of freely distributable content, for the
//! screenshots, in place of 2000 named "Synthetic torrent NNNN", which look made up.

use std::sync::PoisonError;

use super::files::{PLAN_STREAM, Plan};
use super::torrent::Torrent;
use super::{Activity, Lcg, SyntheticDaemon};

/// A torrent of the showcase: what it is called and how big it is.
pub(crate) struct ShowcaseTorrent {
    pub(crate) name: &'static str,
    /// Its size in MiB, about what such content weighs, so that nobody who knows it finds a 42 GiB
    /// Linux ISO or a 59 GiB silent film.
    pub(crate) mib: u32,
}

/// Shorthand for the table: a name and a size in MiB.
const fn torrent(name: &'static str, mib: u32) -> ShowcaseTorrent {
    ShowcaseTorrent { name, mib }
}

/// The torrents of the showcase, in the order the main window lists them by default (by name,
/// ignoring case), so that row `N` is the torrent `N + 1`. They are made up from distributions of
/// free software, films and texts that anyone may share, never from a real daemon.
pub(crate) const SHOWCASE: [ShowcaseTorrent; 24] = [
    torrent("archlinux-2026.11.01-x86_64.iso", 1_440),
    torrent("Big Buck Bunny (2008) 1080p", 720),
    torrent("Blender 5.0 Linux x64", 360),
    torrent("Charge (2022) Blender Studio", 1_640),
    torrent("Cosmos Laundromat (2015)", 2_660),
    torrent("debian-13.2.0-amd64-netinst.iso", 665),
    torrent("Elephants Dream (2006)", 820),
    torrent("Fedora-Workstation-Live-x86_64-43", 2_355),
    torrent("Free Music Archive — 2025 picks", 4_300),
    torrent("kdenlive-25.08-sample-projects", 1_125),
    torrent("LibreOffice 25.8 all platforms", 1_945),
    torrent("linuxmint-22.2-cinnamon-64bit.iso", 2_970),
    torrent("Metropolis (1927)", 4_505),
    torrent("NASA Apollo 11 mission audio", 12_288),
    torrent("Night of the Living Dead (1968)", 1_536),
    torrent("openSUSE-Tumbleweed-DVD-x86_64-Current.iso", 4_405),
    torrent("Project Gutenberg — top 100 ebooks", 205),
    torrent("Sintel (2010) 4K", 5_735),
    torrent("Spring (2019) Blender Open Movie", 1_230),
    torrent("Tears of Steel (2012) 1080p", 1_125),
    torrent("The General (1926)", 2_250),
    torrent("ubuntu-26.04-desktop-amd64.iso", 6_040),
    torrent("Wikimedia Commons picture of the year 2025", 3_070),
    torrent("Wikipedia enwiki dump 2026-10", 24_576),
];

impl SyntheticDaemon {
    /// A daemon of the torrents of [`SHOWCASE`], named and sized as it says, in that order, made as
    /// [`SyntheticDaemon::new`] makes torrents and behaving as `activity` says.
    ///
    /// The mix of states is that of the torrents of a ticking daemon, whichever `activity` is: a
    /// static daemon draws less for each torrent and so picks other kinds, and among 24 of those
    /// none is an error. A static showcase is the ticking one at rest.
    pub(crate) fn showcase(activity: Activity) -> Self {
        let count = u32::try_from(SHOWCASE.len()).unwrap_or(u32::MAX);
        let mut daemon = Self::new(count, Activity::Ticking);
        let state = daemon
            .state
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        state.activity = activity;
        for (torrent, row) in state.torrents.iter_mut().zip(&SHOWCASE) {
            fit(torrent, row);
            if activity == Activity::Static {
                rest(torrent);
            }
        }
        daemon
    }
}

/// Makes `torrent` the one `row` of the showcase says: its name, its size, with the same share of
/// it left to download, and the plan of its files that its id would give it but for the daemon's
/// two stress cases. The first 24 torrents of the seed hold both the 2000-file album and the
/// 50 000-file archive, which are no content of a showcase. Its files and pieces are made from all
/// that when something first asks for them.
fn fit(torrent: &mut Torrent, row: &ShowcaseTorrent) {
    let total_kib = row.mib.saturating_mul(1024);
    torrent.left_kib = share_of(torrent.left_kib, torrent.total_kib, total_kib);
    torrent.total_kib = total_kib;
    row.name.clone_into(&mut torrent.name);
    torrent.plan = Plan::pick(&mut Lcg::stream(torrent.id, PLAN_STREAM));
}

/// Stops what moves in `torrent`: no rates, and so no peers the rates are shared out among.
fn rest(torrent: &mut Torrent) {
    torrent.rate_down = 0;
    torrent.rate_up = 0;
    torrent.peers_sending_to_us = 0;
    torrent.peers_getting_from_us = 0;
}

/// `left` of `total`, as the same share of `new_total`: nothing stays nothing, and something is
/// at least one KiB and at most all of it.
fn share_of(left: u32, total: u32, new_total: u32) -> u32 {
    if left == 0 {
        return 0;
    }
    let scaled = u64::from(left) * u64::from(new_total) / u64::from(total.max(1));
    u32::try_from(scaled)
        .unwrap_or(new_total)
        .clamp(1, new_total)
}
