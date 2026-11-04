//! [`CountryDb`]: a database file, checked once when it is opened and then read one node at a
//! time, at its position.

use std::fmt;
use std::io::{self, SeekFrom};
use std::net::IpAddr;

use super::{
    COUNTRY_BEGIN, CountryCode, DELIMITER, Edition, GeoIpError, MARKER, Source, country_codes,
};

/// The bytes of a node: two records of 3 bytes.
const NODE_BYTES: u64 = 6;

/// The smallest file: one node, the marker and the type.
const MIN_FILE: u64 = 10;

/// The marker starts in the last 22 bytes of a file, as libGeoIP looks for it there: at most 19
/// bytes follow it.
const MARKER_SEARCH: usize = 22;

/// How far before the marker the text that names the build is looked for; the package's is 25
/// bytes long.
const INFO_SEARCH: usize = 128;

/// A country database file of one [`Edition`].
///
/// The file is never read whole: [`CountryDb::open`] reads its last bytes and two walks through
/// the tree, and [`CountryDb::lookup`] reads one node of 6 bytes per bit of the address it walks,
/// at most 32 or 128. No node is kept between lookups.
pub struct CountryDb {
    source: Box<dyn Source>,
    edition: Edition,
    /// The nodes of the tree; a record that names another node is a corrupt file.
    nodes: u64,
    info: String,
    /// What the caller calls the file, such as its path, for the warning about a corrupt file.
    name: Option<String>,
    broken: bool,
}

/// Why a walk through the tree did not end in a leaf.
#[derive(Debug)]
enum WalkError {
    /// A node could not be read.
    Read(io::Error),
    /// A record names a node past the end of the tree.
    Outside(u32),
    /// The walk was still going after the last bit of the address.
    Endless,
}

impl fmt::Display for WalkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "a node could not be read: {error}"),
            Self::Outside(node) => write!(f, "a record names node {node}, past the tree"),
            Self::Endless => f.write_str("a walk does not end in a leaf"),
        }
    }
}

impl CountryDb {
    /// Checks that `source` is a country database of `edition` and opens it.
    ///
    /// The checks are those of libGeoIP and more, so that a cut or mixed-up file is refused
    /// before it gives wrong countries: the marker is the last `FF FF FF` that starts in the last
    /// 22 bytes; the type after it is that of `edition`; after the type the file ends, or holds
    /// the node count in 3 bytes and then ends; the count is not zero and its nodes fit before
    /// the marker; and the walks for the lowest and for the highest address end in a leaf inside
    /// the tree. The text that names the build is the printable ASCII between the last
    /// `00 00 00` before the marker (and after the nodes, when their count is known) and the
    /// marker; it is empty when there is none.
    ///
    /// # Errors
    ///
    /// - [`GeoIpError::TooShort`] for fewer than 10 bytes;
    /// - [`GeoIpError::NoMarker`] when no `FF FF FF` starts in the last 22 bytes;
    /// - [`GeoIpError::WrongEdition`] when the type is not that of `edition`;
    /// - [`GeoIpError::CutTrailer`] when the file does not end 1 or 4 bytes after the marker;
    /// - [`GeoIpError::NodeCount`] when the node count is zero or too large;
    /// - [`GeoIpError::ProbeFailed`] when one of the two walks does not end in a leaf;
    /// - [`GeoIpError::Read`] when `source` cannot be read.
    pub fn open(mut source: Box<dyn Source>, edition: Edition) -> Result<Self, GeoIpError> {
        let size = source.seek(SeekFrom::End(0)).map_err(GeoIpError::Read)?;
        if size < MIN_FILE {
            return Err(GeoIpError::TooShort { size });
        }
        let tail_len = usize::try_from(size).map_or(MARKER_SEARCH + INFO_SEARCH, |size| {
            size.min(MARKER_SEARCH + INFO_SEARCH)
        });
        let mut tail = vec![0; tail_len];
        // The tail is at most 150 bytes, so its length fits in a u64.
        let tail_start = size - u64::try_from(tail_len).unwrap_or(size);
        source
            .seek(SeekFrom::Start(tail_start))
            .and_then(|_| source.read_exact(&mut tail))
            .map_err(GeoIpError::Read)?;
        let trailer = Trailer::parse(&tail, tail_start, edition)?;
        let mut db = Self {
            source,
            edition,
            nodes: trailer.nodes,
            info: trailer.info,
            name: None,
            broken: false,
        };
        for probe in [0, u128::MAX] {
            db.walk(probe).map_err(|error| match error {
                WalkError::Read(error) => GeoIpError::Read(error),
                WalkError::Outside(_) | WalkError::Endless => GeoIpError::ProbeFailed,
            })?;
        }
        Ok(db)
    }

