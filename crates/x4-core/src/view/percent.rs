//! Whole percents for display.

/// Float noise below this still counts toward the next whole percent: `0.29 * 100.0` is
/// `28.999999999999996`, and a torrent that is exactly 29% done must not show 28%.
const NOISE: f64 = 1e-9;

/// The whole percent a `progress` of 0.0 to 1.0 is shown as.
///
/// It rounds down and reaches 100 only from `progress >= 1.0`, so a torrent never shows 100%
/// before it is done. A value outside the range is clamped to 0 or 100, and NaN is 0.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to 0..=99 first, and a NaN casts to 0"
)]
pub(crate) fn percent(progress: f64) -> u8 {
    if progress >= 1.0 {
        return 100;
    }
    (progress * 100.0 + NOISE).floor().clamp(0.0, 99.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_never_reaches_100_before_done() {
        assert_eq!(percent(0.999), 99);
        assert_eq!(percent(0.999_999_999_9), 99);
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(0.634), 63);
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.005), 0);
    }

    #[test]
    fn percent_ignores_float_noise_below_a_whole_percent() {
        // 0.29 * 100.0 is 28.999999999999996 in f64.
        assert_eq!(percent(0.29), 29);
        assert_eq!(percent(0.57), 57);
        assert_eq!(percent(0.58), 58);
    }

    #[test]
    fn percent_clamps_values_outside_the_range() {
        assert_eq!(percent(-0.5), 0);
        assert_eq!(percent(1.5), 100);
        assert_eq!(percent(f64::INFINITY), 100);
        assert_eq!(percent(f64::NEG_INFINITY), 0);
        assert_eq!(percent(f64::NAN), 0);
    }
}
