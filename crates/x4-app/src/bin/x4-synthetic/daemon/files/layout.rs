//! What the files of a made-up torrent are: their names and sizes, laid one after the other, and
//! the pieces that cut the data. All of it follows from a [`Plan`] and the torrent's id.

use super::super::{KIB, Lcg};

/// The random stream that picks a torrent's plan.
pub(in super::super) const PLAN_STREAM: u8 = 1;
/// The random stream that weighs a torrent's files against each other.
const WEIGHTS_STREAM: u8 = 2;
/// The smallest piece, in bytes.
const MIN_PIECE_SIZE: u64 = 64 * KIB;
/// The most pieces a torrent has: the biggest torrents get pieces of 4 MiB.
const MAX_PIECES: u64 = 16_384;
/// The share of a torrent's size, in percent, that all files but the last one are cut from. The
/// last file gets the rest, so there is room for padding.
const SHARE_BEFORE_LAST: u64 = 88;
/// How many files the archive has: as many as the Files tab shows, `x4_core::MAX_CHOOSABLE_FILES`.
const ARCHIVE_FILES: u32 = 50_000;
/// How many scans each folder of the archive holds.
const ARCHIVE_SCANS_PER_FOLDER: u32 = 100;
/// How many folders each box of the archive holds.
const ARCHIVE_FOLDERS_PER_BOX: u32 = 20;

/// A number of bytes or pieces as an index.
pub(super) fn to_index(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// An index or a count as a number of bytes or pieces.
pub(super) fn to_wide(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// How a torrent's files are arranged.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shape {
    /// One file, named like the torrent.
    Single,
    /// Episodes of a series in a folder, and a sample; the number is the count of those files.
    Folder(u32),
    /// A series in sub-folders (two seasons, extras, a sample, a text file): 40 files.
    Series,
    /// An album of 2000 tracks, in discs and parts: folders three deep.
    Album,
    /// An archive of [`ARCHIVE_FILES`] scans, in boxes and folders: folders three deep.
    Archive,
    /// The files of a torrent a client added, as its metainfo lists them: a path and a size in
    /// bytes each, in the metainfo's order.
    Listed(Vec<(String, u64)>),
    /// A torrent added by a magnet link, whose metadata has not arrived: no files yet.
    Magnet,
}

/// How a padded torrent names its padding files (BEP 47).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Padding {
    /// In a `.pad` folder, numbered: `.pad/0`.
    Folder,
    /// Next to the files, as most clients write them: `_____padding_file_0`.
    Prefix,
}

/// What a torrent's files are, before sizes: decided by the seed, or by the metainfo of a torrent a
/// client added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct Plan {
    shape: Shape,
    /// Whether every file but the last is followed by a padding file that fills its last piece.
    padding: Option<Padding>,
}

impl Plan {
    /// A plan by the roll of `random`: about 70% have one file, 25% a folder of 3 to 12 files,
    /// and a few a series of 40 files. About one in five that has several files is padded.
    pub(in super::super) fn pick(random: &mut Lcg) -> Self {
        let shape = match random.below(1000) {
            700..950 => Shape::Folder(random.between(3, 13)),
            950..955 => Shape::Series,
            _ => Shape::Single,
        };
        let padding = match (&shape, random.below(5)) {
            (Shape::Single, _) | (_, 1..) => None,
            (_, 0) if random.below(2) == 0 => Some(Padding::Folder),
            (_, 0) => Some(Padding::Prefix),
        };
        Self { shape, padding }
    }

    /// The plan of the one torrent with 2000 files.
    pub(in super::super) fn album() -> Self {
        Self {
            shape: Shape::Album,
            padding: None,
        }
    }

    /// The plan of the one torrent with [`ARCHIVE_FILES`] files.
    pub(in super::super) fn archive() -> Self {
        Self {
            shape: Shape::Archive,
            padding: None,
        }
    }

