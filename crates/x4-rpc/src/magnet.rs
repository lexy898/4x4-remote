//! Magnet links, validated before they reach the daemon (see [`MagnetLink::parse`]).
//!
//! `torrent_add` reads its `filename` as a URL to fetch, a path on the server's disk to read, or
//! a magnet link (T/rpcimpl.cc:1848-1874). A [`MagnetLink`] is the only thing this crate sends
//! there, so a URL or a path never reaches the daemon by mistake.

use std::fmt;

use thiserror::Error;

use crate::ids::HashString;

/// The longest magnet link accepted, in bytes: 64 KiB.
///
/// Real links are far shorter; a few hundred trackers still fit.
pub const MAX_MAGNET_BYTES: usize = 64 * 1024;

/// A magnet link that Transmission 4.1 adds by its v1 info hash (SHA-1).
///
/// Built only by [`MagnetLink::parse`], which reads the link the way the daemon does
/// (`parseMagnet`, T/magnet-metainfo.cc:218-286), so the hash and the name here are the ones the
/// daemon will show.
///
/// Its `Debug` shows only the hash and the length: trackers in the link can carry passkeys, and
/// a request that holds a link may be logged.
#[derive(Clone, PartialEq, Eq)]
pub struct MagnetLink {
    text: String,
    hash: HashString,
    name: Option<String>,
}

impl MagnetLink {
    /// Validates a magnet link.
    ///
    /// The text is trimmed of white space. It must start with `magnet:?` in any case, which is
    /// sent in lower case, because the daemon accepts only that spelling. Its query is split on
    /// `&`, each entry into a key and a value at the first `=`, keys compared exactly as the
    /// daemon does. Exactly one `xt=urn:btih:` must give the info hash, as 40 hexadecimal or 32
    /// base32 characters, not percent-decoded.
    ///
    /// # Errors
    ///
    /// A [`MagnetError`] when the text is empty, longer than [`MAX_MAGNET_BYTES`], holds a
    /// control character, is a bare info hash, does not start with `magnet:?`, or has no valid
    /// v1 info hash, or several.
    pub fn parse(text: &str) -> Result<Self, MagnetError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(MagnetError::Empty);
        }
        if text.len() > MAX_MAGNET_BYTES {
            return Err(MagnetError::TooLong { len: text.len() });
        }
        if let Some(offset) = text.find(char::is_control) {
            return Err(MagnetError::ControlCharacter { offset });
        }
        if info_hash(text).is_some() {
            return Err(MagnetError::BareHash);
        }
        let query = text
            .get(..PREFIX.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
            .and_then(|_| text.get(PREFIX.len()..))
            .ok_or(MagnetError::NotMagnet)?;

        // What the daemon reads, in the order it reads it (T/magnet-metainfo.cc:237-276).
        let mut hashes = Vec::new();
        let mut v2 = false;
        let mut name = None;
        for entry in query.split('&') {
            let (key, value) = entry.split_once('=').unwrap_or((entry, ""));
            match key {
                "dn" => name = Some(display_name(value)),
                "xt" => {
                    if let Some(hash) = value.strip_prefix("urn:btih:") {
                        hashes.push(info_hash(hash));
                    } else if value.starts_with("urn:btmh:") {
                        v2 = true;
                    }
                }
                _ => {}
            }
        }
        let digest = match hashes.as_slice() {
            [] if v2 => return Err(MagnetError::V2Only),
            [] => return Err(MagnetError::NoHash),
            [hash] => hash.ok_or(MagnetError::InvalidHash)?,
            several => {
                return Err(MagnetError::SeveralHashes {
                    count: several.len(),
                });
            }
        };
        Ok(Self {
            text: format!("{PREFIX}{query}"),
            hash: HashString::from_sha1(&digest),
            name: name.filter(|name: &String| !name.is_empty()),
        })
    }

    /// The link as it is sent: trimmed, starting with a lower-case `magnet:?`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The info hash the daemon keys the torrent by, as lowercase hex; a base32 hash is
    /// converted.
    #[must_use]
    pub fn hash(&self) -> &HashString {
        &self.hash
    }

    /// The display name (`dn`), percent-decoded as the daemon decodes it: `%XX` only, a `+`
    /// stays a `+`. It is cut at a NUL, and bytes that are not UTF-8 become U+FFFD. `None` when
    /// the link has no name or an empty one; the daemon then shows the hash.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl fmt::Debug for MagnetLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MagnetLink")
            .field("hash", &self.hash)
            .field("len", &self.text.len())
            .finish_non_exhaustive()
    }
}

