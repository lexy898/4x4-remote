//! The global speed limits as the user types them, and the two units they come in.
//!
//! The user reads and types limits in KiB/s (1024 B/s), like every other speed of the UI, while
//! the daemon stores them in kB/s (1000 B/s). [`parse_limit`] reads what the user typed, and
//! [`kib_to_kbps`] and [`kbps_to_kib`] convert between the two units. A limit that goes out
//! through one comes back unchanged through the other.

use std::num::IntErrorKind;

/// The highest limit [`parse_limit`] accepts, in KiB/s: almost 1 GiB/s.
pub const MAX_LIMIT_KIB: u32 = 1_000_000;

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

/// Reads a speed limit the user typed, in KiB/s; `Ok(None)` means no limit.
///
/// Spaces around the text are ignored, and empty text means no limit. Anything else must be
/// ASCII digits only, making a number from 1 to [`MAX_LIMIT_KIB`].
///
/// # Errors
///
/// [`LimitError::NotANumber`] when the text holds anything but ASCII digits;
/// [`LimitError::OutOfRange`] when the number is 0 or above [`MAX_LIMIT_KIB`], however many
/// digits it has.
pub fn parse_limit(text: &str) -> Result<Option<u32>, LimitError> {
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
    if (1..=MAX_LIMIT_KIB).contains(&kib) {
        Ok(Some(kib))
    } else {
        Err(LimitError::OutOfRange)
    }
}

/// A limit in KiB/s as the daemon's kB/s: `kib × 1.024`, rounded half away from zero, and
/// `u32::MAX` when it does not fit.
#[must_use]
pub fn kib_to_kbps(kib: u32) -> u32 {
    rounded_ratio(kib, BYTES_PER_KIB, BYTES_PER_KB)
}

/// A limit in the daemon's kB/s as KiB/s: `kbps ÷ 1.024`, rounded half away from zero.
#[must_use]
pub fn kbps_to_kib(kbps: u32) -> u32 {
    rounded_ratio(kbps, BYTES_PER_KB, BYTES_PER_KIB)
}

/// A rate in the daemon's kB/s as bytes per second.
pub(crate) fn kbps_to_bytes_per_sec(kbps: u32) -> u64 {
    u64::from(kbps) * BYTES_PER_KB
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
        let cases: &[(&str, Result<Option<u32>, LimitError>)] = &[
            ("", Ok(None)),
            ("   ", Ok(None)),
            (" 512 ", Ok(Some(512))),
            ("1", Ok(Some(1))),
            ("0512", Ok(Some(512))),
            ("1000000", Ok(Some(1_000_000))),
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
        for kib in 1..=MAX_LIMIT_KIB {
            let kbps = kib_to_kbps(kib);
            assert_eq!(
                kbps_to_kib(kbps),
                kib,
                "{kib} KiB/s went out as {kbps} kB/s"
            );
        }
    }

    #[test]
    fn the_conversions_round_half_away_from_zero() {
        assert_eq!(kib_to_kbps(0), 0);
        // 1.024 kB/s.
        assert_eq!(kib_to_kbps(1), 1);
        // 21.504 kB/s.
        assert_eq!(kib_to_kbps(21), 22);
        assert_eq!(kib_to_kbps(1000), 1024);
        assert_eq!(kib_to_kbps(MAX_LIMIT_KIB), 1_024_000);

        assert_eq!(kbps_to_kib(0), 0);
        // 97.66 KiB/s.
        assert_eq!(kbps_to_kib(100), 98);
        // 62.5 and 187.5 KiB/s: the halves go up.
        assert_eq!(kbps_to_kib(64), 63);
        assert_eq!(kbps_to_kib(192), 188);
        assert_eq!(kbps_to_kib(1024), 1000);
    }

    #[test]
    fn the_conversions_saturate_instead_of_overflowing() {
        assert_eq!(kib_to_kbps(u32::MAX), u32::MAX);
        assert_eq!(kbps_to_kib(u32::MAX), 4_194_303_999);
    }

    #[test]
    fn a_kb_per_second_is_a_thousand_bytes() {
        assert_eq!(kbps_to_bytes_per_sec(0), 0);
        assert_eq!(kbps_to_bytes_per_sec(50), 50_000);
        assert_eq!(kbps_to_bytes_per_sec(u32::MAX), 4_294_967_295_000);
    }
}
