//! The clock seam (AGENTS §9 injected seams) and FR-PRM-36's turn budgets.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A monotonic clock. `now` is the time since an arbitrary origin.
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration);
}

/// The real clock.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// A clock that only moves when told to; `sleep` advances it at once (tests, fixtures).
#[derive(Debug, Clone, Default)]
pub struct FakeClock {
    now: Arc<Mutex<Duration>>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&self, d: Duration) {
        if let Ok(mut n) = self.now.lock() {
            *n += d;
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.now.lock().map(|n| *n).unwrap_or_default()
    }

    fn sleep(&self, d: Duration) {
        self.advance(d);
    }
}

/// FR-PRM-36: the first stream event within `first_event` of a send, at most `gap` between
/// events, at most `total` per turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnBudget {
    pub first_event: Duration,
    pub gap: Duration,
    pub total: Duration,
}

impl Default for TurnBudget {
    fn default() -> Self {
        Self {
            first_event: Duration::from_secs(60),
            gap: Duration::from_secs(90),
            total: Duration::from_secs(300),
        }
    }
}

/// One turn's timer: started at the send, moved on by every stdout line.
#[derive(Debug, Clone, Copy)]
pub struct TurnTimer {
    budget: TurnBudget,
    started: Duration,
    last_event: Option<Duration>,
}

impl TurnTimer {
    pub fn start(budget: TurnBudget, now: Duration) -> Self {
        Self {
            budget,
            started: now,
            last_event: None,
        }
    }

    pub fn event(&mut self, now: Duration) {
        self.last_event = Some(now);
    }

    /// How long to wait for the next line, or `None` once a budget is spent.
    pub fn remaining(&self, now: Duration) -> Option<Duration> {
        let event_deadline = match self.last_event {
            Some(t) => t + self.budget.gap,
            None => self.started + self.budget.first_event,
        };
        let deadline = event_deadline.min(self.started + self.budget.total);
        deadline.checked_sub(now).filter(|d| !d.is_zero())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn fr_prm_36_budgets_first_event_gap_and_total() {
        let b = TurnBudget::default();
        let mut t = TurnTimer::start(b, S(10));
        assert_eq!(t.remaining(S(10)), Some(S(60)));
        assert_eq!(t.remaining(S(70)), None);
        t.event(S(69));
        assert_eq!(t.remaining(S(69)), Some(S(90)));
        // Events every 80 s never trip the gap, but the 300 s total does.
        for k in 1..4 {
            t.event(S(69 + 80 * k));
        }
        assert_eq!(t.remaining(S(309)), Some(S(1)));
        assert_eq!(t.remaining(S(310)), None);
    }

    #[test]
    fn fake_clock_sleep_advances() {
        let c = FakeClock::new();
        c.sleep(S(5));
        let c2 = c.clone();
        c2.advance(S(1));
        assert_eq!(c.now(), S(6));
    }
}
