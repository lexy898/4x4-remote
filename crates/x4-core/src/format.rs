//! Sizes, speeds and durations as structured values for the UI to render.
//!
//! Nothing here produces text: the UI turns a [`HumanSize`] into "22.9 GiB" through its i18n
//! layer, using [`HumanSize::fraction_digits`] for the number of decimals. Units are binary
//! (base 1024).

use std::time::Duration;

/// The unit of a [`HumanSize`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeUnit {
    /// Bytes.
    B,
    /// Kibibytes, 1024 bytes.
    KiB,
    /// Mebibytes, 1024 KiB.
    MiB,
    /// Gibibytes, 1024 MiB.
    GiB,
    /// Tebibytes, 1024 GiB; the largest unit.
    TiB,
}

/// A size scaled to the unit that reads best, already rounded for display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HumanSize {
    /// The scaled value: a whole number of bytes, otherwise rounded to one decimal below 100
    /// and to a whole number from 100.
    pub value: f64,
    /// The unit of `value`.
    pub unit: SizeUnit,
}

impl HumanSize {
    /// How many decimals to show: none for bytes and for values from 100, otherwise one.
    #[must_use]
    pub fn fraction_digits(&self) -> usize {
        fraction_digits(self.value, self.unit == SizeUnit::B)
    }
}

/// The unit of a [`HumanSpeed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeedUnit {
    /// Bytes per second.
    BytesPerSec,
    /// Kibibytes per second.
    KiBPerSec,
    /// Mebibytes per second.
    MiBPerSec,
    /// Gibibytes per second; the largest unit.
    GiBPerSec,
}

/// A transfer rate scaled to the unit that reads best, already rounded for display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HumanSpeed {
    /// The scaled value, rounded like [`HumanSize::value`].
    pub value: f64,
    /// The unit of `value`.
    pub unit: SpeedUnit,
}

impl HumanSpeed {
    /// How many decimals to show: none for bytes per second and for values from 100, otherwise
    /// one.
    #[must_use]
    pub fn fraction_digits(&self) -> usize {
        fraction_digits(self.value, self.unit == SpeedUnit::BytesPerSec)
    }
}

/// A duration split for display; the UI picks the parts it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DurationParts {
    /// Whole days.
    pub days: u64,
    /// Hours past the last whole day, 0 to 23.
    pub hours: u8,
    /// Minutes past the last whole hour, 0 to 59.
    pub minutes: u8,
    /// Seconds past the last whole minute, 0 to 59.
    pub seconds: u8,
}

/// A byte count scaled to the largest binary unit in which it is at least 1.
///
/// Below 1024 the unit is [`SizeUnit::B`]. A value that rounds up to 1024 moves up one unit, so
/// 1023.97 KiB is 1.0 MiB, except at the top unit, [`SizeUnit::TiB`], which just grows.
#[must_use]
pub fn human_size(bytes: u64) -> HumanSize {
    let (value, unit) = scale(
        bytes,
        SizeUnit::B,
        &[SizeUnit::KiB, SizeUnit::MiB, SizeUnit::GiB, SizeUnit::TiB],
    );
    HumanSize { value, unit }
}

/// A transfer rate scaled like [`human_size`], with [`SpeedUnit::GiBPerSec`] as the top unit.
#[must_use]
pub fn human_speed(bytes_per_sec: u64) -> HumanSpeed {
    let (value, unit) = scale(
        bytes_per_sec,
        SpeedUnit::BytesPerSec,
        &[
            SpeedUnit::KiBPerSec,
            SpeedUnit::MiBPerSec,
            SpeedUnit::GiBPerSec,
        ],
    );
    HumanSpeed { value, unit }
}

