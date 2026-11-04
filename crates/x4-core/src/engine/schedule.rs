//! When the engine sends what: the timers of the polls, the tick and the reconnect backoff.

use std::time::{Duration, Instant};

use super::{Engine, Request, Visibility};

/// The wait between retries when the configured backoff list is empty.
const EMPTY_BACKOFF: Duration = Duration::from_secs(30);

/// When one kind of poll was last sent, and whether it is wanted before its interval is up.
///
/// A timer that has never been sent is due at once, so resetting a timer to its default makes
/// the poll due now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Timer {
    last_sent: Option<Instant>,
    forced: bool,
}

impl Timer {
    /// When the poll is due with `interval` between sends: `now` when it is due at once, `None`
    /// when the interval reaches past what an `Instant` can hold (never).
    pub(super) fn due_at(&self, now: Instant, interval: Duration) -> Option<Instant> {
        match self.last_sent {
            Some(last_sent) if !self.forced => last_sent.checked_add(interval),
            _ => Some(now),
        }
    }

    /// Whether the poll is due at `now`.
    pub(super) fn is_due(&self, now: Instant, interval: Duration) -> bool {
        self.due_at(now, interval).is_some_and(|at| at <= now)
    }

    /// Whether the poll comes due before `before`.
    pub(super) fn is_due_before(&self, now: Instant, interval: Duration, before: Instant) -> bool {
        self.due_at(now, interval).is_some_and(|at| at < before)
    }

    /// The poll was handed out at `now`; the next one is an interval later.
    pub(super) fn sent(&mut self, now: Instant) {
        self.last_sent = Some(now);
        self.forced = false;
    }

    /// Makes the poll due now, without waiting for its interval.
    pub(super) fn force(&mut self) {
        self.forced = true;
    }

    /// The answer to the poll came at `at`: the next one is due an interval after the answer
    /// if that is later than an interval after the send, so a slow daemon rests a full interval.
    /// A forced poll stays forced.
    pub(super) fn answered(&mut self, at: Instant) {
        if let Some(last_sent) = self.last_sent {
            self.last_sent = Some(last_sent.max(at));
        }
    }
}

/// The wait after `attempt` failed tries in a row: the `attempt`-th entry of `backoff`, then its
/// last entry for every further try.
fn backoff_delay(backoff: &[Duration], attempt: u32) -> Duration {
    let index = usize::try_from(attempt.saturating_sub(1)).unwrap_or(usize::MAX);
    backoff
        .get(index)
        .or(backoff.last())
        .copied()
        .unwrap_or(EMPTY_BACKOFF)
}

impl Engine {
    /// The time between ticks, which depends on whether the window is visible.
    pub(super) fn tick_interval(&self) -> Duration {
        match self.visibility {
            Visibility::Visible => self.config.visible_interval,
            Visibility::Hidden => self.config.hidden_interval,
        }
    }

    /// The attempt the next failure to use the daemon would be.
    pub(super) fn next_attempt(&self) -> u32 {
        self.attempts.saturating_add(1)
    }

    /// The wait before checking the server again after `attempt` failed tries in a row.
    pub(super) fn retry_delay(&self, attempt: u32) -> Duration {
        backoff_delay(&self.config.backoff, attempt)
    }

    /// Hands out the polls due at `now`, in a fixed order, and schedules each one's next turn.
    pub(super) fn due_polls(&mut self, now: Instant, due: &mut Vec<Request>) {
        if self.session_timer.is_due(now, self.config.session_interval) {
            due.push(Request::SessionGet);
            self.session_timer.sent(now);
        }
        let tick_interval = self.tick_interval();
        let tick = self.tick_timer.is_due(now, tick_interval);
        // A full list that would come due before the next tick goes with this one, in place of
        // its recently active torrents: wakeups are a little late, so the two seldom fall due at
        // the same instant.
        let full_interval = self.config.full_refresh_interval;
        let full_list = self.full_list_timer.is_due(now, full_interval)
            || (tick
                && now.checked_add(tick_interval).is_some_and(|next_tick| {
                    self.full_list_timer
                        .is_due_before(now, full_interval, next_tick)
                }));
        if full_list {
            due.push(Request::FullList);
            self.full_list_timer.sent(now);
        }
        if tick {
            // A full list holds the recently active torrents too.
            if !full_list {
                due.push(Request::RecentlyActive);
            }
            due.push(Request::SessionStats);
            self.tick_timer.sent(now);
            self.tick_batch = Some(self.batch);
        }
        if let Some(dir) = &self.download_dir
            && self
                .free_space_timer
                .is_due(now, self.config.free_space_interval)
        {
            due.push(Request::FreeSpace(dir.clone()));
            self.free_space_timer.sent(now);
        }
        if let Some(read) = self.due_details(now, full_list) {
            due.push(read);
        }
        // After the details: a full read that goes out now lets the tab's read go with it.
        if let Some(read) = self.due_tab_read(now) {
            due.push(read);
        }
    }

