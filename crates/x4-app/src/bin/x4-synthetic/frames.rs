//! The scroll run: scrolls the torrent table by one row per frame, records the interval between
//! consecutive frames and logs a summary of them.
//!
//! Each step runs as a next-frame callback, which the window calls when the platform asks for a
//! frame, just before drawing it. The step records the moment, scrolls the table one row
//! further (which makes that frame draw) and asks for the next callback, so the intervals are the
//! pace at which the window draws while it scrolls.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, App, Entity, Window};
use x4_app::main_view::MainView;

/// One frame at 60 Hz, rounded up.
const ONE_FRAME: Duration = Duration::from_micros(16_700);
/// Two frames at 60 Hz, rounded down: an interval over it means at least one frame was missed.
const TWO_FRAMES: Duration = Duration::from_millis(33);

/// After `after`, scrolls the table of `view` in `window` by one row per frame for `lasting`,
/// logs the summary of the frame intervals and quits the app.
pub(crate) fn scroll_then_quit(
    window: AnyWindowHandle,
    view: Entity<MainView>,
    after: Duration,
    lasting: Duration,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(after).await;
        cx.update(|cx| {
            tracing::info!(
                seconds = lasting.as_secs(),
                "scrolling the table one row per frame"
            );
            let run = Rc::new(RefCell::new(ScrollRun {
                until: Instant::now() + lasting,
                row: 0,
                intervals: FrameIntervals::default(),
            }));
            if let Err(error) = window.update(cx, |_, window, _| next_frame(run, view, window)) {
                tracing::error!(%error, "the window is gone; nothing to scroll");
                cx.quit();
            }
        });
    })
    .detach();
}

/// A scroll in progress.
struct ScrollRun {
    /// When to stop.
    until: Instant,
    /// The row at the top of the table.
    row: usize,
    intervals: FrameIntervals,
}

fn next_frame(run: Rc<RefCell<ScrollRun>>, view: Entity<MainView>, window: &Window) {
    window.on_next_frame(move |window, cx| {
        if scroll_one_row(&run, &view, cx) {
            next_frame(run, view, window);
        } else {
            finish(&run.borrow().intervals, cx);
        }
    });
}

/// Records this frame and scrolls one row further, back to the top after the last row; `false`
/// once the run is over.
fn scroll_one_row(run: &RefCell<ScrollRun>, view: &Entity<MainView>, cx: &mut App) -> bool {
    let now = Instant::now();
    let mut run = run.borrow_mut();
    run.intervals.record(now);
    if now >= run.until {
        return false;
    }
    let table = view.read(cx).table().clone();
    let rows = table.read(cx).delegate().rows().len();
    run.row = (run.row + 1) % rows.max(1);
    let row = run.row;
    drop(run);
    table.update(cx, |table, cx| table.scroll_to_row(row, cx));
    true
}

fn finish(intervals: &FrameIntervals, cx: &mut App) {
    if let Some(summary) = intervals.summary() {
        tracing::info!(
            intervals = summary.count,
            p50_ms = %millis(summary.p50),
            p99_ms = %millis(summary.p99),
            max_ms = %millis(summary.max),
            over_16_7_ms = summary.over_one_frame,
            over_33_ms = summary.over_two_frames,
            "scrolled the table one row per frame"
        );
    } else {
        tracing::warn!("no two frames were drawn while scrolling");
    }
    cx.quit();
}

/// A duration in milliseconds with two decimals.
fn millis(duration: Duration) -> String {
    format!("{:.2}", duration.as_secs_f64() * 1000.0)
}

/// The intervals between the frames recorded so far.
#[derive(Debug, Default)]
struct FrameIntervals {
    last: Option<Instant>,
    intervals: Vec<Duration>,
}

impl FrameIntervals {
    fn record(&mut self, at: Instant) {
        if let Some(last) = self.last.replace(at) {
            self.intervals.push(at.saturating_duration_since(last));
        }
    }

    /// The summary, or `None` before two frames were recorded.
    fn summary(&self) -> Option<FrameSummary> {
        let mut sorted = self.intervals.clone();
        sorted.sort_unstable();
        // Nearest rank: the smallest interval with at least `percent` % of them at or below it.
        let percentile = |percent: usize| {
            let rank = (sorted.len() * percent).div_ceil(100);
            sorted.get(rank.saturating_sub(1)).copied()
        };
        Some(FrameSummary {
            count: sorted.len(),
            p50: percentile(50)?,
            p99: percentile(99)?,
            max: sorted.last().copied()?,
            over_one_frame: sorted.iter().filter(|&&each| each > ONE_FRAME).count(),
            over_two_frames: sorted.iter().filter(|&&each| each > TWO_FRAMES).count(),
        })
    }
}

/// The distribution of the frame intervals.
#[derive(Debug, PartialEq, Eq)]
struct FrameSummary {
    count: usize,
    p50: Duration,
    p99: Duration,
    max: Duration,
    /// Intervals longer than one frame at 60 Hz (16.7 ms).
    over_one_frame: usize,
    /// Intervals longer than 33 ms: a frame or more missed.
    over_two_frames: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn the_summary_has_nearest_rank_percentiles_and_counts_long_intervals() {
        let mut intervals = FrameIntervals::default();
        let mut at = Instant::now();
        intervals.record(at);
        for millis in [17, 10, 40, 16, 20] {
            at += ms(millis);
            intervals.record(at);
        }

        assert_eq!(
            intervals.summary(),
            Some(FrameSummary {
                count: 5,
                p50: ms(17),
                p99: ms(40),
                max: ms(40),
                over_one_frame: 3,
                over_two_frames: 1,
            })
        );
    }

    #[test]
    fn the_limits_are_one_and_two_frames_at_60_hz() {
        let mut intervals = FrameIntervals::default();
        let mut at = Instant::now();
        intervals.record(at);
        for each in [ONE_FRAME, TWO_FRAMES, Duration::from_micros(33_334)] {
            at += each;
            intervals.record(at);
        }

        let summary = intervals.summary().expect("three intervals");
        assert_eq!(
            (summary.over_one_frame, summary.over_two_frames),
            (2, 1),
            "an interval of exactly one or two frames is not over"
        );
    }

    #[test]
    fn there_is_no_summary_without_two_frames() {
        let mut intervals = FrameIntervals::default();
        assert_eq!(intervals.summary(), None);
        intervals.record(Instant::now());
        assert_eq!(intervals.summary(), None);
    }

    #[test]
    fn durations_are_logged_in_milliseconds() {
        assert_eq!(millis(Duration::from_micros(16_667)), "16.67");
        assert_eq!(millis(ms(2)), "2.00");
    }
}
