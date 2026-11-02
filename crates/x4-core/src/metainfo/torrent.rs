//! What libtransmission 4.1 makes of a bencoded `.torrent` file (`MetainfoHandler`,
//! T/torrent-metainfo.cc:60-582), read strictly: the info hash, the name, and the files in the
//! order the daemon numbers them, or no files at all when the daemon could number them otherwise.
//!
//! The daemon reads the file as a stream and keeps no check on keys, so a file whose dictionaries
//! have keys out of order or twice, or two `info` dictionaries, can make it read other files than
//! a reader of the dictionaries would. Such a file, and a `files` entry the daemon would skip,
//! split or misread, gives no file list (RT §2.3, R1-R5). The daemon's own refusals (R6, R7, R10)
//! are errors, predicted only where they are certain.

use sha1::{Digest, Sha1};
use x4_rpc::HashString;

use super::bencode::{Dict, Document, List, Value};
use super::names::{
    Text, display_text, push_path_element, sanitized_subpath, sanitized_subpath_len,
};
use super::{MetaFile, MetainfoError, ParsedTorrent};

/// The most bytes that the text of a [`ParsedTorrent`] may take: its name and the paths of its
/// files. Each path starts with the name as its folder, so a long name and many files could
/// otherwise turn a small file into gigabytes of text, and bytes that are not UTF-8 triple in
/// size as U+FFFD. Real torrents stay far below it. It is checked before any path is built.
const MAX_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// The refusal when [`MAX_TEXT_BYTES`] would be passed.
const TOO_LONG: MetainfoError = MetainfoError::Invalid("the name and the file paths are too long");

/// The size of one SHA-1 piece hash in `pieces` (T/torrent-metainfo.cc:374-381).
const PIECE_HASH_BYTES: usize = 20;

/// Reads the torrent in a checked bencode document.
///
/// # Errors
///
/// [`MetainfoError::NoInfo`] without an `info` dictionary at the top, [`MetainfoError::V2Only`]
/// for a torrent without v1 piece hashes but with v2 metadata, and [`MetainfoError::Invalid`] for
/// one the daemon refuses. A fault in the bencode is reported as
/// [`MetainfoError::NotBencode`], though the document has been checked already.
pub(super) fn read_torrent(document: &Document<'_>) -> Result<ParsedTorrent, MetainfoError> {
    let Value::Dict(root) = document.root else {
        return Err(MetainfoError::NoInfo);
    };
    let top = TopLevel::read(root)?;
    let info_dict = top.info.ok_or(MetainfoError::NoInfo)?;
    let hash = info_hash(info_dict.raw())?;
    let info = InfoFacts::read(info_dict)?;
    let single_info = top.info_count == 1;
    // With one `info` the daemon's verdict on `pieces` does not depend on the order of the keys
    // (T/torrent-metainfo.cc:366-381, 484-488).
    let piece_count = if single_info {
        Some(info.pieces.verdict(info.v2 || info.file_tree)?)
    } else {
        None
    };
    let name = display_text(info.name.unwrap_or_default());
    let without_choice = |name: Text<'_>, reason: &'static str| {
        tracing::debug!(reason, "torrent file read without a choice of files");
        ParsedTorrent {
            name: name.text.into_owned(),
            hash: hash.clone(),
            total_size: None,
            files: None,
        }
    };
    let Some(piece_count) = piece_count else {
        return Ok(without_choice(name, "two info dictionaries"));
    };
    if document.odd_keys {
        return Ok(without_choice(
            name,
            "dictionary keys out of order or repeated",
        ));
    }

    // The keys are in order and none is repeated, so the daemon meets each one once, in the
    // order read here.
    let files = match file_list(&info, &name.text)? {
        FileList::Files(files) => files,
        FileList::Unsure(reason) => return Ok(without_choice(name, reason)),
    };
    let total_size = files
        .iter()
        .try_fold(0_u64, |total, file| total.checked_add(file.length))
        .ok_or(MetainfoError::Invalid(
            "the total size does not fit in 64 bits",
        ))?;
    // A top-level `piece length` comes after `info` and wins (T/torrent-metainfo.cc:232-235).
    check_pieces(
        top.piece_length.or(info.piece_length),
        total_size,
        piece_count,
    )?;
    Ok(ParsedTorrent {
        name: name.text.into_owned(),
        hash,
        total_size: Some(total_size),
        files: Some(files),
    })
}