/// A duration split into days, hours, minutes and seconds; fractions of a second are dropped.
#[must_use]
pub fn duration_parts(d: Duration) -> DurationParts {
    let total = d.as_secs();
    // Each part is below 24 or 60, so the saturation never applies.
    let part = |value: u64| u8::try_from(value).unwrap_or(u8::MAX);
    DurationParts {
        days: total / SECONDS_PER_DAY,
        hours: part(total % SECONDS_PER_DAY / SECONDS_PER_HOUR),
        minutes: part(total % SECONDS_PER_HOUR / SECONDS_PER_MINUTE),
        seconds: part(total % SECONDS_PER_MINUTE),
    }
}

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;

const BASE: f64 = 1024.0;

/// Scales `amount` down by 1024 through `larger` while it is at least 1024, then rounds it for
/// display; a value that rounds up to 1024 moves up one more unit if there is one.
///
/// `base` is the unit of `amount` itself; `larger` lists the following units in order.
#[expect(
    clippy::cast_precision_loss,
    reason = "the value is only shown, rounded to at most one decimal"
)]
fn scale<U: Copy>(amount: u64, base: U, larger: &[U]) -> (f64, U) {
    let mut value = amount as f64;
    let mut unit = base;
    let mut steps = larger.iter();
    while value >= BASE
        && let Some(&next) = steps.next()
    {
        value /= BASE;
        unit = next;
    }
    let mut rounded = round_for_display(value);
    if rounded >= BASE
        && let Some(&next) = steps.next()
    {
        // At most 1024, since the value was below 1024 before rounding.
        rounded = round_for_display(rounded / BASE);
        unit = next;
    }
    (rounded, unit)
}

/// One decimal below 100, none from 100.
fn round_for_display(value: f64) -> f64 {
    if value < 100.0 {
        (value * 10.0).round() / 10.0
    } else {
        value.round()
    }
}

