//! The pure polling schedule (FR-PRV-19). RGN's job runner owns the loop (R14).

use std::time::Duration;

use crate::ProviderError;

/// Delay before the next `status` poll.
///
/// 2 s for the first 30 s, then +2 s per poll up to 10 s. `rate_limited` waits at least its
/// `retry_after`. `offline` waits 10 s. Polling never resubmits (OQ-PRV-05).
pub fn poll_delay(elapsed: Duration, last_error: Option<&ProviderError>) -> Duration {
    const BASE: u64 = 2;
    const MAX: u64 = 10;
    let schedule = if elapsed <= Duration::from_secs(30) {
        Duration::from_secs(BASE)
    } else {
        // Polls after 30 s wait 4, 6, 8, then 10 s; they happen at +0, +4, +10, +18 s.
        let after = elapsed.as_secs_f64() - 30.0;
        let polls_before = [4.0, 10.0, 18.0].iter().filter(|t| after >= **t).count() as u64;
        Duration::from_secs((BASE * (polls_before + 2)).min(MAX))
    };
    match last_error {
        Some(ProviderError::RateLimited { retry_after_ms }) => {
            schedule.max(Duration::from_millis(*retry_after_ms))
        }
        Some(ProviderError::Offline) => Duration::from_secs(MAX),
        _ => schedule,
    }
}

/// True once a job has run long enough for the "still running" notice (180 s).
pub fn long_running(elapsed: Duration) -> bool {
    elapsed >= Duration::from_secs(180)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prv_19_two_seconds_for_the_first_30_seconds() {
        for s in [0, 1, 15, 30] {
            assert_eq!(
                poll_delay(Duration::from_secs(s), None),
                Duration::from_secs(2)
            );
        }
    }

    #[test]
    fn fr_prv_19_backs_off_by_two_seconds_up_to_ten() {
        let d = |s| poll_delay(Duration::from_secs(s), None).as_secs();
        assert_eq!(d(31), 4);
        assert_eq!(d(34), 6);
        assert_eq!(d(40), 8);
        assert_eq!(d(48), 10);
        assert_eq!(d(600), 10);
        let mut last = 0;
        for s in 30..200 {
            assert!(d(s) >= last && d(s) <= 10);
            last = d(s);
        }
    }

    #[test]
    fn fr_prv_19_rate_limit_and_offline() {
        let rl = ProviderError::RateLimited {
            retry_after_ms: 7_000,
        };
        assert_eq!(
            poll_delay(Duration::ZERO, Some(&rl)),
            Duration::from_secs(7)
        );
        let short = ProviderError::RateLimited { retry_after_ms: 10 };
        assert_eq!(
            poll_delay(Duration::ZERO, Some(&short)),
            Duration::from_secs(2)
        );
        assert_eq!(
            poll_delay(Duration::ZERO, Some(&ProviderError::Offline)),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn fr_prv_19_long_running_from_180_seconds() {
        assert!(!long_running(Duration::from_secs(179)));
        assert!(long_running(Duration::from_secs(180)));
    }
}