/// The SHA-1 of the raw `info` bytes as they are in the file, not re-encoded
/// (T/torrent-metainfo.cc:510-519).
fn info_hash(info: &[u8]) -> Result<HashString, MetainfoError> {
    let mut hex = String::with_capacity(40);
    for byte in &Sha1::digest(info) {
        for nibble in [byte >> 4, byte & 0x0f] {
            hex.extend(char::from_digit(u32::from(nibble), 16));
        }
    }
    HashString::parse(&hex)
        .map_err(|_| MetainfoError::Invalid("the info hash is not 40 hex digits"))
}

/// What the top dictionary holds for the torrent.
struct TopLevel<'a> {
    /// The last `info` that is a dictionary: the daemon hashes each one as it ends, so the last
    /// one is the hash it keeps (T/torrent-metainfo.cc:110-114, 133-136).
    info: Option<Dict<'a>>,
    /// The `info` keys of any type.
    info_count: usize,
    /// A `piece length` outside `info`, which the daemon reads too.
    piece_length: Option<i64>,
}

impl<'a> TopLevel<'a> {
    fn read(root: Dict<'a>) -> Result<Self, MetainfoError> {
        let mut top = Self {
            info: None,
            info_count: 0,
            piece_length: None,
        };
        for entry in root.entries() {
            match entry? {
                (b"info", value) => {
                    top.info_count += 1;
                    if let Value::Dict(info) = value {
                        top.info = Some(info);
                    }
                }
                (b"piece length", Value::Int(length)) => top.piece_length = Some(length),
                _ => {}
            }
        }
        Ok(top)
    }
}

/// What the info dictionary holds, read in the order of its keys. Values of another type than
/// the daemon reads are skipped, as the daemon skips them.
struct InfoFacts<'a> {
    /// The first `files`: the daemon reads a later one only after an empty one
    /// (T/torrent-metainfo.cc:168-173).
    files: Option<Value<'a>>,
    /// The last `name` or `name.utf-8` (`set_name`, T/torrent-metainfo.cc:362-365).
    name: Option<&'a [u8]>,
    /// The single file's `length` (T/torrent-metainfo.cc:236-239).
    length: Option<i64>,
    piece_length: Option<i64>,
    pieces: Pieces,
    /// `meta version` is 2.
    v2: bool,
    /// There is a `file tree`, the v2 file list.
    file_tree: bool,
}

impl<'a> InfoFacts<'a> {
    fn read(info: Dict<'a>) -> Result<Self, MetainfoError> {
        let mut facts = Self {
            files: None,
            name: None,
            length: None,
            piece_length: None,
            pieces: Pieces::default(),
            v2: false,
            file_tree: false,
        };
        for entry in info.entries() {
            match entry? {
                (b"files", value) => {
                    facts.files.get_or_insert(value);
                }
                (b"file tree", _) => facts.file_tree = true,
                (b"length", Value::Int(length)) => facts.length = Some(length),
                (b"meta version", Value::Int(version)) => facts.v2 = version == 2,
                (b"name" | b"name.utf-8", Value::Bytes(name)) => facts.name = Some(name),
                (b"piece length", Value::Int(length)) => facts.piece_length = Some(length),
                (b"pieces", Value::Bytes(hashes)) => facts.pieces.read(hashes),
                _ => {}
            }
        }
        Ok(facts)
    }
}

/// The daemon's reading of the `pieces` strings, one after the other.
#[derive(Debug, Default)]
struct Pieces {
    /// The number of piece hashes read; 0 means no v1 metadata.
    count: usize,
    /// The daemon's refusal, at the first `pieces` it refuses.
    refusal: Option<&'static str>,
}

impl Pieces {
    /// Reads one `pieces` string (T/torrent-metainfo.cc:366-384).
    fn read(&mut self, hashes: &[u8]) {
        if self.refusal.is_some() {
            return;
        }
        if self.count > 0 {
            self.refusal = Some("the info dictionary has two piece lists");
        } else if !hashes.len().is_multiple_of(PIECE_HASH_BYTES) {
            self.refusal = Some("the piece hashes are not a whole number of SHA-1 hashes");
        } else {
            self.count = hashes.len() / PIECE_HASH_BYTES;
        }
    }

    /// The number of v1 piece hashes, or why the daemon refuses the torrent: a v1 torrent needs
    /// at least one (`has_v1_metadata`, T/torrent-metainfo.h:140-145), and one without them but
    /// with v2 metadata is v2-only, which 4.1 cannot add.
    fn verdict(&self, v2_metadata: bool) -> Result<usize, MetainfoError> {
        if let Some(refusal) = self.refusal {
            return Err(MetainfoError::Invalid(refusal));
        }
        match self.count {
            0 if v2_metadata => Err(MetainfoError::V2Only),
            0 => Err(MetainfoError::Invalid("the torrent has no piece hashes")),
            count => Ok(count),
        }
    }
}

