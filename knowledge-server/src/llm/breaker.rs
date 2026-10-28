use std::sync::Mutex;
use std::time::{Duration, Instant};

const FAILURE_THRESHOLD: u32 = 3;
const COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    RateLimited,
    Transient,
    Permanent,
}

impl Failure {
    pub fn of(err: &anyhow::Error) -> Self {
        let message = format!("{err:#}").to_lowercase();
        if message.contains("429") || message.contains("resource_exhausted") {
            return Failure::RateLimited;
        }
        let transient = [
            "500",
            "502",
            "503",
            "504",
            "529",
            "overloaded",
            "unavailable",
            "timed out",
            "timeout",
            "connection",
            "connect error",
            "broken pipe",
            "reset by peer",
        ];
        if transient.iter().any(|needle| message.contains(needle)) {
            return Failure::Transient;
        }
        Failure::Permanent
    }

    pub fn moves_on(self) -> bool {
        self != Failure::Permanent
    }
}

#[derive(Default)]
struct State {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

#[derive(Default)]
pub struct CircuitBreaker {
    state: Mutex<State>,
}

impl CircuitBreaker {
    pub fn is_open(&self) -> bool {
        self.is_open_at(Instant::now())
    }

    fn is_open_at(&self, now: Instant) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.open_until.is_some_and(|until| now < until)
    }

    pub fn succeeded(&self) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = State::default();
    }

    pub fn failed(&self, failure: Failure) {
        self.failed_at(failure, Instant::now());
    }

    fn failed_at(&self, failure: Failure, now: Instant) {
        if failure == Failure::Permanent {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.consecutive_failures += 1;
        if state.consecutive_failures >= FAILURE_THRESHOLD {
            state.open_until = Some(now + COOLDOWN);
            state.consecutive_failures = FAILURE_THRESHOLD - 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_classified_by_their_message() {
        assert_eq!(
            Failure::of(&anyhow::anyhow!("HTTP 429 Too Many Requests")),
            Failure::RateLimited
        );
        assert_eq!(
            Failure::of(&anyhow::anyhow!("API error 503: overloaded")),
            Failure::Transient
        );
        assert_eq!(
            Failure::of(&anyhow::anyhow!("operation timed out")),
            Failure::Transient
        );
        assert_eq!(
            Failure::of(&anyhow::anyhow!("400 Bad Request: invalid tool schema")),
            Failure::Permanent
        );
        assert!(!Failure::Permanent.moves_on());
        assert!(Failure::Transient.moves_on());
    }

    #[test]
    fn the_breaker_opens_after_consecutive_failures_and_closes_after_the_cooldown() {
        let breaker = CircuitBreaker::default();
        let now = Instant::now();
        breaker.failed_at(Failure::Transient, now);
        breaker.failed_at(Failure::RateLimited, now);
        assert!(!breaker.is_open_at(now));
        breaker.failed_at(Failure::Transient, now);
        assert!(breaker.is_open_at(now));
        assert!(!breaker.is_open_at(now + COOLDOWN));
    }

    #[test]
    fn a_failure_after_the_cooldown_reopens_at_once() {
        let breaker = CircuitBreaker::default();
        let now = Instant::now();
        for _ in 0..FAILURE_THRESHOLD {
            breaker.failed_at(Failure::Transient, now);
        }
        let later = now + COOLDOWN;
        breaker.failed_at(Failure::Transient, later);
        assert!(breaker.is_open_at(later));
    }

    #[test]
    fn success_and_permanent_errors_do_not_trip_the_breaker() {
        let breaker = CircuitBreaker::default();
        let now = Instant::now();
        breaker.failed_at(Failure::Transient, now);
        breaker.failed_at(Failure::Transient, now);
        breaker.succeeded();
        breaker.failed_at(Failure::Transient, now);
        breaker.failed_at(Failure::Permanent, now);
        breaker.failed_at(Failure::Permanent, now);
        assert!(!breaker.is_open_at(now));
    }
}
