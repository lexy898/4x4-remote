//! Progress for display: whole percents and the fraction for the bar.

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

/// `progress` limited to 0.0 to 1.0, so a bar never overflows its track; NaN is 0.0.
#[must_use]
pub(crate) fn fraction(progress: f64) -> f64 {
    if progress.is_nan() {
        0.0
    } else {
        progress.clamp(0.0, 1.0)
    }
}

/// The largest `f64` below 1.0.
const BELOW_ONE: f64 = 1.0 - f64::EPSILON / 2.0;

/// `done` of `total` as 0.0 to 1.0, for a bar and for [`percent`]. Nothing to do (`total` is 0)
/// counts as done, so an empty file or folder is not shown as 0%; `done` beyond `total` counts as
/// all of it. It is 1.0 only when `done` has reached `total`, also for sizes where the division
/// rounds up.
#[must_use]
pub(crate) fn fraction_of(done: u64, total: u64) -> f64 {
    if done >= total {
        return 1.0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "the value is only shown, as a bar and whole percents"
    )]
    let fraction = done as f64 / total as f64;
    fraction.min(BELOW_ONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_never_reaches_100_before_done() {
        assert_eq!(percent(0.999), 99);
        assert_eq!(percent(0.999_999_999_9), 99);
        assert_eq!(percent(0.999_999_999_999), 99);
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
    fn fraction_limits_progress_to_the_unit_interval() {
        assert!((fraction(0.634) - 0.634).abs() < f64::EPSILON);
        assert!(fraction(0.0).abs() < f64::EPSILON);
        assert!((fraction(1.0) - 1.0).abs() < f64::EPSILON);
        assert!(fraction(-0.2).abs() < f64::EPSILON);
        assert!((fraction(1.7) - 1.0).abs() < f64::EPSILON);
        assert!((fraction(f64::INFINITY) - 1.0).abs() < f64::EPSILON);
        assert!(fraction(f64::NEG_INFINITY).abs() < f64::EPSILON);
        assert!(fraction(f64::NAN).abs() < f64::EPSILON);
    }

    #[test]
    fn percent_clamps_values_outside_the_range() {
        assert_eq!(percent(-0.5), 0);
        assert_eq!(percent(1.5), 100);
        assert_eq!(percent(f64::INFINITY), 100);
        assert_eq!(percent(f64::NEG_INFINITY), 0);
        assert_eq!(percent(f64::NAN), 0);
    }

    #[test]
    fn fraction_of_counts_nothing_to_do_as_done() {
        assert!((fraction_of(0, 0) - 1.0).abs() < f64::EPSILON);
        assert!((fraction_of(5, 0) - 1.0).abs() < f64::EPSILON);
        assert!((fraction_of(10, 10) - 1.0).abs() < f64::EPSILON);
        assert!((fraction_of(11, 10) - 1.0).abs() < f64::EPSILON);
        assert!((fraction_of(1, 4) - 0.25).abs() < f64::EPSILON);
        assert!(fraction_of(0, 10).abs() < f64::EPSILON);
        // One byte short stays below 100%, also for sizes beyond what f64 holds exactly.
        assert_eq!(percent(fraction_of(9, 10)), 90);
        assert_eq!(percent(fraction_of(u64::MAX - 1, u64::MAX)), 99);
    }
}
