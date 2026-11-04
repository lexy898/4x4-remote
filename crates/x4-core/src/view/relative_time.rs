//! How long ago or from now a time is, in the units a label wants.
//!
//! The Trackers tab says "3 min ago" for the last announce and "in 27 min" for the next one. The
//! time `now` is passed in, so the result depends on nothing but the arguments.

use std::time::{Duration, SystemTime};

use super::eta::CompactDuration;

/// Which side of now a time is expected on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeDirection {
    /// Something that happened, such as the last announce.
    Past,
    /// Something that is due, such as the next announce.
    Future,
}

/// How far a time is from now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeTime {
    /// Now: less than a second away, or on the wrong side of now.
    Now,
    /// So long ago.
    Ago(CompactDuration),
    /// In so long.
    In(CompactDuration),
}

/// How far `at` is from `now`, on the side `direction` expects.
///
/// Up to a minute the seconds are kept; above it the time is rounded down to whole minutes, so a
/// countdown does not change every second and "3 h 2 m 50 s" is "3 h 2 m". A time on the other
/// side of `now` than `direction` expects, which a clock that runs ahead of ours or a read that
/// is a little old makes of the last announce or the next one, is [`RelativeTime::Now`], and so is
/// a distance under a second.
#[must_use]
pub fn relative_time(at: SystemTime, now: SystemTime, direction: TimeDirection) -> RelativeTime {
    let distance = match direction {
        TimeDirection::Past => now.duration_since(at),
        TimeDirection::Future => at.duration_since(now),
    };
    let seconds = distance.map_or(0, |distance| distance.as_secs());
    if seconds == 0 {
        return RelativeTime::Now;
    }
    let seconds = if seconds > 60 {
        seconds - seconds % 60
    } else {
        seconds
    };
    let compact = CompactDuration::from(Duration::from_secs(seconds));
    match direction {
        TimeDirection::Past => RelativeTime::Ago(compact),
        TimeDirection::Future => RelativeTime::In(compact),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_793_784_667)
    }

    fn ago(seconds: u64) -> RelativeTime {
        relative_time(
            now() - Duration::from_secs(seconds),
            now(),
            TimeDirection::Past,
        )
    }

    fn within(seconds: u64) -> RelativeTime {
        relative_time(
            now() + Duration::from_secs(seconds),
            now(),
            TimeDirection::Future,
        )
    }

    fn compact(seconds: u64) -> CompactDuration {
        CompactDuration::from(Duration::from_secs(seconds))
    }

    #[test]
    fn the_present_and_less_than_a_second_are_now() {
        assert_eq!(ago(0), RelativeTime::Now);
        assert_eq!(within(0), RelativeTime::Now);
        let a_moment = Duration::from_millis(900);
        assert_eq!(
            relative_time(now() - a_moment, now(), TimeDirection::Past),
            RelativeTime::Now
        );
        assert_eq!(
            relative_time(now() + a_moment, now(), TimeDirection::Future),
            RelativeTime::Now
        );
    }

    #[test]
    fn up_to_a_minute_the_seconds_are_kept() {
        assert_eq!(ago(1), RelativeTime::Ago(compact(1)));
        assert_eq!(ago(59), RelativeTime::Ago(compact(59)));
        assert_eq!(within(1), RelativeTime::In(compact(1)));
        assert_eq!(within(59), RelativeTime::In(compact(59)));
    }

    #[test]
    fn above_a_minute_the_seconds_are_rounded_down_to_whole_minutes() {
        for (seconds, rounded) in [
            (60, 60),
            (61, 60),
            (119, 60),
            (120, 120),
            // 3 h 2 m 50 s is shown as 3 h 2 m.
            (3 * 3_600 + 2 * 60 + 50, 3 * 3_600 + 2 * 60),
            // 2 d 5 h 59 m 59 s is 2 d 5 h.
            (
                2 * 86_400 + 5 * 3_600 + 59 * 60 + 59,
                2 * 86_400 + 5 * 3_600 + 59 * 60,
            ),
        ] {
            assert_eq!(
                ago(seconds),
                RelativeTime::Ago(compact(rounded)),
                "{seconds}"
            );
            assert_eq!(
                within(seconds),
                RelativeTime::In(compact(rounded)),
                "{seconds}"
            );
        }
        assert_eq!(
            ago(3 * 3_600 + 2 * 60 + 50),
            RelativeTime::Ago(CompactDuration {
                major: (3, crate::view::TimeUnit::Hours),
                minor: Some((2, crate::view::TimeUnit::Minutes)),
            })
        );
    }

    #[test]
    fn a_time_on_the_wrong_side_of_now_is_now() {
        // The daemon's clock ahead of ours, or a read that is a little stale.
        let later = now() + Duration::from_secs(90);
        let earlier = now() - Duration::from_secs(90);

        assert_eq!(
            relative_time(later, now(), TimeDirection::Past),
            RelativeTime::Now
        );
        assert_eq!(
            relative_time(earlier, now(), TimeDirection::Future),
            RelativeTime::Now
        );
    }

    #[test]
    fn a_time_at_the_edge_of_the_clock_does_not_panic() {
        assert!(matches!(
            relative_time(UNIX_EPOCH, now(), TimeDirection::Past),
            RelativeTime::Ago(_)
        ));
        assert!(matches!(
            relative_time(now(), UNIX_EPOCH, TimeDirection::Future),
            RelativeTime::In(_)
        ));
    }
}
