//! Reading `.torrent` files, strictly, the way Transmission 4.1 reads them (see
//! [`parse_torrent`]).
//!
//! `bencode` reads the format within libtransmission's limits and without recursion, `torrent`
//! applies libtransmission's rules for the info dictionary (T/torrent-metainfo.cc), and `names`
//! its rules for the names it shows (T/torrent-files.cc, T/utils.cc).

mod bencode;
mod names;
mod torrent;

use thiserror::Error;
use x4_rpc::HashString;

/// The largest `.torrent` file that is read, in bytes: 16 MiB.
///
/// Transmission sets no limit on the file, only on a single string in it (128 MiB), and keeps the
/// piece hashes of every torrent in memory. Real files are far smaller: the piece hashes of
/// 100 GiB in 4 MiB pieces take 512 KB, and 10 000 files about 1 MB.
pub const MAX_TORRENT_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// What a `.torrent` file holds, as the daemon will read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTorrent {
    /// The torrent's name as the daemon shows it: the last of `name` and `name.utf-8`, cut at a
    /// NUL, with bytes that are not UTF-8 replaced by U+FFFD. Empty when the file has none.
    pub name: String,
    /// The info hash the daemon keys the torrent by: the SHA-1 of the raw bytes of `info`.
    pub hash: HashString,
    /// The size of all files, in bytes; `None` exactly when `files` is.
    pub total_size: Option<u64>,
    /// The files, at the daemon's indices. `None` when the daemon could read them otherwise: the
    /// torrent can then be added only with all its files.
    pub files: Option<Vec<MetaFile>>,
}

/// One file of a [`ParsedTorrent`]; its index is its position in [`ParsedTorrent::files`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaFile {
    /// The path inside the torrent, `/`-separated, under the torrent's folder if it has one,
    /// sanitized as the daemon names the file: `.` and `..` become `_.` and `_..`, and empty
    /// parts vanish.
    pub path: String,
    /// The size in bytes.
    pub length: u64,
}

/// Why a `.torrent` file cannot be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MetainfoError {
    /// The file is larger than [`MAX_TORRENT_FILE_BYTES`].
    #[error(
        "the torrent file has {size} bytes, more than the {max} that are read",
        max = MAX_TORRENT_FILE_BYTES
    )]
    TooLarge {
        /// The size of the file in bytes.
        size: u64,
    },
    /// The file is not bencoded, or it ends too early.
    #[error("the torrent file is not bencoded, at byte {offset}")]
    NotBencode {
        /// The offset in the file of the value that is not bencode, or of the end of the file.
        offset: usize,
    },
    /// Lists and dictionaries are nested more than 31 levels deep.
    #[error(
        "the torrent file nests lists and dictionaries more than {max} levels deep, at byte {offset}",
        max = bencode::MAX_DEPTH
    )]
    TooDeep {
        /// The offset in the file of the list or dictionary that is one level too deep.
        offset: usize,
    },
    /// The file has no `info` dictionary at its top level.
    #[error("the torrent file has no info dictionary")]
    NoInfo,
    /// The torrent has only v2 metadata (BEP 52), which Transmission 4.1 cannot add.
    #[error("the torrent is BitTorrent v2 only, which Transmission 4.1 cannot add")]
    V2Only,
    /// The daemon would refuse the torrent; the text says why, for the log.
    #[error("the torrent is invalid: {0}")]
    Invalid(&'static str),
}

impl From<bencode::Error> for MetainfoError {
    fn from(error: bencode::Error) -> Self {
        match error {
            bencode::Error::Syntax { offset } => Self::NotBencode { offset },
            bencode::Error::TooDeep { offset } => Self::TooDeep { offset },
        }
    }
}

/// Reads the content of a `.torrent` file, strictly, the way Transmission 4.1 reads it.
///
/// A file's position in [`ParsedTorrent::files`] is the index the daemon gives it, which
/// `files_unwanted` names, so the list keeps the order of `info.files` in the file: nothing is
/// sorted, merged or left out, padding files keep their place, and a hybrid v1+v2 torrent gives
/// its v1 list, which is the one the daemon reads.
///
/// The reader is stricter than the daemon. Where the daemon could read other files, or the same
/// ones in another order, the torrent is read with `files: None` and can be added only with all
/// its files: keys out of order or twice in any dictionary, two `info` dictionaries, an entry of
/// `info.files` that is not a dictionary of scalars and lists of scalars (the daemon would skip
/// it, count a file for a dictionary inside it, or stop at a list keyed `files` in it), a
/// negative or non-integer length, or two paths that are equal only once made valid UTF-8.
/// Where the daemon is certain to refuse the torrent, so is the reader.
///
/// The input is untrusted: the reader never recurses and never panics. It reads at most
/// [`MAX_TORRENT_FILE_BYTES`], with lists and dictionaries nested at most 31 levels deep, strings
/// shorter than 128 MiB and integers that fit in 64 bits, as libtransmission does. The name and
/// the paths of the files may take at most 64 MiB together, which is checked before any path is
/// built, so a hostile file is refused before it costs memory for each of its files.
///
/// # Errors
///
/// - [`MetainfoError::TooLarge`] for more than [`MAX_TORRENT_FILE_BYTES`], before anything is
///   read;
/// - [`MetainfoError::NotBencode`] and [`MetainfoError::TooDeep`] when the bytes are not bencode
///   within libtransmission's limits;
/// - [`MetainfoError::NoInfo`] when the top level is not a dictionary with an `info` dictionary;
/// - [`MetainfoError::V2Only`] for a torrent without v1 piece hashes but with v2 metadata;
/// - [`MetainfoError::Invalid`] for a torrent the daemon refuses: without files, with a file
///   whose path is empty, with two files of the same path, or with piece hashes that do not match
///   the size; and for one whose name and paths would take more than 64 MiB.
pub fn parse_torrent(bytes: &[u8]) -> Result<ParsedTorrent, MetainfoError> {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > MAX_TORRENT_FILE_BYTES {
        return Err(MetainfoError::TooLarge { size });
    }
    let document = bencode::parse(bytes)?;
    torrent::read_torrent(&document)
}
