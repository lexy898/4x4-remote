//! The files and pieces of the made-up torrents, and what `torrent_set` does to the files.
//!
//! What a torrent's files are (their names, sizes and pieces) follows from its [`Plan`] and its id
//! alone, so it is the same on every run (`layout`). The state that changes is made when a read or
//! a `torrent_set` first needs it: which files are wanted, with which priority, and which pieces
//! are present. It starts from how much of the torrent the daemon says is left, and from then
//! on the pieces are the truth: a download adds pieces, from the start of the torrent when it is
//! sequential and scattered when it is not, and what is left follows from them.
//!
//! All sizes are whole KiB and a piece is a power of two of at least 64 KiB, so the sizes the list
//! reports (in KiB) agree exactly with what the pieces hold. The file lists of a `torrent_set` are
//! in `ops`.

mod layout;
mod ops;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};

use super::{Fields, KIB, Kind, Lcg, Torrent};
use layout::{Layout, to_index, to_wide};

pub(super) use layout::{PLAN_STREAM, Plan};
pub(super) use ops::FileOps;

/// The random stream that scatters the pieces a torrent has.
const PIECES_STREAM: u8 = 5;
/// The fields that need a torrent's files and pieces; the file count alone does not.
const FILE_FIELDS: [&str; 8] = [
    "files",
    "file_stats",
    "priorities",
    "wanted",
    "bytes_completed",
    "pieces",
    "piece_count",
    "piece_size",
];

/// A random number below `bound`, as an index.
fn pick_below(random: &mut Lcg, bound: usize) -> usize {
    to_index(u64::from(
        random.below(u32::try_from(bound).unwrap_or(u32::MAX)),
    ))
}

/// What a torrent's files are doing: which are wanted, their priority, and the pieces present.
pub(super) struct FileState {
    layout: Layout,
    wanted: Vec<bool>,
    /// -1, 0 or 1, as the wire has it.
    priority: Vec<i8>,
    /// Which pieces are present.
    have: Vec<bool>,
    /// Which pieces hold data of a wanted file.
    needed: Vec<bool>,
    random: Lcg,
    /// Bytes downloaded that did not fill a piece yet.
    credit: u64,
    /// No piece before it is both needed and missing, so a sequential pick starts from here.
    cursor: usize,
}

impl FileState {
    /// The state of a torrent with `layout` and `left_kib` KiB still to download, with everything
    /// wanted at the normal priority. The pieces it has are the first ones when `sequential`, and
    /// scattered otherwise.
    fn new(layout: Layout, left_kib: u32, sequential: bool, id: u32) -> Self {
        let pieces = layout.piece_count();
        let files = layout.files.len();
        let left = u64::from(left_kib) * KIB;
        // Whatever is left, a piece is missing: the progress shown is never 100% while it is.
        let present = if left == 0 {
            pieces
        } else {
            to_index(to_wide(pieces) * layout.total.saturating_sub(left) / layout.total)
                .min(pieces.saturating_sub(1))
        };
        let mut state = Self {
            layout,
            wanted: vec![true; files],
            priority: vec![0; files],
            have: vec![false; pieces],
            needed: vec![true; pieces],
            random: Lcg::stream(id, PIECES_STREAM),
            credit: 0,
            cursor: 0,
        };
        if sequential {
            state
                .have
                .iter_mut()
                .take(present)
                .for_each(|have| *have = true);
        } else {
            // A random subset of that size, by shuffling its share of a list of all the pieces.
            let mut order: Vec<usize> = (0..pieces).collect();
            for position in 0..present {
                let other = position + pick_below(&mut state.random, pieces - position);
                order.swap(position, other);
                if let Some(have) = order
                    .get(position)
                    .and_then(|&piece| state.have.get_mut(piece))
                {
                    *have = true;
                }
            }
        }
        state
    }

    /// How many files the torrent has.
    pub(super) fn file_count(&self) -> usize {
        self.layout.files.len()
    }

    /// Whether piece `piece` is wanted and not here yet.
    fn is_open(&self, piece: usize) -> bool {
        self.needed.get(piece) == Some(&true) && self.have.get(piece) == Some(&false)
    }

