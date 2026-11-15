use std::time::Duration;

use super::error::NetworkKind;

/// Exponential backoff with jitter. Tests use [`RetryPolicy::immediate`].
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 4,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Same retry count, no waiting.
    pub fn immediate() -> Self {
        Self { max_retries: 4, base_delay: Duration::ZERO, max_delay: Duration::ZERO, jitter: false }
    }

    /// Delay before retry number `retry` (1-based). A server `Retry-After` wins over
    /// the computed backoff; both are capped at `max_delay`.
    pub(crate) fn delay(&self, retry: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(ra) = retry_after {
            return ra.min(self.max_delay);
        }
        let factor = 1u32 << retry.saturating_sub(1).min(20);
        let exp = self.base_delay.saturating_mul(factor).min(self.max_delay);
        if !self.jitter || exp.is_zero() {
            return exp;
        }
        // Equal jitter (50%..100% of the exponential delay), integer math only: this
        // crate denies float arithmetic.
        let mut b = [0u8; 4];
        let r = if getrandom::fill(&mut b).is_ok() { u32::from_le_bytes(b) } else { u32::MAX };
        let half = exp / 2;
        let extra = half.as_nanos() * u128::from(r) / u128::from(u32::MAX);
        half + Duration::from_nanos(u64::try_from(extra).unwrap_or(u64::MAX))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Idempotency {
    Idempotent,
    /// `POST /auth` and `POST /sessions`: the code is single-use.
    NonIdempotent,
}

pub(crate) fn status_retryable(status: u16, idem: Idempotency) -> bool {
    match status {
        429 => true,
        500 | 502 | 503 | 504 => idem == Idempotency::Idempotent,
        _ => false,
    }
}

pub(crate) fn network_retryable(kind: NetworkKind, idem: Idempotency) -> bool {
    match kind {
        NetworkKind::Connect => true,
        NetworkKind::Timeout => idem == Idempotency::Idempotent,
        NetworkKind::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        let p = RetryPolicy { jitter: false, ..RetryPolicy::default() };
        assert_eq!(p.delay(1, None), Duration::from_secs(1));
        assert_eq!(p.delay(3, None), Duration::from_secs(4));
        assert_eq!(p.delay(10, None), Duration::from_secs(30));
    }

    #[test]
    fn retry_after_wins_but_is_capped() {
        let p = RetryPolicy::default();
        assert_eq!(p.delay(1, Some(Duration::from_secs(5))), Duration::from_secs(5));
        assert_eq!(p.delay(1, Some(Duration::from_secs(999))), Duration::from_secs(30));
    }

    #[test]
    fn jitter_stays_within_bounds() {
        let p = RetryPolicy::default();
        for _ in 0..50 {
            let d = p.delay(2, None);
            assert!(d >= Duration::from_secs(1) && d <= Duration::from_secs(2));
        }
    }

    #[test]
    fn non_idempotent_only_retries_429_and_connect() {
        use Idempotency::*;
        assert!(status_retryable(429, NonIdempotent));
        assert!(!status_retryable(500, NonIdempotent));
        assert!(status_retryable(503, Idempotent));
        assert!(!status_retryable(400, Idempotent));
        assert!(network_retryable(NetworkKind::Connect, NonIdempotent));
        assert!(!network_retryable(NetworkKind::Timeout, NonIdempotent));
        assert!(network_retryable(NetworkKind::Timeout, Idempotent));
    }
}