/// The files of a torrent read without ambiguity, or why they cannot be.
enum FileList {
    Files(Vec<MetaFile>),
    Unsure(&'static str),
}

/// The files as the daemon lists them (RT §2.1), from an info dictionary whose keys are in order
/// and unique: the entries of a non-empty `files` list, in order, under the sanitized name, or
/// else the one file the name and `length` describe.
fn file_list(info: &InfoFacts<'_>, name: &str) -> Result<FileList, MetainfoError> {
    match info.files {
        Some(Value::List(entries)) if !entries.is_empty() => folder_files(entries, name),
        // An empty `files` adds nothing, so the torrent is a single file
        // (T/torrent-metainfo.cc:535-538).
        None | Some(Value::List(_)) => single_file(info, name),
        Some(_) => Ok(FileList::Unsure("files is not a list")),
    }
}

/// The single file of a torrent without `files` (R6, T/torrent-metainfo.cc:535-550): named by the
/// sanitized name, without a folder.
fn single_file(info: &InfoFacts<'_>, name: &str) -> Result<FileList, MetainfoError> {
    let length = info.length.unwrap_or(0);
    if length == 0 || name.is_empty() {
        return Err(MetainfoError::Invalid("the torrent has no files"));
    }
    let Ok(length) = u64::try_from(length) else {
        return Ok(FileList::Unsure("the file has a negative length"));
    };
    if name.len().saturating_add(sanitized_subpath_len(name)) > MAX_TEXT_BYTES {
        return Err(TOO_LONG);
    }
    Ok(FileList::Files(vec![MetaFile {
        path: sanitized_subpath(name),
        length,
    }]))
}

/// What one entry of `info.files` gives besides its path.
struct EntryFacts {
    length: u64,
    /// Whether a part of the path had bytes that are not UTF-8.
    repaired: bool,
}

/// The files of a `files` list (R2-R5, R10), under the folder `name`.
///
/// The list is walked twice. The first walk checks every entry and adds up the bytes of the
/// paths, building each one only in a buffer that every entry reuses, and stops at the entry
/// that would pass [`MAX_TEXT_BYTES`]: hostile input is refused before it costs memory for each
/// file. The second walk builds the paths, each at its exact size.
fn folder_files(entries: List<'_>, name: &str) -> Result<FileList, MetainfoError> {
    // The sanitized name is the folder of every file, unless it is empty
    // (T/torrent-metainfo.cc:503-508).
    let folder_bytes = match sanitized_subpath_len(name) {
        0 => 0,
        length => length.saturating_add(1),
    };
    let mut subpath = String::new();

    let mut count: usize = 0;
    let mut text_bytes = name.len();
    for item in entries.items() {
        if let Err(reason) = read_entry(item?, &mut subpath)? {
            return Ok(FileList::Unsure(reason));
        }
        count += 1;
        text_bytes = text_bytes.saturating_add(folder_bytes.saturating_add(subpath.len()));
        if text_bytes > MAX_TEXT_BYTES {
            return Err(TOO_LONG);
        }
    }

    let folder = sanitized_subpath(name);
    let mut files = Vec::with_capacity(count);
    let mut repaired = Vec::with_capacity(count);
    for item in entries.items() {
        let entry = match read_entry(item?, &mut subpath)? {
            Ok(entry) => entry,
            Err(reason) => return Ok(FileList::Unsure(reason)),
        };
        let mut path = String::with_capacity(folder_bytes + subpath.len());
        if !folder.is_empty() {
            path.push_str(&folder);
            path.push('/');
        }
        path.push_str(&subpath);
        files.push(MetaFile {
            path,
            length: entry.length,
        });
        repaired.push(entry.repaired);
    }
    if let Some(unsure) = check_unique(&files, &repaired)? {
        return Ok(FileList::Unsure(unsure));
    }
    Ok(FileList::Files(files))
}

/// Reads one item of `files` into `subpath`, its path before the torrent's folder, or says why
/// the daemon could read it otherwise.
///
/// An entry is a dictionary; the daemon skips any other item, which would shift every later index.
/// It holds `length`, an integer of at least 0 (0 when missing, T/torrent-metainfo.cc:470), `path`
/// and `path.utf-8`, lists of strings, and any other key with a scalar or a list of scalars (R4).
/// In order, `path.utf-8` comes after `path` and replaces it (T/torrent-metainfo.cc:174-178).
/// Anything else makes the daemon count the files differently: a dictionary inside the entry ends
/// like an entry and adds a file (:148-156), and a list keyed `files` ends the file list
/// (:187-191).
fn read_entry(
    item: Value<'_>,
    subpath: &mut String,
) -> Result<Result<EntryFacts, &'static str>, MetainfoError> {
    let Value::Dict(entry) = item else {
        return Ok(Err("a file entry is not a dictionary"));
    };
    subpath.clear();
    let mut facts = EntryFacts {
        length: 0,
        repaired: false,
    };
    for item in entry.entries() {
        match item? {
            (b"length", Value::Int(length)) => match u64::try_from(length) {
                Ok(length) => facts.length = length,
                Err(_) => return Ok(Err("a file has a negative length")),
            },
            (b"length", _) => return Ok(Err("a file length is not an integer")),
            (key @ (b"path" | b"path.utf-8"), value) => {
                let Value::List(parts) = value else {
                    return Ok(Err("a file path is not a list"));
                };
                if key == b"path.utf-8" {
                    subpath.clear();
                    facts.repaired = false;
                }
                for part in parts.items() {
                    let Value::Bytes(part) = part? else {
                        return Ok(Err("a file path holds something else than strings"));
                    };
                    let part = display_text(part);
                    facts.repaired |= part.repaired;
                    push_path_element(subpath, &part.text);
                }
            }
            (b"files", _) => return Ok(Err("a file entry holds a files key")),
            (_, value) => {
                if !is_flat(value)? {
                    return Ok(Err("a file entry holds a dictionary or a nested list"));
                }
            }
        }
    }
    // The daemon refuses a file whose path is empty (T/torrent-metainfo.cc:460-464).
    if subpath.is_empty() {
        return Err(MetainfoError::Invalid("a file has an empty path"));
    }
    Ok(Ok(facts))
}

