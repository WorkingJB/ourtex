//! Token-bucket-ish sliding-window rate limiter per MCP.md §8.
//!
//! Keyed internally by the pre-authenticated token (there is one token
//! per stdio process, so the per-token dimension is trivially satisfied
//! today). Keeping the key in the API lets the cloud relay reuse the
//! same limiter when a single process eventually fans out to multiple
//! tokens.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Defaults from MCP.md §8: 60 requests per 10 seconds per token.
pub const DEFAULT_MAX: u32 = 60;
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(10);

pub struct RateLimiter {
    max: u32,
    window: Duration,
    timestamps: Mutex<VecDeque<Instant>>,
}

#[derive(Debug, Clone, Copy)]
pub struct Throttled {
    pub retry_after_ms: u64,
}

impl RateLimiter {
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            max,
            window,
            timestamps: Mutex::new(VecDeque::with_capacity(max as usize + 1)),
        }
    }

    pub fn default_stdio() -> Self {
        Self::new(DEFAULT_MAX, DEFAULT_WINDOW)
    }

    /// Record an attempt. Returns `Err(Throttled)` when the window is
    /// saturated; the retry hint is how long until the oldest in-window
    /// request ages out.
    pub fn check(&self) -> Result<(), Throttled> {
        self.check_at(Instant::now())
    }

    pub fn check_at(&self, now: Instant) -> Result<(), Throttled> {
        let mut ts = self.timestamps.lock().unwrap();
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        while let Some(&front) = ts.front() {
            if front < cutoff {
                ts.pop_front();
            } else {
                break;
            }
        }
        if ts.len() >= self.max as usize {
            let earliest = *ts.front().expect("non-empty since len >= max");
            let retry_at = earliest + self.window;
            let retry_after_ms = retry_at.saturating_duration_since(now).as_millis() as u64;
            return Err(Throttled { retry_after_ms });
        }
        ts.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_max_in_window() {
        let rl = RateLimiter::new(3, Duration::from_secs(10));
        let t = Instant::now();
        assert!(rl.check_at(t).is_ok());
        assert!(rl.check_at(t).is_ok());
        assert!(rl.check_at(t).is_ok());
        assert!(rl.check_at(t).is_err(), "4th in window must be rejected");
    }

    #[test]
    fn window_ages_out() {
        let rl = RateLimiter::new(2, Duration::from_secs(10));
        let t0 = Instant::now();
        rl.check_at(t0).unwrap();
        rl.check_at(t0).unwrap();
        // Same instant: full.
        assert!(rl.check_at(t0).is_err());
        // 11s later: oldest two have aged out.
        let t1 = t0 + Duration::from_secs(11);
        assert!(rl.check_at(t1).is_ok());
    }

    #[test]
    fn retry_hint_reflects_oldest_expiry() {
        let rl = RateLimiter::new(1, Duration::from_secs(10));
        let t0 = Instant::now();
        rl.check_at(t0).unwrap();
        let err = rl.check_at(t0 + Duration::from_secs(3)).unwrap_err();
        // Oldest is at t0, ages out at t0+10s; at t0+3s we wait ~7s.
        assert!(
            (6_500..=7_500).contains(&err.retry_after_ms),
            "got {}",
            err.retry_after_ms
        );
    }
}