    /// The plan of a torrent whose metainfo lists `files`, each a path and a size in bytes, and
    /// how many KiB they take together.
    ///
    /// The made-up torrents count in whole KiB, so every size is rounded up to one (0 stays 0);
    /// the files then add up to exactly the KiB returned. `None` when they are more than a torrent
    /// of this daemon can hold, which counts in `u32`.
    pub(in super::super) fn listed(files: Vec<(String, u64)>) -> Option<(Self, u32)> {
        let mut total_kib = 0_u32;
        let mut rounded = Vec::with_capacity(files.len());
        for (name, length) in files {
            let length = length.checked_next_multiple_of(KIB)?;
            total_kib = total_kib.checked_add(u32::try_from(length / KIB).ok()?)?;
            rounded.push((name, length));
        }
        let shape = Shape::Listed(rounded);
        Some((
            Self {
                shape,
                padding: None,
            },
            total_kib,
        ))
    }

    /// The plan of a torrent added by a magnet link: no metadata, so no files.
    pub(in super::super) fn magnet() -> Self {
        Self {
            shape: Shape::Magnet,
            padding: None,
        }
    }

    /// Whether the torrent knows its files: not a magnet link still looking for its metadata.
    pub(in super::super) fn has_metadata(&self) -> bool {
        !matches!(self.shape, Shape::Magnet)
    }

    /// The files without padding, with the name inside the folder `top` and a weight for the size
    /// each takes: the bigger the weight, the bigger the file.
    fn content(&self, top: &str, random: &mut Lcg) -> Vec<(String, u64)> {
        let mut weigh = |base: u32| u64::from(base * random.between(1, 5));
        match self.shape {
            Shape::Single => vec![(top.to_owned(), 1)],
            Shape::Listed(_) | Shape::Magnet => Vec::new(),
            Shape::Folder(count) => {
                let mut files: Vec<_> = (1..count)
                    .map(|number| (format!("{top}/E{number:02}.mkv"), weigh(8)))
                    .collect();
                files.push((format!("{top}/Sample/sample.mkv"), weigh(1)));
                files
            }
            Shape::Series => {
                let mut files = Vec::new();
                for season in 1..=2 {
                    for number in 1..=18 {
                        let name = format!("{top}/Season {season}/E{number:02}.mkv");
                        files.push((name, weigh(8)));
                    }
                }
                for number in 1..=2 {
                    files.push((format!("{top}/Extras/Interview {number}.mkv"), weigh(3)));
                }
                files.push((format!("{top}/Sample/sample.mkv"), weigh(1)));
                // A text file has no size to speak of; it gets the smallest one.
                files.push((format!("{top}/info.nfo"), 0));
                files
            }
            Shape::Album => (0..2000_u32)
                .map(|track| {
                    let (disc, part) = (track / 100 + 1, track % 100 / 20 + 1);
                    let name = format!(
                        "{top}/Disc {disc:02}/Part {part}/Track {:03}.flac",
                        track + 1
                    );
                    (name, weigh(1))
                })
                .collect(),
            Shape::Archive => (0..ARCHIVE_FILES)
                .map(|scan| {
                    let folder = scan / ARCHIVE_SCANS_PER_FOLDER;
                    let (number, within) = (
                        folder / ARCHIVE_FOLDERS_PER_BOX + 1,
                        folder % ARCHIVE_FOLDERS_PER_BOX + 1,
                    );
                    let name = format!(
                        "{top}/Box {number:02}/Folder {within:02}/Scan {:05}.tif",
                        scan + 1
                    );
                    (name, weigh(1))
                })
                .collect(),
        }
    }
}

/// One file of a torrent.
#[derive(Debug, Clone)]
pub(super) struct FileSpec {
    /// The path inside the torrent, starting with its folder.
    pub(super) name: String,
    pub(super) length: u64,
    /// Where the file starts in the torrent's data.
    pub(super) offset: u64,
    /// The first piece with data of the file.
    pub(super) first_piece: usize,
    /// One past the last piece with data of the file.
    pub(super) end_piece: usize,
}