    /// The database, which logs `name`, such as the path it was opened from, in the warning it
    /// writes when a lookup finds the file corrupt. x4-core opens no files, so only the caller
    /// knows where the bytes come from.
    #[must_use]
    pub fn named(self, name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..self
        }
    }

    /// What the caller named the file ([`CountryDb::named`]), if it did.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The text that names the build of the file, such as `"GeoLite2 Country 20260203"`.
    #[must_use]
    pub fn info(&self) -> &str {
        &self.info
    }

    /// The country of `ip`, or `None` when the file has none for it.
    ///
    /// An [`Edition::V4`] database takes IPv4 addresses and IPv4-mapped IPv6 addresses; an
    /// [`Edition::V6`] database takes IPv6 addresses. Any other address gives `None` without a
    /// read. "Not found", the regions AP and EU and the entries A1, A2 and O1 are `None` too.
    ///
    /// When a node cannot be read, a record names a node past the tree, or the walk does not end
    /// after the last bit, the file is corrupt: the lookup gives `None`, logs a warning once, with
    /// the [name](Self::named) of the file if it has one, and from then on the database is
    /// [broken](Self::is_broken) and gives `None` without reading.
    pub fn lookup(&mut self, ip: IpAddr) -> Option<CountryCode> {
        if self.broken {
            return None;
        }
        let address = match (self.edition, ip) {
            (Edition::V4, IpAddr::V4(v4)) => u128::from(u32::from(v4)),
            (Edition::V4, IpAddr::V6(v6)) => u128::from(u32::from(v6.to_ipv4_mapped()?)),
            (Edition::V6, IpAddr::V6(v6)) => u128::from(v6),
            (Edition::V6, IpAddr::V4(_)) => return None,
        };
        match self.walk(address) {
            Ok(index) => country_codes::country(index),
            Err(error) => {
                self.broken = true;
                tracing::warn!(
                    file = self.name.as_deref(),
                    edition = %self.edition,
                    info = %self.info,
                    %error,
                    "the GeoIP database is corrupt; it gives no more countries"
                );
                None
            }
        }
    }

    /// Whether a lookup found the file corrupt; a broken database gives no countries.
    #[must_use]
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// Walks the tree for the low [`Edition::bits`] bits of `address`, the most significant
    /// first, to the index of the leaf it ends in.
    fn walk(&mut self, address: u128) -> Result<u32, WalkError> {
        let mut node = 0;
        for depth in (0..self.edition.bits()).rev() {
            let record = self.record(node, (address >> depth) & 1 == 1)?;
            if record >= COUNTRY_BEGIN {
                return Ok(record - COUNTRY_BEGIN);
            }
            node = record;
        }
        Err(WalkError::Endless)
    }

    /// The record of `node` for a 0 bit or, if `one`, for a 1 bit.
    fn record(&mut self, node: u32, one: bool) -> Result<u32, WalkError> {
        if u64::from(node) >= self.nodes {
            return Err(WalkError::Outside(node));
        }
        let mut bytes = [0; 6];
        let source = &mut self.source;
        source
            .seek(SeekFrom::Start(u64::from(node) * NODE_BYTES))
            .and_then(|_| source.read_exact(&mut bytes))
            .map_err(WalkError::Read)?;
        let [
            zero_low,
            zero_middle,
            zero_high,
            one_low,
            one_middle,
            one_high,
        ] = bytes;
        Ok(if one {
            u32::from_le_bytes([one_low, one_middle, one_high, 0])
        } else {
            u32::from_le_bytes([zero_low, zero_middle, zero_high, 0])
        })
    }
}

impl fmt::Debug for CountryDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("CountryDb");
        if let Some(name) = &self.name {
            debug.field("name", name);
        }
        debug
            .field("edition", &self.edition)
            .field("nodes", &self.nodes)
            .field("info", &self.info)
            .field("broken", &self.broken)
            .finish_non_exhaustive()
    }
}

/// What the end of a file says about it.
struct Trailer {
    /// The nodes of the tree: the node count, or, without one, as many as fit before the marker.
    nodes: u64,
    info: String,
}

impl Trailer {
    /// Reads the end of a file of `edition`: `tail`, its last bytes, starts at `tail_start`.
    fn parse(tail: &[u8], tail_start: u64, edition: Edition) -> Result<Self, GeoIpError> {
        let search_start = tail.len().saturating_sub(MARKER_SEARCH);
        let marker = tail
            .get(search_start..)
            .and_then(|end| end.windows(MARKER.len()).rposition(|bytes| bytes == MARKER))
            .map(|position| search_start + position)
            .ok_or(GeoIpError::NoMarker)?;
        let after_marker = tail.get(marker + MARKER.len()..).unwrap_or_default();
        let Some((&found, count)) = after_marker.split_first() else {
            return Err(GeoIpError::CutTrailer { after_marker: 0 });
        };
        if found != edition.type_byte() {
            return Err(GeoIpError::WrongEdition { edition, found });
        }
        // Positions in the tail fit in a u64: the tail is at most 150 bytes.
        let room = tail_start + u64::try_from(marker).unwrap_or(0);
        let (nodes, counted) = match *count {
            [] => (room / NODE_BYTES, false),
            [low, middle, high] => {
                let count = u32::from_le_bytes([low, middle, high, 0]);
                if count == 0 || u64::from(count) * NODE_BYTES > room {
                    return Err(GeoIpError::NodeCount { count, room });
                }
                (u64::from(count), true)
            }
            _ => {
                return Err(GeoIpError::CutTrailer {
                    after_marker: after_marker.len(),
                });
            }
        };
        // With a node count the delimiter comes after the nodes; without, anywhere before the
        // marker.
        let info_start = if counted {
            usize::try_from((nodes * NODE_BYTES).saturating_sub(tail_start)).unwrap_or(marker)
        } else {
            0
        };
        let before_marker = tail.get(info_start..marker).unwrap_or_default();
        let info = before_marker
            .windows(DELIMITER.len())
            .rposition(|bytes| bytes == DELIMITER)
            .and_then(|position| before_marker.get(position + DELIMITER.len()..))
            .map(printable)
            .unwrap_or_default();
        Ok(Self { nodes, info })
    }
}

/// `bytes` as text, with `?` for each byte that is not printable ASCII, so that the text can go
/// into a log line as it is.
fn printable(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| {
            if byte == b' ' || byte.is_ascii_graphic() {
                char::from(byte)
            } else {
                '?'
            }
        })
        .collect()
}
