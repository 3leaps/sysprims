//! Time utilities for sysprims
//!
//! Consolidates timestamp generation and provides drift-free scheduling.

use std::time::{Duration, Instant};

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{SysprimsError, SysprimsResult};

/// Minimum accepted tick interval (1 ms).
const MIN_TICK_INTERVAL: Duration = Duration::from_millis(1);

/// Get current timestamp in RFC 3339 / ISO 8601 format (UTC).
///
/// Falls back to Unix epoch if formatting fails (should never happen).
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Drift-free periodic scheduler.
///
/// `Tick` tracks the next deadline and sleeps only for the remaining time,
/// so accumulated processing time doesn't cause the interval to drift.
///
/// On overrun (work exceeds one interval), the deadline advances by the
/// smallest number of whole intervals that lands in the future, preserving
/// phase alignment with the original schedule.
///
/// # Example
///
/// ```no_run
/// use std::time::Duration;
/// use sysprims_core::time::Tick;
///
/// let mut tick = Tick::new(Duration::from_secs(5)).unwrap();
/// loop {
///     // do work …
///     tick.sleep_until_next();
/// }
/// ```
pub struct Tick {
    interval: Duration,
    next: Instant,
}

impl Tick {
    /// Create a new `Tick` that fires at `interval` from now.
    ///
    /// Returns `InvalidArgument` if `interval` is less than 1 ms.
    pub fn new(interval: Duration) -> SysprimsResult<Self> {
        if interval < MIN_TICK_INTERVAL {
            return Err(SysprimsError::invalid_argument(format!(
                "tick interval must be >= 1ms, got {:?}",
                interval
            )));
        }
        Ok(Self {
            interval,
            next: Instant::now() + interval,
        })
    }

    /// Sleep until the next tick deadline, then advance the deadline.
    ///
    /// If the deadline has already passed (work took longer than one interval),
    /// this returns immediately and advances by the smallest whole-interval
    /// multiple that lands in the future, preserving phase alignment.
    pub fn sleep_until_next(&mut self) {
        let now = Instant::now();
        if self.next > now {
            std::thread::sleep(self.next - now);
        }
        // Advance by whole-interval multiples to stay phase-aligned
        let now = Instant::now();
        if self.next <= now {
            let behind = now - self.next;
            let skipped = behind.as_nanos() / self.interval.as_nanos() + 1;
            self.next += self.interval * skipped as u32;
        } else {
            self.next += self.interval;
        }
    }

    /// Returns the interval duration.
    pub fn interval(&self) -> Duration {
        self.interval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_rfc3339_format() {
        let ts = now_rfc3339();
        // Should contain 'T' separator and 'Z' suffix (UTC)
        assert!(ts.contains('T'), "timestamp should contain T: {}", ts);
        assert!(ts.ends_with('Z'), "timestamp should end with Z: {}", ts);
        // Basic structure: YYYY-MM-DDTHH:MM:SS...Z
        assert!(
            ts.len() >= 20,
            "timestamp should be at least 20 chars: {}",
            ts
        );
        assert_eq!(&ts[4..5], "-", "should have dash at pos 4: {}", ts);
        assert_eq!(&ts[7..8], "-", "should have dash at pos 7: {}", ts);
    }

    #[test]
    fn test_tick_rejects_zero_interval() {
        let result = Tick::new(Duration::ZERO);
        assert!(result.is_err(), "zero interval should be rejected");
    }

    #[test]
    fn test_tick_rejects_sub_millisecond() {
        let result = Tick::new(Duration::from_micros(500));
        assert!(result.is_err(), "sub-ms interval should be rejected");
    }

    #[test]
    fn test_tick_accepts_one_millisecond() {
        let result = Tick::new(Duration::from_millis(1));
        assert!(result.is_ok(), "1ms interval should be accepted");
    }

    #[test]
    fn test_tick_sleeps_for_interval() {
        let mut tick = Tick::new(Duration::from_millis(50)).unwrap();
        let start = Instant::now();
        tick.sleep_until_next();
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(40),
            "should sleep roughly 50ms, got {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "should not overshoot by too much: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_tick_interval_accessor() {
        let tick = Tick::new(Duration::from_secs(10)).unwrap();
        assert_eq!(tick.interval(), Duration::from_secs(10));
    }
}
