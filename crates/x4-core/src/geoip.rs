//! The country of an IP address, from the legacy country database that the `geoip-database`
//! package installs: `GeoIP.dat` for IPv4 and `GeoIPv6.dat` for IPv6.
//!
//! A database file is a binary tree of 6-byte nodes. A node holds two records of 3 bytes,
//! little-endian, the one for a 0 bit first. A lookup starts at node 0 and follows one record per
//! bit of the address, the most significant bit first, until a record is a leaf: a value of at
//! least `0xFFFF00`, whose distance from `0xFFFF00` is an index into the country table
//! (`country_codes`). Any smaller value is the number of the next node. After the tree the file
//! holds `00 00 00`, a text that names the build (`"GeoLite2 Country 20260203"`), the marker
//! `FF FF FF`, the type of the database (1 for countries by IPv4 address, 12 by IPv6 address)
//! and, in the files of the package, the number of nodes in 3 little-endian bytes. The layout
//! comes from the MIT-licensed converter `geolite2legacy`, which writes these files, and was
//! checked against the installed files.
//!
//! [`CountryDb::open`] checks a file before it is used, and [`CountryDb::lookup`] reads one node
//! per step at its position, so the files (3.7 MB and 9.5 MB) never take memory, and gives up on
//! a corrupt file instead of trusting it. [`GeoIp`] holds the two databases, either of which may
//! be missing, and sends each address to the one that knows it ([`CountryLookup`]).
//! [`build_country_file`] writes a database for tests and the synthetic demo, and
//! [`known_countries`] lists every country a lookup can give.

mod build;
mod country_codes;
mod reader;
#[cfg(test)]
mod tests;

use std::fmt;
use std::io::{self, Read, Seek};
use std::net::IpAddr;

use thiserror::Error;

pub use build::build_country_file;
pub use reader::CountryDb;

/// A record of at least this value is a leaf; what it has above this value is an index into the
/// country table. Node numbers stay below it.
const COUNTRY_BEGIN: u32 = 0xFF_FF00;

/// The bytes before the database type at the end of a file.
const MARKER: [u8; 3] = [0xFF; 3];

/// The bytes between the tree and the text that names the build.
const DELIMITER: [u8; 3] = [0; 3];

/// A country as its ISO 3166-1 alpha-2 code: two ASCII upper-case letters, such as "US".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CountryCode([u8; 2]);

impl CountryCode {
    /// The code made of `letters`, or `None` unless they are exactly two ASCII upper-case
    /// letters.
    #[must_use]
    pub fn new(letters: &str) -> Option<Self> {
        match *letters.as_bytes() {
            [first, second] if first.is_ascii_uppercase() && second.is_ascii_uppercase() => {
                Some(Self([first, second]))
            }
            _ => None,
        }
    }

    /// The two letters.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Both bytes are ASCII letters, so they are always UTF-8.
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

impl fmt::Debug for CountryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CountryCode").field(&self.as_str()).finish()
    }
}

/// Every country a lookup can give, in the order of the table: the table of the format without
/// the entries that are not countries ("--", AP, EU, A1, A2 and O1).
pub fn known_countries() -> impl Iterator<Item = CountryCode> {
    (0..country_codes::TABLE_LEN).filter_map(country_codes::country)
}

/// Something that knows the countries of addresses.
pub trait CountryLookup {
    /// The country of `ip`, or `None` when it has none: an address of a local network, one the
    /// database does not know, or one that belongs to no single country.
    fn country(&mut self, ip: IpAddr) -> Option<CountryCode>;
}

/// The bytes of a database file: [`CountryDb`] reads them at positions, a few at a time.
pub trait Source: Read + Seek + Send {}

impl<T: Read + Seek + Send> Source for T {}

/// Which addresses a database file is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    /// Countries by IPv4 address (`GeoIP.dat`, database type 1).
    V4,
    /// Countries by IPv6 address (`GeoIPv6.dat`, database type 12).
    V6,
}

impl Edition {
    /// The database type a file of this edition carries after its marker.
    const fn type_byte(self) -> u8 {
        match self {
            Self::V4 => 1,
            Self::V6 => 12,
        }
    }

    /// The bits of an address, which is the longest walk through the tree.
    const fn bits(self) -> u32 {
        match self {
            Self::V4 => 32,
            Self::V6 => 128,
        }
    }
}

impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::V4 => "IPv4",
            Self::V6 => "IPv6",
        })
    }
}

/// Why a file is not a country database that can be used. The reasons name no path: the app
/// knows which file it opened.
#[derive(Debug, Error)]
pub enum GeoIpError {
    /// The file is shorter than the smallest database: one node, the marker and the type.
    #[error("the file has {size} bytes, fewer than the 10 of the smallest database")]
    TooShort {
        /// The size of the file in bytes.
        size: u64,
    },
    /// No `FF FF FF` starts in the last 22 bytes, where the marker must be.
    #[error("the file has no database marker (FF FF FF) in its last 22 bytes")]
    NoMarker,
    /// The file is a database of another type: of the other edition, or not of countries.
    #[error(
        "the file is a database of type {found}, not of type {expected} (countries by {edition} \
         address)",
        expected = .edition.type_byte()
    )]
    WrongEdition {
        /// The edition the file was opened as.
        edition: Edition,
        /// The database type the file carries.
        found: u8,
    },
    /// The file does not end with the type alone or with the type and the node count after its
    /// marker: it was cut short, or something follows the database.
    #[error("the file ends {after_marker} bytes after its marker, not 1 or 4")]
    CutTrailer {
        /// How many bytes follow the marker.
        after_marker: usize,
    },
    /// The node count of the file is zero, or the nodes would not fit before the marker.
    #[error("the file names {count} nodes, which do not fit in the {room} bytes before its marker")]
    NodeCount {
        /// The node count the file names.
        count: u32,
        /// The bytes before the marker.
        room: u64,
    },
    /// The walk for the lowest or the highest address does not end in a leaf: it leaves the
    /// tree, or it is still going after the last bit.
    #[error("a walk from the root of the tree does not end in a leaf inside the file")]
    ProbeFailed,
    /// The file could not be read.
    #[error("reading the file failed")]
    Read(#[source] io::Error),
}

/// The two databases of the installed files, either of which may be missing.
#[derive(Debug)]
pub struct GeoIp {
    v4: Option<CountryDb>,
    v6: Option<CountryDb>,
}

impl GeoIp {
    /// The databases to look addresses up in: `v4`, opened as [`Edition::V4`], for IPv4
    /// addresses and `v6`, opened as [`Edition::V6`], for the other IPv6 addresses.
    #[must_use]
    pub fn new(v4: Option<CountryDb>, v6: Option<CountryDb>) -> Self {
        Self { v4, v6 }
    }

    /// No databases: no address has a country.
    #[must_use]
    pub fn none() -> Self {
        Self { v4: None, v6: None }
    }
}

impl CountryLookup for GeoIp {
    /// An IPv4 address, and an IPv4-mapped IPv6 address as the IPv4 address it maps, is looked up
    /// in the IPv4 database only: older builds of the IPv6 file do not have IPv4 addresses, and a
    /// newer one takes 113 reads for one instead of about 20.
    fn country(&mut self, ip: IpAddr) -> Option<CountryCode> {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            IpAddr::V4(_) => ip,
        };
        let database = match ip {
            IpAddr::V4(_) => self.v4.as_mut(),
            IpAddr::V6(_) => self.v6.as_mut(),
        };
        database?.lookup(ip)
    }
}