/// A torrent's files, laid one after the other, and the pieces that cut the data.
#[derive(Debug, Clone)]
pub(super) struct Layout {
    pub(super) files: Vec<FileSpec>,
    pub(super) piece_size: u64,
    /// The size of all files, in bytes.
    pub(super) total: u64,
}

impl Layout {
    /// The layout of torrent `id`, called `top`, of `total_kib` KiB, as `plan` says.
    ///
    /// The files add up to exactly `total_kib`. Their sizes are cut from it by weight, and the
    /// last file takes what remains, padding included. The files of a metainfo are laid as it
    /// lists them, and a magnet link has none.
    pub(super) fn of(plan: &Plan, top: &str, total_kib: u32, id: u32) -> Self {
        match &plan.shape {
            Shape::Listed(files) => return Self::listed(files),
            Shape::Magnet => return Self::listed(&[]),
            _ => {}
        }
        let total = u64::from(total_kib) * KIB;
        let piece_size = Self::piece_size(total);
        let content = plan.content(top, &mut Lcg::stream(id, WEIGHTS_STREAM));
        let weights: u64 = content.iter().map(|(_, weight)| weight).sum::<u64>().max(1);
        let budget = u64::from(total_kib) * SHARE_BEFORE_LAST / 100;
        let last = content.len().saturating_sub(1);
        let mut layout = Self {
            files: Vec::with_capacity(content.len()),
            piece_size,
            total,
        };
        let mut padding = 0_u32;
        for (position, (name, weight)) in content.into_iter().enumerate() {
            let at_kib = layout.end() / KIB;
            let kib = if position == last {
                u64::from(total_kib).saturating_sub(at_kib).max(1)
            } else {
                (budget * weight / weights).max(1)
            };
            layout.push(name, kib * KIB);
            let (Some(style), true) = (plan.padding, position != last) else {
                continue;
            };
            // Fills the piece the file ends in, so that the next file starts on a boundary.
            let fill = (piece_size - layout.end() % piece_size) % piece_size;
            if fill > 0 {
                let name = match style {
                    Padding::Folder => format!("{top}/.pad/{padding}"),
                    Padding::Prefix => format!("{top}/_____padding_file_{padding}"),
                };
                padding += 1;
                layout.push(name, fill);
            }
        }
        layout
    }

    /// The layout of the files a metainfo lists, one after the other, in its order.
    fn listed(files: &[(String, u64)]) -> Self {
        let total = files.iter().map(|&(_, length)| length).sum();
        let mut layout = Self {
            files: Vec::with_capacity(files.len()),
            piece_size: Self::piece_size(total),
            total,
        };
        for (name, length) in files {
            layout.push(name.clone(), *length);
        }
        layout
    }

    /// The piece size for `total` bytes: a power of two, with as many pieces as fit in
    /// `MAX_PIECES` at most.
    fn piece_size(total: u64) -> u64 {
        total
            .div_ceil(MAX_PIECES)
            .next_power_of_two()
            .max(MIN_PIECE_SIZE)
    }

    /// Where the files so far end, in bytes.
    fn end(&self) -> u64 {
        self.files
            .last()
            .map_or(0, |file| file.offset + file.length)
    }

    /// Lays a file of `length` bytes after the last one.
    fn push(&mut self, name: String, length: u64) {
        let offset = self.end();
        self.files.push(FileSpec {
            name,
            length,
            offset,
            first_piece: to_index(offset / self.piece_size),
            end_piece: to_index((offset + length).div_ceil(self.piece_size)),
        });
    }

    /// How many pieces the data is cut into.
    pub(super) fn piece_count(&self) -> usize {
        to_index(self.total.div_ceil(self.piece_size))
    }

    /// How many bytes of the file `file` lie in piece `piece`.
    pub(super) fn overlap(&self, piece: usize, file: &FileSpec) -> u64 {
        let start = to_wide(piece) * self.piece_size;
        let end = (start + self.piece_size).min(self.total);
        let (from, to) = (start.max(file.offset), end.min(file.offset + file.length));
        to.saturating_sub(from)
    }
}
