//! Failure classification and backoff decisions for queued work.
//!
//! Keeping timing policy outside the database scheduler makes retry behavior
//! reviewable independently from lease and transaction handling.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryClass {
    Retryable,
    Terminal,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    Retry { available_at_ms: i64 },
    Failed,
    Cancelled,
}

const MAX_ATTEMPTS: i64 = 5;
const INITIAL_DELAY_MS: i64 = 1_000;
const MAX_DELAY_MS: i64 = 60_000;

pub fn decide(class: RetryClass, attempt: i64, now_ms: i64) -> RetryDecision {
    match class {
        RetryClass::Retryable if attempt < MAX_ATTEMPTS => {
            let exponent = attempt.saturating_sub(1).clamp(0, 6) as u32;
            let delay = INITIAL_DELAY_MS
                .saturating_mul(1_i64 << exponent)
                .min(MAX_DELAY_MS);
            RetryDecision::Retry {
                available_at_ms: now_ms.saturating_add(delay),
            }
        }
        RetryClass::Retryable | RetryClass::Terminal => RetryDecision::Failed,
        RetryClass::Cancelled => RetryDecision::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::{RetryClass, RetryDecision, decide};

    #[test]
    fn increases_backoff_and_caps_delay() {
        assert_eq!(
            decide(RetryClass::Retryable, 1, 10_000),
            RetryDecision::Retry {
                available_at_ms: 11_000
            }
        );
        assert_eq!(
            decide(RetryClass::Retryable, 5, 10_000),
            RetryDecision::Failed
        );
        assert_eq!(
            decide(RetryClass::Retryable, 4, 10_000),
            RetryDecision::Retry {
                available_at_ms: 18_000
            }
        );
    }

    #[test]
    fn terminal_and_cancelled_failures_do_not_retry() {
        assert_eq!(decide(RetryClass::Terminal, 1, 0), RetryDecision::Failed);
        assert_eq!(
            decide(RetryClass::Cancelled, 1, 0),
            RetryDecision::Cancelled
        );
    }
}
