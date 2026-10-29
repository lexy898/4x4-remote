//! The global speed limits as the user types them, and the two units they come in.
//!
//! The user reads and types limits in KiB/s ([`KibPerSecond`], 1024 B/s), like every other speed
//! of the UI, while the daemon stores them in kB/s ([`KbPerSecond`], 1000 B/s). [`parse_limit`]
//! reads what the user typed, and [`kib_to_kbps`] and [`kbps_to_kib`] convert between the two
//! units.
//!
//! The round trip KiB/s → kB/s → KiB/s is exact for every limit [`parse_limit`] accepts, so a
//! value the user typed comes back as typed. The other way, kB/s → KiB/s → kB/s, can move a
//! value by one: 21 kB/s show as 21 KiB/s, which go out as 22 kB/s. That is why
//! [`limit_edit`] decides what an edit sends from the text the input started with
//! ([`limit_prefill`]): an untouched value is never converted back.

mod edit;

pub use edit::{LimitEdit, limit_edit, limit_prefill};

use std::num::IntErrorKind;

/// A speed in the daemon's decimal kB/s, 1000 bytes per second, as it reads and stores limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KbPerSecond(pub u32);

/// A speed in KiB/s, 1024 bytes per second, the unit the UI shows and takes limits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KibPerSecond(pub u32);

/// The highest limit [`parse_limit`] accepts: almost 1 GiB/s.
pub const MAX_LIMIT_KIB: KibPerSecond = KibPerSecond(1_000_000);

/// Bytes per second in one of the daemon's kB/s.
const BYTES_PER_KB: u64 = 1000;
/// Bytes per second in one KiB/s.
const BYTES_PER_KIB: u64 = 1024;

/// Why a typed limit is refused.
///
/// The UI words it by variant through its i18n layer. The `Display` text is for logs, and it
/// never repeats what was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LimitError {
    /// The text is not a whole number in plain digits: it holds a sign, a decimal point, a space
    /// between digits, a letter or another character.
    #[error("the speed limit is not a whole number")]
    NotANumber,
    /// The number is 0 or above [`MAX_LIMIT_KIB`].
    #[error("the speed limit is out of range")]
    OutOfRange,
}

/// Reads a speed limit the user typed; `Ok(None)` means no limit.
///
/// Spaces around the text are ignored, and empty text means no limit. Anything else must be
/// ASCII digits only, making a number from 1 to [`MAX_LIMIT_KIB`].
///
/// # Errors
///
/// [`LimitError::NotANumber`] when the text holds anything but ASCII digits;
/// [`LimitError::OutOfRange`] when the number is 0 or above [`MAX_LIMIT_KIB`], however many
/// digits it has.
pub fn parse_limit(text: &str) -> Result<Option<KibPerSecond>, LimitError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    // `parse` alone would also take a leading `+`.
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(LimitError::NotANumber);
    }
    let kib = text.parse::<u32>().map_err(|error| {
        if *error.kind() == IntErrorKind::PosOverflow {
            LimitError::OutOfRange
        } else {
            LimitError::NotANumber
        }
    })?;
    if (1..=MAX_LIMIT_KIB.0).contains(&kib) {
        Ok(Some(KibPerSecond(kib)))
    } else {
        Err(LimitError::OutOfRange)
    }
}

/// A speed in KiB/s as the daemon's kB/s: `kib × 1.024`, rounded half away from zero, and
/// `u32::MAX` kB/s when it does not fit.
#[must_use]
pub fn kib_to_kbps(kib: KibPerSecond) -> KbPerSecond {
    KbPerSecond(rounded_ratio(kib.0, BYTES_PER_KIB, BYTES_PER_KB))
}

/// A speed in the daemon's kB/s as KiB/s: `kbps ÷ 1.024`, rounded half away from zero.
#[must_use]
pub fn kbps_to_kib(kbps: KbPerSecond) -> KibPerSecond {
    KibPerSecond(rounded_ratio(kbps.0, BYTES_PER_KB, BYTES_PER_KIB))
}

/// `value × numerator ÷ denominator`, rounded half away from zero and saturated at `u32::MAX`.
///
/// Exact in `u64`: a `u32` times 1024 cannot overflow it, and the values are never negative, so
/// adding half the denominator rounds halves up, away from zero.
fn rounded_ratio(value: u32, numerator: u64, denominator: u64) -> u32 {
    let scaled = (u64::from(value) * numerator + denominator / 2) / denominator;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_limit_table() {
        let kib = |value| Ok(Some(KibPerSecond(value)));
        let cases: &[(&str, Result<Option<KibPerSecond>, LimitError>)] = &[
            ("", Ok(None)),
            ("   ", Ok(None)),
            (" 512 ", kib(512)),
            ("1", kib(1)),
            ("0512", kib(512)),
            ("1000000", kib(1_000_000)),
            ("0", Err(LimitError::OutOfRange)),
            ("1000001", Err(LimitError::OutOfRange)),
            ("4294967296", Err(LimitError::OutOfRange)),
            ("99999999999999999999", Err(LimitError::OutOfRange)),
            ("-1", Err(LimitError::NotANumber)),
            ("+1", Err(LimitError::NotANumber)),
            ("1.5", Err(LimitError::NotANumber)),
            ("1,5", Err(LimitError::NotANumber)),
            ("1 000", Err(LimitError::NotANumber)),
            ("abc", Err(LimitError::NotANumber)),
            ("12k", Err(LimitError::NotANumber)),
            // A full-width digit is a digit, but not an ASCII one.
            ("\u{ff15}", Err(LimitError::NotANumber)),
        ];

        for (text, expected) in cases {
            assert_eq!(parse_limit(text), *expected, "{text:?}");
        }
    }

    #[test]
    fn kib_kbps_round_trip_is_stable_for_every_valid_limit() {
        for value in 1..=MAX_LIMIT_KIB.0 {
            let kib = KibPerSecond(value);
            let kbps = kib_to_kbps(kib);
            assert_eq!(kbps_to_kib(kbps), kib, "{kib:?} went out as {kbps:?}");
        }
    }

    #[test]
    fn the_conversions_round_half_away_from_zero() {
        let up = |kib| kib_to_kbps(KibPerSecond(kib)).0;
        let down = |kbps| kbps_to_kib(KbPerSecond(kbps)).0;

        assert_eq!(up(0), 0);
        // 1.024 kB/s.
        assert_eq!(up(1), 1);
        // 21.504 kB/s.
        assert_eq!(up(21), 22);
        assert_eq!(up(1000), 1024);
        assert_eq!(kib_to_kbps(MAX_LIMIT_KIB), KbPerSecond(1_024_000));

        assert_eq!(down(0), 0);
        // 97.66 KiB/s.
        assert_eq!(down(100), 98);
        // 62.5 and 187.5 KiB/s: the halves go up.
        assert_eq!(down(64), 63);
        assert_eq!(down(192), 188);
        assert_eq!(down(1024), 1000);
    }

    #[test]
    fn the_conversions_saturate_instead_of_overflowing() {
        assert_eq!(kib_to_kbps(KibPerSecond(u32::MAX)), KbPerSecond(u32::MAX));
        assert_eq!(
            kbps_to_kib(KbPerSecond(u32::MAX)),
            KibPerSecond(4_194_303_999)
        );
    }
}