    /// Measures the next turn of `poll` from its answer, which came at `now`.
    pub(super) fn answered_at(&mut self, poll: &Request, now: Instant) {
        let timer = match poll {
            Request::SessionGet => &mut self.session_timer,
            Request::FullList => &mut self.full_list_timer,
            Request::FreeSpace(_) => &mut self.free_space_timer,
            Request::RecentlyActive | Request::SessionStats => &mut self.tick_timer,
            Request::Details { kind, .. } => {
                self.details_answered_at(*kind, now);
                return;
            }
            Request::CheckServer
            | Request::Action(..)
            | Request::MagnetLinks(..)
            | Request::FolderFreeSpace(_) => return,
        };
        timer.answered(now);
    }

    /// When the next poll is due while connected.
    pub(super) fn next_poll(&self, now: Instant) -> Option<Instant> {
        let free_space = self.download_dir.as_ref().and_then(|_| {
            self.free_space_timer
                .due_at(now, self.config.free_space_interval)
        });
        let details = self.next_details(now);
        let tab = self.next_tab_read(now);
        [
            self.session_timer.due_at(now, self.config.session_interval),
            self.full_list_timer
                .due_at(now, self.config.full_refresh_interval),
            self.tick_timer.due_at(now, self.tick_interval()),
            free_space,
            details,
            tab,
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn a_timer_never_sent_is_due_at_once() {
        let now = Instant::now();

        assert_eq!(Timer::default().due_at(now, secs(60)), Some(now));
        assert!(Timer::default().is_due(now, secs(60)));
    }

    #[test]
    fn a_sent_timer_is_due_one_interval_later() {
        let base = Instant::now();
        let mut timer = Timer::default();

        timer.sent(base);

        assert_eq!(timer.due_at(base, secs(30)), Some(base + secs(30)));
        assert!(!timer.is_due(base + secs(29), secs(30)));
        assert!(timer.is_due(base + secs(30), secs(30)));
        // The interval is read when asked, so a new one applies to the last send at once.
        assert!(timer.is_due(base + secs(10), secs(10)));
    }

    #[test]
    fn a_forced_timer_is_due_until_it_is_sent() {
        let base = Instant::now();
        let mut timer = Timer::default();
        timer.sent(base);

        timer.force();
        assert!(timer.is_due(base, secs(30)));

        timer.sent(base + secs(1));
        assert!(!timer.is_due(base + secs(2), secs(30)));
    }

    #[test]
    fn is_due_before_excludes_the_instant_itself() {
        let base = Instant::now();
        let mut timer = Timer::default();
        timer.sent(base);

        assert!(timer.is_due_before(base, secs(30), base + secs(31)));
        assert!(!timer.is_due_before(base, secs(30), base + secs(30)));
        timer.force();
        assert!(timer.is_due_before(base, secs(30), base + secs(1)));
    }

    #[test]
    fn an_answer_later_than_the_interval_moves_the_next_turn() {
        let base = Instant::now();
        let mut timer = Timer::default();
        timer.sent(base);

        // The next turn is an interval after the latest answer, never sooner.
        timer.answered(base + secs(1));
        assert_eq!(timer.due_at(base, secs(10)), Some(base + secs(11)));
        timer.answered(base + secs(3));
        assert_eq!(timer.due_at(base, secs(10)), Some(base + secs(13)));
        // Never earlier than an interval after the last answer.
        timer.answered(base + secs(2));
        assert_eq!(timer.due_at(base, secs(10)), Some(base + secs(13)));
        // A forced poll stays due at once.
        timer.force();
        timer.answered(base + secs(4));
        assert_eq!(timer.due_at(base, secs(10)), Some(base));
        // A timer never sent is not moved by an answer.
        let mut fresh = Timer::default();
        fresh.answered(base + secs(4));
        assert_eq!(fresh.due_at(base, secs(10)), Some(base));
    }

    #[test]
    fn an_interval_too_long_for_an_instant_is_never_due() {
        let base = Instant::now();
        let mut timer = Timer::default();
        timer.sent(base);

        assert_eq!(timer.due_at(base, Duration::MAX), None);
        assert!(!timer.is_due(base, Duration::MAX));
    }

    #[test]
    fn backoff_follows_the_list_and_stays_at_its_last_step() {
        let backoff = [secs(1), secs(2), secs(4)];

        let delays: Vec<Duration> = (1..=6)
            .map(|attempt| backoff_delay(&backoff, attempt))
            .collect();

        assert_eq!(
            delays,
            [secs(1), secs(2), secs(4), secs(4), secs(4), secs(4)]
        );
        assert_eq!(backoff_delay(&backoff, u32::MAX), secs(4));
        // Attempt 0 does not happen; it waits like the first.
        assert_eq!(backoff_delay(&backoff, 0), secs(1));
    }

    #[test]
    fn an_empty_backoff_waits_thirty_seconds() {
        assert_eq!(backoff_delay(&[], 1), secs(30));
        assert_eq!(backoff_delay(&[], 9), secs(30));
    }
}