fn fraction_digits(value: f64, is_base_unit: bool) -> usize {
    usize::from(!is_base_unit && value < 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    fn size(value: f64, unit: SizeUnit) -> HumanSize {
        HumanSize { value, unit }
    }

    fn speed(value: f64, unit: SpeedUnit) -> HumanSpeed {
        HumanSpeed { value, unit }
    }

    #[test]
    fn human_size_examples() {
        assert_eq!(human_size(0), size(0.0, SizeUnit::B));
        assert_eq!(human_size(1023), size(1023.0, SizeUnit::B));
        assert_eq!(human_size(1024), size(1.0, SizeUnit::KiB));
        assert_eq!(human_size(24_592_841_836), size(22.9, SizeUnit::GiB));
        assert_eq!(human_size(157_286_400), size(150.0, SizeUnit::MiB));
        assert_eq!(human_size(1_099_511_627_776), size(1.0, SizeUnit::TiB));
    }

    #[test]
    fn human_speed_examples() {
        assert_eq!(human_speed(0), speed(0.0, SpeedUnit::BytesPerSec));
        assert_eq!(human_speed(2_516_582), speed(2.4, SpeedUnit::MiBPerSec));
        assert_eq!(human_speed(184_320), speed(180.0, SpeedUnit::KiBPerSec));
    }

    #[test]
    fn human_size_rounds_to_one_decimal_below_100_and_to_whole_numbers_from_100() {
        // 1.5 KiB, 99.9 KiB and 100.4 KiB.
        assert_eq!(human_size(1_536), size(1.5, SizeUnit::KiB));
        assert_eq!(human_size(102_298), size(99.9, SizeUnit::KiB));
        assert_eq!(human_size(102_810), size(100.0, SizeUnit::KiB));
    }

    #[test]
    fn human_size_rounding_up_to_100_has_no_decimals() {
        // 99.96 KiB rounds to 100.0, which is shown without decimals.
        let rounded = human_size(102_359);
        assert_eq!(rounded, size(100.0, SizeUnit::KiB));
        assert_eq!(rounded.fraction_digits(), 0);
    }

    #[test]
    fn human_size_carries_into_the_next_unit_when_rounding_reaches_1024() {
        // 1023.97 KiB would read "1024 KiB".
        assert_eq!(human_size(1_048_545), size(1.0, SizeUnit::MiB));
        assert_eq!(human_size(MIB - 1), size(1.0, SizeUnit::MiB));
        assert_eq!(human_size(GIB - 1), size(1.0, SizeUnit::GiB));
        assert_eq!(human_size(TIB - 1), size(1.0, SizeUnit::TiB));
    }

    #[test]
    fn human_size_does_not_carry_below_the_rounding_edge() {
        // 1023.3 KiB rounds to 1023, which stays in KiB.
        assert_eq!(human_size(1_047_900), size(1023.0, SizeUnit::KiB));
    }

    #[test]
    fn human_size_stays_in_tib_beyond_the_top_unit() {
        assert_eq!(human_size(2_000 * TIB), size(2_000.0, SizeUnit::TiB));
        // u64::MAX is 2^64 - 1 bytes, which rounds to 2^24 TiB.
        assert_eq!(human_size(u64::MAX), size(16_777_216.0, SizeUnit::TiB));
    }

    #[test]
    fn human_speed_carries_and_stops_at_gib_per_sec() {
        assert_eq!(human_speed(MIB - 1), speed(1.0, SpeedUnit::MiBPerSec));
        assert_eq!(human_speed(GIB - 1), speed(1.0, SpeedUnit::GiBPerSec));
        assert_eq!(human_speed(TIB), speed(1_024.0, SpeedUnit::GiBPerSec));
        // u64::MAX is 2^64 - 1 bytes/s, which rounds to 2^34 GiB/s.
        assert_eq!(
            human_speed(u64::MAX),
            speed(17_179_869_184.0, SpeedUnit::GiBPerSec)
        );
    }

    #[test]
    fn size_fraction_digits_follow_the_rounded_value() {
        assert_eq!(human_size(0).fraction_digits(), 0);
        assert_eq!(human_size(99).fraction_digits(), 0);
        assert_eq!(human_size(1_023).fraction_digits(), 0);
        assert_eq!(human_size(1_536).fraction_digits(), 1);
        assert_eq!(human_size(24_592_841_836).fraction_digits(), 1);
        assert_eq!(human_size(157_286_400).fraction_digits(), 0);
        assert_eq!(human_size(MIB - 1).fraction_digits(), 1);
    }

    #[test]
    fn speed_fraction_digits_follow_the_rounded_value() {
        assert_eq!(human_speed(0).fraction_digits(), 0);
        assert_eq!(human_speed(99).fraction_digits(), 0);
        assert_eq!(human_speed(2_516_582).fraction_digits(), 1);
        assert_eq!(human_speed(184_320).fraction_digits(), 0);
    }

    #[test]
    fn duration_parts_examples() {
        let parts = |days, hours, minutes, seconds| DurationParts {
            days,
            hours,
            minutes,
            seconds,
        };
        assert_eq!(
            duration_parts(Duration::from_secs(3_725)),
            parts(0, 1, 2, 5)
        );
        assert_eq!(
            duration_parts(Duration::from_secs(90_061)),
            parts(1, 1, 1, 1)
        );
    }

    #[test]
    fn duration_parts_edges() {
        let zero = DurationParts {
            days: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        };
        assert_eq!(duration_parts(Duration::ZERO), zero);
        assert_eq!(
            duration_parts(Duration::from_secs(86_399)),
            DurationParts {
                days: 0,
                hours: 23,
                minutes: 59,
                seconds: 59,
            }
        );
        assert_eq!(
            duration_parts(Duration::from_hours(24)),
            DurationParts { days: 1, ..zero }
        );
        assert_eq!(
            duration_parts(Duration::from_millis(59_999)),
            DurationParts {
                seconds: 59,
                ..zero
            }
        );
    }

    #[test]
    fn duration_parts_keeps_many_days() {
        assert_eq!(
            duration_parts(Duration::from_hours(400 * 24)),
            DurationParts {
                days: 400,
                hours: 0,
                minutes: 0,
                seconds: 0,
            }
        );
    }
}
