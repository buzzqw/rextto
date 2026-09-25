//! Escalating provider backoff, ported from Sonarr's `EscalationBackOff`.
//!
//! A source (RSS feed, indexer, web engine) that keeps failing is temporarily
//! disabled for a growing interval instead of being retried on every cycle.
//! The first success steps the level back down, so recovery is automatic.

/// Disable intervals in seconds, indexed by escalation level. Level 0 means
/// "not disabled"; the last entry is the cap.
pub const PERIODS: [i64; 10] = [0, 60, 300, 900, 1800, 3600, 10800, 21600, 43200, 86400];

pub fn max_level() -> i64 {
    (PERIODS.len() - 1) as i64
}

/// Seconds a provider at `level` stays disabled.
pub fn period_secs(level: i64) -> i64 {
    let level = level.clamp(0, max_level());
    PERIODS[level as usize]
}

/// Next escalation level after a failure. A failure only escalates when the
/// previous one is older than the current period; rapid consecutive failures
/// keep the same level (Sonarr's grace behaviour) so a burst of timeouts does
/// not jump straight to a day.
pub fn next_level(level: i64, seconds_since_previous_failure: Option<i64>) -> i64 {
    let level = level.clamp(0, max_level());
    if level == 0 {
        return 1;
    }
    match seconds_since_previous_failure {
        Some(seconds) if seconds < period_secs(level) => level,
        _ => (level + 1).clamp(1, max_level()),
    }
}

/// Level after a success: step down one notch and clear the disabled window.
pub fn success_level(level: i64) -> i64 {
    (level - 1).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_failure_starts_at_one_and_sixty_seconds() {
        assert_eq!(next_level(0, None), 1);
        assert_eq!(period_secs(1), 60);
    }

    #[test]
    fn repeated_failures_escalate_but_bursts_do_not() {
        // Escalates when the previous failure is older than the current period.
        assert_eq!(next_level(1, Some(120)), 2);
        // A rapid retry keeps the same level.
        assert_eq!(next_level(1, Some(10)), 1);
        assert_eq!(period_secs(2), 300);
        assert_eq!(next_level(max_level(), Some(999_999)), max_level());
    }

    #[test]
    fn success_steps_down_and_reaches_zero() {
        assert_eq!(success_level(3), 2);
        assert_eq!(success_level(1), 0);
        assert_eq!(success_level(0), 0);
    }
}
