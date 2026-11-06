//! Wake-up plumbing for the render loop.
//!
//! The winit loop runs `ControlFlow::Wait`: it sleeps until a window event
//! arrives or Servo asks to be spun. Servo asks through an `EventLoopWaker`,
//! which its threads call whenever they queue work for the embedder — a
//! message for a delegate, a finished frame, the refresh driver's frame timer.
//! [`Waker`] turns that into an [`AppEvent::Wake`] on the winit loop, and the
//! loop answers by calling `Servo::spin_event_loop` on its own thread.
//!
//! Every wake-up sends its own event, as in Servo's `winit_minimal` example
//! and servoshell: a spin with nothing queued is cheap, and collapsing a burst
//! into one event measured no CPU difference.
//!
//! [`LoopStats`] counts what the loop did, for the debug log.

use std::time::{Duration, Instant};

use servo::EventLoopWaker;
use winit::event_loop::EventLoopProxy;

/// The winit loop's user event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    /// Servo has work for the embedder thread: spin its event loop.
    Wake,
}

/// Servo's handle for waking the winit loop, callable from any thread.
///
/// It holds only the loop's proxy (a channel sender) — no window, engine, or
/// `WebView` handle — so the clones Servo's threads keep can outlive the engine
/// and the loop without keeping either alive.
#[derive(Debug, Clone)]
pub struct Waker(EventLoopProxy<AppEvent>);

impl Waker {
    pub const fn new(proxy: EventLoopProxy<AppEvent>) -> Self {
        Self(proxy)
    }
}

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        // The send fails only once the loop is gone. Servo's threads can still
        // wake during and after shutdown, when nothing is left to spin, so the
        // error is dropped rather than logged.
        let _ = self.0.send_event(AppEvent::Wake);
    }
}

/// How long [`LoopStats`] accumulates before it reports.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// What the loop did over one reporting window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoopCounts {
    /// [`AppEvent::Wake`] events received.
    pub wakes: u32,
    /// Calls to `Servo::spin_event_loop`.
    pub spins: u32,
    /// Frames painted and presented.
    pub paints: u32,
}

/// Counts the loop's wake-ups, spins, and paints, for a debug log line that
/// shows whether the loop is idle. There is no timer behind it: a window is
/// reported by the first loop iteration at least [`REPORT_INTERVAL`] after it
/// opened, so an idle loop reports nothing and isn't woken to say so.
#[derive(Debug)]
pub struct LoopStats {
    window_start: Instant,
    counts: LoopCounts,
}

impl LoopStats {
    pub fn new(now: Instant) -> Self {
        Self {
            window_start: now,
            counts: LoopCounts::default(),
        }
    }

    pub const fn record_wake(&mut self) {
        self.counts.wakes = self.counts.wakes.saturating_add(1);
    }

    pub const fn record_spin(&mut self) {
        self.counts.spins = self.counts.spins.saturating_add(1);
    }

    pub const fn record_paint(&mut self) {
        self.counts.paints = self.counts.paints.saturating_add(1);
    }

    /// Returns the window's counts and its length once it is at least
    /// [`REPORT_INTERVAL`] old, and starts the next window. A window that
    /// counted nothing is restarted without a report. The length can be much
    /// longer than the interval: the loop was asleep for the rest.
    pub fn take_report(&mut self, now: Instant) -> Option<(LoopCounts, Duration)> {
        let elapsed = now.saturating_duration_since(self.window_start);
        if elapsed < REPORT_INTERVAL {
            return None;
        }
        self.window_start = now;
        let counts = std::mem::take(&mut self.counts);
        (counts != LoopCounts::default()).then_some((counts, elapsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_report_nothing_before_the_interval() {
        let start = Instant::now();
        let mut stats = LoopStats::new(start);
        stats.record_wake();
        assert_eq!(stats.take_report(start), None);
        assert_eq!(stats.take_report(start + REPORT_INTERVAL / 2), None);
    }

    #[test]
    fn stats_report_counts_and_elapsed_then_reset() {
        let start = Instant::now();
        let mut stats = LoopStats::new(start);
        stats.record_wake();
        stats.record_wake();
        stats.record_spin();
        stats.record_spin();
        stats.record_spin();
        stats.record_paint();
        let later = start + REPORT_INTERVAL;
        assert_eq!(
            stats.take_report(later),
            Some((
                LoopCounts {
                    wakes: 2,
                    spins: 3,
                    paints: 1
                },
                REPORT_INTERVAL
            ))
        );
        // the next window starts empty, at the report
        stats.record_spin();
        assert_eq!(stats.take_report(later + REPORT_INTERVAL / 2), None);
        assert_eq!(
            stats.take_report(later + REPORT_INTERVAL),
            Some((
                LoopCounts {
                    wakes: 0,
                    spins: 1,
                    paints: 0
                },
                REPORT_INTERVAL
            ))
        );
    }

    #[test]
    fn stats_window_covers_the_time_asleep() {
        let start = Instant::now();
        let mut stats = LoopStats::new(start);
        stats.record_wake();
        let asleep = REPORT_INTERVAL * 30;
        assert_eq!(
            stats.take_report(start + asleep),
            Some((
                LoopCounts {
                    wakes: 1,
                    spins: 0,
                    paints: 0
                },
                asleep
            ))
        );
    }

    #[test]
    fn stats_empty_window_restarts_without_a_report() {
        let start = Instant::now();
        let mut stats = LoopStats::new(start);
        let later = start + REPORT_INTERVAL * 5;
        assert_eq!(stats.take_report(later), None);
        // restarted at `later`, so the next window is measured from there
        stats.record_paint();
        assert_eq!(stats.take_report(later + REPORT_INTERVAL / 2), None);
        assert_eq!(
            stats.take_report(later + REPORT_INTERVAL),
            Some((
                LoopCounts {
                    wakes: 0,
                    spins: 0,
                    paints: 1
                },
                REPORT_INTERVAL
            ))
        );
    }

    #[test]
    fn stats_tolerate_a_clock_that_goes_backwards() {
        let earlier = Instant::now();
        let mut stats = LoopStats::new(earlier + REPORT_INTERVAL);
        stats.record_wake();
        assert_eq!(stats.take_report(earlier), None);
    }
}