    /// The piece to download next: the first one missing when `sequential`, and one at random
    /// otherwise; `None` when no wanted piece is missing.
    fn next_piece(&mut self, sequential: bool) -> Option<usize> {
        let count = self.have.len();
        if sequential {
            while self.cursor < count {
                if self.is_open(self.cursor) {
                    return Some(self.cursor);
                }
                self.cursor += 1;
            }
            return None;
        }
        let start = pick_below(&mut self.random, count);
        (0..count)
            .map(|step| (start + step) % count)
            .find(|&piece| self.is_open(piece))
    }

    /// Notes that piece `piece` is here.
    fn mark(&mut self, piece: usize) {
        if let Some(have) = self.have.get_mut(piece) {
            *have = true;
        }
    }

    /// Downloads `bytes` more: whole pieces, picked as `sequential` says, up to the last percent
    /// of the pieces wanted (and at least the last one), which stays missing so that the download
    /// keeps going in the demo. The percent is of the wanted pieces, not of all of them: a small
    /// choice of files must still download.
    fn advance(&mut self, bytes: u64, sequential: bool) {
        self.credit = self.credit.saturating_add(bytes);
        let wanted = self.needed.iter().filter(|&&needed| needed).count();
        let floor = (wanted / 100).max(1);
        let mut open = (0..self.have.len())
            .filter(|&piece| self.is_open(piece))
            .count();
        while self.credit >= self.layout.piece_size && open > floor {
            let Some(piece) = self.next_piece(sequential) else {
                break;
            };
            self.mark(piece);
            self.credit -= self.layout.piece_size;
            open -= 1;
        }
        if open <= floor {
            self.credit = 0;
        }
    }

    /// Works out which pieces hold data of a wanted file, after the wanted files changed.
    fn refresh_needed(&mut self) {
        self.needed.fill(false);
        for (file, &wanted) in self.layout.files.iter().zip(&self.wanted) {
            for piece in file.first_piece..file.end_piece {
                if let Some(needed) = self.needed.get_mut(piece).filter(|_| wanted) {
                    *needed = true;
                }
            }
        }
        self.cursor = 0;
    }

    /// The bytes of each file that the pieces present hold.
    fn completed(&self) -> Vec<u64> {
        self.layout
            .files
            .iter()
            .map(|file| {
                (file.first_piece..file.end_piece)
                    .filter(|&piece| self.have.get(piece) == Some(&true))
                    .map(|piece| self.layout.overlap(piece, file))
                    .sum()
            })
            .collect()
    }

    /// The KiB of the wanted files that are still missing.
    pub(super) fn left_kib(&self) -> u32 {
        let left: u64 = self
            .layout
            .files
            .iter()
            .zip(&self.wanted)
            .zip(self.completed())
            .filter(|&((_, &wanted), _)| wanted)
            .map(|((file, _), completed)| file.length - completed)
            .sum();
        u32::try_from(left / KIB).unwrap_or(u32::MAX)
    }

    /// The KiB of the wanted files.
    fn wanted_kib(&self) -> u32 {
        let wanted: u64 = self
            .layout
            .files
            .iter()
            .zip(&self.wanted)
            .filter(|&(_, &wanted)| wanted)
            .map(|(file, _)| file.length)
            .sum();
        u32::try_from(wanted / KIB).unwrap_or(u32::MAX)
    }

    /// The pieces present as the wire has them: a bit each, the first piece the highest bit of the
    /// first byte, in base64.
    fn pieces_base64(&self) -> String {
        let mut bytes = vec![0_u8; self.have.len().div_ceil(8)];
        for (piece, _) in self.have.iter().enumerate().filter(|&(_, &have)| have) {
            if let Some(byte) = bytes.get_mut(piece / 8) {
                *byte |= 0x80_u8 >> (piece % 8);
            }
        }
        STANDARD.encode(bytes)
    }