/// The only spelling the daemon takes as a magnet link (T/web-utils.cc:308-316).
const PREFIX: &str = "magnet:?";

/// The SHA-1 digest a v1 info hash names: 40 hexadecimal digits in either case, or 32 base32
/// characters (RFC 4648, either case, no padding), as `parseHash` reads them
/// (T/magnet-metainfo.cc:114-148).
fn info_hash(text: &str) -> Option<[u8; 20]> {
    let bytes = text.as_bytes();
    if let Ok(digits) = <&[u8; 40]>::try_from(bytes) {
        decode_hex(digits)
    } else if let Ok(characters) = <&[u8; 32]>::try_from(bytes) {
        decode_base32(characters)
    } else {
        None
    }
}

fn decode_hex(digits: &[u8; 40]) -> Option<[u8; 20]> {
    let mut digest = [0; 20];
    for (byte, &[high, low]) in digest.iter_mut().zip(digits.as_chunks::<2>().0) {
        *byte = (hex_digit(high)? << 4) | hex_digit(low)?;
    }
    Some(digest)
}

/// Eight base32 characters carry 40 bits, which are five bytes of the digest, high bits first.
fn decode_base32(characters: &[u8; 32]) -> Option<[u8; 20]> {
    let mut digest = [0; 20];
    let groups = characters.as_chunks::<8>().0;
    for (bytes, group) in digest.as_chunks_mut::<5>().0.iter_mut().zip(groups) {
        let mut bits = 0_u64;
        for &character in group {
            bits = (bits << 5) | u64::from(base32_digit(character)?);
        }
        let [_, _, _, low @ ..] = bits.to_be_bytes();
        *bytes = low;
    }
    Some(digest)
}

fn hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

fn base32_digit(character: u8) -> Option<u8> {
    match character {
        b'A'..=b'Z' => Some(character - b'A'),
        b'a'..=b'z' => Some(character - b'a'),
        b'2'..=b'7' => Some(character - b'2' + 26),
        _ => None,
    }
}