/// Whether `value` is a scalar or a list of scalars.
fn is_flat(value: Value<'_>) -> Result<bool, MetainfoError> {
    match value {
        Value::Int(_) | Value::Bytes(_) => Ok(true),
        Value::Dict(_) => Ok(false),
        Value::List(items) => {
            for item in items.items() {
                if matches!(item?, Value::List(_) | Value::Dict(_)) {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

/// The daemon refuses two files with the same path (R10, T/torrent-metainfo.cc:490-501); every
/// path here starts with the same folder, so whole paths compare as the daemon's do. Paths that
/// are equal only after bytes that are not UTF-8 were replaced (`repaired`) may not be equal for
/// the daemon, whose repair differs, so they leave no choice instead of an error.
fn check_unique(
    files: &[MetaFile],
    repaired: &[bool],
) -> Result<Option<&'static str>, MetainfoError> {
    let key = |index: usize| {
        (
            files.get(index).map(|file| file.path.as_str()),
            repaired.get(index).copied(),
        )
    };
    // Within equal paths the ones read exactly come first, so two of them are neighbours. The
    // sort is in place, so it needs no memory beyond the indices.
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_unstable_by(|&a, &b| key(a).cmp(&key(b)));
    let mut unsure = None;
    for pair in order.windows(2) {
        let [first, second] = pair else { continue };
        let ((first_path, first_repaired), (second_path, second_repaired)) =
            (key(*first), key(*second));
        if first_path == second_path {
            if first_repaired == Some(false) && second_repaired == Some(false) {
                return Err(MetainfoError::Invalid("two files have the same path"));
            }
            unsure = Some("two file paths may be the same");
        }
    }
    Ok(unsure)
}

/// The daemon's checks of the pieces against the size (R7, T/torrent-metainfo.cc:552-575):
/// a piece length that is not 0, and one hash for each piece the files take.
///
/// The daemon keeps the piece length in 32 bits and the piece count in 32 bits, cutting off what
/// does not fit; a value that only fits that way is refused here instead.
fn check_pieces(
    piece_length: Option<i64>,
    total_size: u64,
    piece_count: usize,
) -> Result<(), MetainfoError> {
    let piece_length = piece_length
        .map(u32::try_from)
        .ok_or(MetainfoError::Invalid("the piece length is missing"))?
        .map_err(|_| MetainfoError::Invalid("the piece length does not fit in 32 bits"))?;
    if piece_length == 0 {
        return Err(MetainfoError::Invalid("the piece length is 0"));
    }
    let needed = total_size.div_ceil(u64::from(piece_length));
    if u64::try_from(piece_count).ok() != Some(needed) {
        return Err(MetainfoError::Invalid(
            "the number of piece hashes does not match the size",
        ));
    }
    Ok(())
}