    /// Adds the fields of `fields` that this state answers to `record`.
    fn add_fields(&self, fields: &Fields, record: &mut Map<String, Value>) {
        let mut put = |field: &str, value: &dyn Fn() -> Value| {
            if fields.asks(field) {
                record.insert(field.to_owned(), value());
            }
        };
        let completed = if fields.asks_any(&["files", "file_stats", "bytes_completed"]) {
            self.completed()
        } else {
            Vec::new()
        };
        put("piece_count", &|| json!(self.layout.piece_count()));
        put("piece_size", &|| json!(self.layout.piece_size));
        put("bytes_completed", &|| json!(completed));
        put("priorities", &|| json!(self.priority));
        put("wanted", &|| json!(self.wanted));
        put("pieces", &|| json!(self.pieces_base64()));
        put("files", &|| {
            self.layout
                .files
                .iter()
                .zip(&completed)
                .map(|(file, completed)| {
                    json!({
                        "begin_piece": file.first_piece,
                        "bytes_completed": completed,
                        "end_piece": file.end_piece,
                        "length": file.length,
                        "name": file.name,
                    })
                })
                .collect()
        });
        put("file_stats", &|| {
            completed
                .iter()
                .zip(&self.wanted)
                .zip(&self.priority)
                .map(|((completed, wanted), priority)| {
                    json!({
                        "bytes_completed": completed,
                        "priority": priority,
                        "wanted": wanted,
                    })
                })
                .collect()
        });
    }
}

impl Torrent {
    /// The torrent's files and pieces, made on first use from how much is left.
    ///
    /// Making them settles what is left to what the pieces hold, which differs from what it was
    /// by less than a piece.
    pub(super) fn files(&mut self) -> &mut FileState {
        let Self {
            files,
            plan,
            name,
            total_kib,
            left_kib,
            sequential,
            id,
            ..
        } = self;
        files.get_or_insert_with(|| {
            let layout = Layout::of(*plan, name, *total_kib, *id);
            let state = FileState::new(layout, *left_kib, *sequential, *id);
            *left_kib = state.left_kib();
            state
        })
    }

    /// Whether `fields` asks for something the files and pieces answer.
    pub(super) fn needs_files(fields: &Fields) -> bool {
        fields.asks_any(&FILE_FIELDS)
    }

    /// The KiB to download: those of the wanted files.
    pub(super) fn size_when_done_kib(&self) -> u32 {
        self.files
            .as_ref()
            .map_or(self.total_kib, FileState::wanted_kib)
    }

    /// Downloads `kib` more, in pieces if the torrent has them.
    pub(super) fn download(&mut self, kib: u32) {
        if let Some(files) = &mut self.files {
            files.advance(u64::from(kib) * KIB, self.sequential);
            self.left_kib = files.left_kib();
            return;
        }
        // A download comes close to the end and stays there, so that it keeps moving.
        self.left_kib = self.left_kib.saturating_sub(kib).max(self.total_kib / 100);
    }

    /// Applies the file lists of a `torrent_set`, which have passed [`FileOps::check`] for this
    /// torrent, and then the sequential download.
    pub(super) fn set(&mut self, lists: &FileOps, sequential: Option<bool>) {
        if !lists.is_empty() {
            let state = self.files();
            lists.apply(state);
            self.left_kib = state.left_kib();
            self.settle();
        }
        if let Some(sequential) = sequential {
            self.sequential = sequential;
        }
        self.touched |= !lists.is_empty() || sequential.is_some();
    }

    /// Turns a download that has nothing wanted left into a seed, and a seed that has something
    /// wanted missing into a download, as a daemon does once the wanted files change.
    fn settle(&mut self) {
        match self.kind {
            Kind::Downloading if self.left_kib == 0 => {
                self.kind = Kind::Seeding;
                self.rate_down = 0;
                self.peers_sending_to_us = 0;
            }
            Kind::Seeding | Kind::Uploading if self.left_kib > 0 => self.kind = Kind::Downloading,
            _ => {}
        }
    }

    /// Adds to `record` the fields of `fields` that the files answer: the file count, and the
    /// lists and the pieces, which come from the files [`Torrent::files`] has made.
    pub(super) fn add_file_fields(&self, fields: &Fields, record: &mut Map<String, Value>) {
        if let Some(files) = &self.files {
            files.add_fields(fields, record);
        }
        if fields.asks("file_count") {
            // Counting needs the layout only, so a read of every torrent's file count does not
            // make the pieces of each.
            let count = self.files.as_ref().map_or_else(
                || {
                    Layout::of(self.plan, &self.name, self.total_kib, self.id)
                        .files
                        .len()
                },
                FileState::file_count,
            );
            record.insert("file_count".to_owned(), json!(count));
        }
    }
}