/// The name `dn` gives, as the daemon makes it: `%XX` decoded, with no `+` to space
/// (`tr_urlPercentDecode`, T/web-utils.cc:465-495), then cut at the first NUL, with bytes that
/// are not UTF-8 replaced (`tr_strv_replace_invalid`, T/utils.cc:325-336).
fn display_name(value: &str) -> String {
    let mut decoded = Vec::with_capacity(value.len());
    let mut rest = value.as_bytes();
    while let Some((&first, after)) = rest.split_first() {
        if let (b'%', &[high, low, ..]) = (first, after)
            && let (Some(high), Some(low)) = (hex_digit(high), hex_digit(low))
        {
            decoded.push((high << 4) | low);
            rest = after.get(2..).unwrap_or_default();
        } else {
            decoded.push(first);
            rest = after;
        }
    }
    if let Some(nul) = decoded.iter().position(|&byte| byte == 0) {
        decoded.truncate(nul);
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Why a text is not a magnet link Transmission 4.1 can add.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MagnetError {
    /// Nothing but white space.
    #[error("the magnet link is empty")]
    Empty,
    /// Longer than [`MAX_MAGNET_BYTES`].
    #[error(
        "the magnet link has {len} bytes, more than the {max} accepted",
        max = MAX_MAGNET_BYTES
    )]
    TooLong {
        /// The length of the trimmed link in bytes.
        len: usize,
    },
    /// A control character, such as a line break inside the text.
    #[error("the magnet link has a control character at byte {offset}")]
    ControlCharacter {
        /// The offset of the character in the trimmed link.
        offset: usize,
    },
    /// A bare info hash, without `magnet:?xt=urn:btih:` around it.
    #[error("a bare info hash is not a magnet link")]
    BareHash,
    /// The text does not start with `magnet:?`.
    #[error("the text does not start with magnet:?")]
    NotMagnet,
    /// No `xt=urn:btih:` entry.
    #[error("the magnet link has no BitTorrent v1 info hash (xt=urn:btih:)")]
    NoHash,
    /// Only a v2 info hash (`xt=urn:btmh:`), which Transmission 4.1 cannot add.
    #[error(
        "the magnet link has only a BitTorrent v2 info hash (xt=urn:btmh:), which Transmission 4.1 \
         cannot add"
    )]
    V2Only,
    /// The `xt=urn:btih:` value is not 40 hexadecimal or 32 base32 characters.
    #[error("the info hash of the magnet link is not 40 hexadecimal or 32 base32 characters")]
    InvalidHash,
    /// More than one `xt=urn:btih:` entry: the daemon would take the last valid one, so the hash
    /// shown here could differ from the one it adds.
    #[error("the magnet link has {count} info hashes (xt=urn:btih:), not one")]
    SeveralHashes {
        /// How many `xt=urn:btih:` entries the link has.
        count: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
    /// [`HASH`] in base32 (RFC 4648, as Python's `base64.b32encode` writes it).
    const HASH_BASE32: &str = "AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH";
    const V2_HASH: &str = "1220aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn link(query: &str) -> String {
        format!("magnet:?{query}")
    }

    #[test]
    fn a_magnet_is_normalized_and_its_hash_and_name_read() {
        let text = format!(
            " \t\n magnet:?xt=urn:btih:{}&dn=Big%20Buck+Bunny%2Fx&tr=udp%3A%2F%2Ft.example%3A80 \r\n",
            HASH.to_uppercase()
        );

        let magnet = MagnetLink::parse(&text).unwrap();

        assert_eq!(
            magnet.as_str(),
            format!(
                "magnet:?xt=urn:btih:{}&dn=Big%20Buck+Bunny%2Fx&tr=udp%3A%2F%2Ft.example%3A80",
                HASH.to_uppercase()
            )
        );
        assert_eq!(magnet.hash().as_str(), HASH);
        assert_eq!(magnet.name(), Some("Big Buck+Bunny/x"));

        // Raw UTF-8 and spaces, as KIO passes them, are kept; a bad escape stays as it is.
        let raw = MagnetLink::parse(&link(&format!("dn=Фильм 1%2&xt=urn:btih:{HASH}"))).unwrap();
        assert_eq!(raw.name(), Some("Фильм 1%2"));

        // The last `dn` wins, as in the daemon; an empty one, or one that starts with a NUL,
        // leaves no name. Bytes that are not UTF-8 become U+FFFD, and a NUL cuts the name.
        let name = |query: &str| {
            MagnetLink::parse(&link(&format!("xt=urn:btih:{HASH}&{query}")))
                .unwrap()
                .name()
                .map(str::to_owned)
        };
        assert_eq!(name("dn=a&dn=b"), Some("b".to_owned()));
        assert_eq!(name("dn=a&dn="), None);
        assert_eq!(name("dn=%00a"), None);
        assert_eq!(name("dn=a%00b"), Some("a".to_owned()));
        assert_eq!(name("dn=a%FFb"), Some("a\u{FFFD}b".to_owned()));
        assert_eq!(name("tr=x"), None);
        // Keys are compared exactly: `DN` is not a name.
        assert_eq!(name("DN=x"), None);
    }

    #[test]
    fn upper_case_scheme_is_sent_lower_case() {
        for prefix in ["MAGNET:?", "Magnet:?", "magnet:?"] {
            let magnet = MagnetLink::parse(&format!("{prefix}xt=urn:btih:{HASH}")).unwrap();

            assert_eq!(magnet.as_str(), link(&format!("xt=urn:btih:{HASH}")));
        }
        assert_eq!(
            MagnetLink::parse(&format!("magnet:xt=urn:btih:{HASH}")),
            Err(MagnetError::NotMagnet)
        );
        assert_eq!(
            MagnetLink::parse(&format!("http://example.com/?xt=urn:btih:{HASH}")),
            Err(MagnetError::NotMagnet)
        );
        assert_eq!(
            MagnetLink::parse("/var/lib/transmission/x.torrent"),
            Err(MagnetError::NotMagnet)
        );
    }

    #[test]
    fn a_base32_hash_is_accepted() {
        for base32 in [HASH_BASE32.to_owned(), HASH_BASE32.to_lowercase()] {
            let magnet = MagnetLink::parse(&link(&format!("xt=urn:btih:{base32}"))).unwrap();

            assert_eq!(magnet.hash().as_str(), HASH);
            // It is sent as it was given: the daemon reads base32 itself.
            assert_eq!(magnet.as_str(), link(&format!("xt=urn:btih:{base32}")));
        }
        let all_ones = MagnetLink::parse(&link(&format!("xt=urn:btih:{}", "7".repeat(32))));
        assert_eq!(all_ones.unwrap().hash().as_str(), "f".repeat(40));
        // A hybrid link has a v1 hash, which is what the daemon adds it by.
        let hybrid = MagnetLink::parse(&link(&format!("xt=urn:btmh:{V2_HASH}&xt=urn:btih:{HASH}")));
        assert_eq!(hybrid.unwrap().hash().as_str(), HASH);
    }

    #[test]
    fn btmh_only_and_bad_hashes_are_refused() {
        let parse = |query: &str| MagnetLink::parse(&link(query));

        assert_eq!(
            parse(&format!("xt=urn:btmh:{V2_HASH}&dn=x")),
            Err(MagnetError::V2Only)
        );
        assert_eq!(parse("dn=x&tr=y"), Err(MagnetError::NoHash));
        assert_eq!(parse(""), Err(MagnetError::NoHash));
        let bad = [
            &HASH[1..],
            &format!("{HASH}0"),
            &format!("{}g", &HASH[1..]),
            &HASH_BASE32[1..],
            &format!("{}1", &HASH_BASE32[1..]),
            &format!("{}8", &HASH_BASE32[1..]),
            // The value is not percent-decoded, as in the daemon.
            &format!("{}%41", &HASH[2..]),
            "",
        ];
        for value in bad {
            assert_eq!(
                parse(&format!("xt=urn:btih:{value}")),
                Err(MagnetError::InvalidHash),
                "{value}"
            );
        }
        // The daemon matches the key and the `urn:btih:` prefix exactly, and does not decode
        // them.
        for query in [
            format!("XT=urn:btih:{HASH}"),
            format!("xt=URN:BTIH:{HASH}"),
            format!("xt=urn%3Abtih%3A{HASH}"),
        ] {
            assert_eq!(parse(&query), Err(MagnetError::NoHash), "{query}");
        }
        // Two hashes, even the same one twice, are refused: the daemon takes the last valid one.
        let other = "f".repeat(40);
        assert_eq!(
            parse(&format!("xt=urn:btih:{HASH}&xt=urn:btih:{other}")),
            Err(MagnetError::SeveralHashes { count: 2 })
        );
        assert_eq!(
            parse(&format!("xt=urn:btih:{HASH}&xt=urn:btih:{HASH_BASE32}")),
            Err(MagnetError::SeveralHashes { count: 2 })
        );
        assert_eq!(
            parse(&format!("xt=urn:btih:{HASH}&xt=urn:btih:bad")),
            Err(MagnetError::SeveralHashes { count: 2 })
        );
        // A bare hash is refused: only the explicit `magnet:?` form is taken.
        for bare in [HASH, HASH_BASE32, &format!("  {HASH}\n")] {
            assert_eq!(
                MagnetLink::parse(bare),
                Err(MagnetError::BareHash),
                "{bare}"
            );
        }
    }

    #[test]
    fn control_characters_and_oversized_magnets_are_refused() {
        let valid = link(&format!("xt=urn:btih:{HASH}&dn="));
        for control in ['\0', '\t', '\u{1b}', '\u{7f}', '\u{85}'] {
            let text = format!("{valid}a{control}b");
            assert_eq!(
                MagnetLink::parse(&text),
                Err(MagnetError::ControlCharacter {
                    offset: valid.len() + 1
                }),
                "{control:?}"
            );
        }
        // Several lines are refused, not read as one link.
        let two = format!("{valid}a\n{valid}b");
        assert_eq!(
            MagnetLink::parse(&two),
            Err(MagnetError::ControlCharacter {
                offset: valid.len() + 1
            })
        );
        assert_eq!(MagnetLink::parse(" \t\r\n "), Err(MagnetError::Empty));
        assert_eq!(MagnetLink::parse(""), Err(MagnetError::Empty));

        let longest = format!("{valid}{}", "a".repeat(MAX_MAGNET_BYTES - valid.len()));
        assert_eq!(longest.len(), MAX_MAGNET_BYTES);
        assert!(MagnetLink::parse(&format!(" {longest} ")).is_ok());
        let too_long = format!("{longest}a");
        assert_eq!(
            MagnetLink::parse(&too_long),
            Err(MagnetError::TooLong {
                len: MAX_MAGNET_BYTES + 1
            })
        );
        let huge = format!("{valid}{}", "a".repeat(1024 * 1024));
        assert_eq!(
            MagnetLink::parse(&huge),
            Err(MagnetError::TooLong { len: huge.len() })
        );
    }
}
