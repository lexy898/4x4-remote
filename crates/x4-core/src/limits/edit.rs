//! What an edit of a limit sends: decided from the stored limit, the text and the switch.
//!
//! The input shows the stored kB/s as KiB/s. Turning that text back into kB/s can move the
//! value by one, and a stored value above the input's range would not parse at all. So text
//! the user did not touch is never converted back: the stored value stands as it is.

use super::{KbPerSecond, LimitError, kbps_to_kib, kib_to_kbps, parse_limit};
use crate::snapshot::SpeedLimit;

/// What to send for an edit of one speed limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitEdit {
    /// Nothing: the limit stays as the daemon has it.
    Unchanged,
    /// Switch the limit on at this value.
    Set(KbPerSecond),
    /// Switch the limit off; the daemon keeps its value.
    Off,
}

/// The digits the limit's input starts with: the stored value in KiB/s, also while the limit
/// is off.
#[must_use]
pub fn limit_prefill(limit: SpeedLimit) -> String {
    kbps_to_kib(limit.kbps).0.to_string()
}

/// What to send when the user leaves the limit `current` with `text` in its input and its
/// switch at `enabled`.
///
/// - Switched off: [`LimitEdit::Off`] if the limit is on now, else nothing; the text is not
///   read.
/// - Switched on with the [prefill](limit_prefill) untouched (spaces aside): nothing if the
///   limit is on now, else the stored value as it is, without a conversion or a range check;
///   but a stored 0 is never switched on.
/// - Switched on with other text: what [`parse_limit`] reads. Empty text turns the limit off
///   (or leaves it off); a value is sent in kB/s, unless it is the value already set.
///
/// # Errors
///
/// The [`LimitError`] of [`parse_limit`] when the switch is on and the text was changed into
/// something that is not a limit; [`LimitError::OutOfRange`] when an untouched stored 0 would be
/// switched on.
pub fn limit_edit(current: SpeedLimit, text: &str, enabled: bool) -> Result<LimitEdit, LimitError> {
    let off = if current.enabled {
        LimitEdit::Off
    } else {
        LimitEdit::Unchanged
    };
    if !enabled {
        return Ok(off);
    }
    if text.trim() == limit_prefill(current) {
        return if current.enabled {
            Ok(LimitEdit::Unchanged)
        } else if current.kbps == KbPerSecond(0) {
            // A limit of 0 stops that direction; the user types a value, as for a typed "0".
            Err(LimitError::OutOfRange)
        } else {
            Ok(LimitEdit::Set(current.kbps))
        };
    }
    Ok(match parse_limit(text)? {
        None => off,
        Some(kib) => {
            let kbps = kib_to_kbps(kib);
            if current.enabled && kbps == current.kbps {
                LimitEdit::Unchanged
            } else {
                LimitEdit::Set(kbps)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(kbps: u32, enabled: bool) -> SpeedLimit {
        SpeedLimit {
            kbps: KbPerSecond(kbps),
            enabled,
        }
    }

    #[test]
    fn the_prefill_is_the_stored_value_in_kib() {
        // 20.5 KiB/s round up.
        assert_eq!(limit_prefill(limit(21, true)), "21");
        assert_eq!(limit_prefill(limit(100, false)), "98");
        assert_eq!(limit_prefill(limit(1024, true)), "1000");
        assert_eq!(limit_prefill(limit(1_024_512, true)), "1000500");
    }

    #[test]
    fn an_untouched_prefill_sends_nothing() {
        let current = limit(21, true);

        assert_eq!(limit_edit(current, "21", true), Ok(LimitEdit::Unchanged));
        assert_eq!(limit_edit(current, " 21 ", true), Ok(LimitEdit::Unchanged));
    }

    #[test]
    fn re_enabling_an_untouched_limit_sends_the_stored_value() {
        // Typed, 21 KiB/s would go out as 22 kB/s; untouched, the stored 21 goes out as it is.
        assert_eq!(
            limit_edit(limit(21, false), "21", true),
            Ok(LimitEdit::Set(KbPerSecond(21)))
        );
    }

    #[test]
    fn re_enabling_a_stored_zero_limit_asks_for_a_value() {
        assert_eq!(limit_prefill(limit(0, false)), "0");
        // Untouched, the same as typing "0": a limit of 0 would stop that direction.
        assert_eq!(
            limit_edit(limit(0, false), "0", true),
            Err(LimitError::OutOfRange)
        );
        assert_eq!(
            limit_edit(limit(0, false), "0", true),
            limit_edit(limit(0, false), " 00 ", true)
        );
        assert_eq!(
            limit_edit(limit(0, false), "512", true),
            Ok(LimitEdit::Set(KbPerSecond(524)))
        );
    }

    #[test]
    fn a_stored_value_above_the_input_range_never_errors_when_untouched() {
        let above = 1_024_512;
        let prefill = limit_prefill(limit(above, true));

        assert_eq!(
            limit_edit(limit(above, true), &prefill, true),
            Ok(LimitEdit::Unchanged)
        );
        assert_eq!(
            limit_edit(limit(above, false), &prefill, true),
            Ok(LimitEdit::Set(KbPerSecond(above)))
        );
        // Typed by hand, the same digits are out of range.
        assert_eq!(parse_limit(&prefill), Err(LimitError::OutOfRange));
    }

    #[test]
    fn typing_1000_kib_sends_1024_kb() {
        let set = Ok(LimitEdit::Set(KbPerSecond(1024)));

        assert_eq!(limit_edit(limit(100, true), "1000", true), set);
        assert_eq!(limit_edit(limit(100, false), "1000", true), set);
    }

    #[test]
    fn typing_the_value_already_set_sends_nothing() {
        // 1000 KiB/s are the 1024 kB/s set now, though the prefill reads differently.
        assert_eq!(
            limit_edit(limit(1024, true), "01000", true),
            Ok(LimitEdit::Unchanged)
        );
        // While the limit is off, the same value turns it on.
        assert_eq!(
            limit_edit(limit(1024, false), "01000", true),
            Ok(LimitEdit::Set(KbPerSecond(1024)))
        );
    }

    #[test]
    fn clearing_the_text_turns_the_limit_off() {
        assert_eq!(limit_edit(limit(100, true), "", true), Ok(LimitEdit::Off));
        assert_eq!(
            limit_edit(limit(100, true), "   ", true),
            Ok(LimitEdit::Off)
        );
        // Already off: nothing to send.
        assert_eq!(
            limit_edit(limit(100, false), "", true),
            Ok(LimitEdit::Unchanged)
        );
    }

    #[test]
    fn turning_off_ignores_the_text() {
        assert_eq!(
            limit_edit(limit(100, true), "abc", false),
            Ok(LimitEdit::Off)
        );
        assert_eq!(
            limit_edit(limit(100, false), "abc", false),
            Ok(LimitEdit::Unchanged)
        );
        assert_eq!(
            limit_edit(limit(100, true), "99999999", false),
            Ok(LimitEdit::Off)
        );
    }

    #[test]
    fn a_changed_text_that_is_not_a_limit_is_refused() {
        assert_eq!(
            limit_edit(limit(100, true), "abc", true),
            Err(LimitError::NotANumber)
        );
        assert_eq!(
            limit_edit(limit(100, false), "0", true),
            Err(LimitError::OutOfRange)
        );
        assert_eq!(
            limit_edit(limit(100, true), "1000001", true),
            Err(LimitError::OutOfRange)
        );
    }
}
