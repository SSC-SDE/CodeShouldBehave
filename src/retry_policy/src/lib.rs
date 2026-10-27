//! Pure scheduling decisions. No clock, network, storage, randomness or credentials.
//! Workers keep lease verification and persistence in the scheduler transaction.

pub const MAX_ATTEMPTS: i64 = 5;
pub const MAX_DELAY_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureClass {
    Retryable,
    RateLimited { retry_after_ms: u64 },
    Terminal,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    RetryAt(i64),
    Failed,
    Cancelled,
}

/// Honor retry-after without retrying early. A wait beyond the automatic budget
/// stops the job for operator review. Stable per-job jitter avoids synchronized
/// retry waves while preserving reproducible decisions across worker restarts.
pub fn retry_decision(
    class: FailureClass,
    attempt: i64,
    job_id: u64,
    now_ms: i64,
) -> RetryDecision {
    if class == FailureClass::Cancelled {
        return RetryDecision::Cancelled;
    }
    if class == FailureClass::Terminal || !(1..MAX_ATTEMPTS).contains(&attempt) || now_ms < 0 {
        return RetryDecision::Failed;
    }
    let retry_after = match class {
        FailureClass::RateLimited { retry_after_ms } => retry_after_ms,
        _ => 0,
    };
    if retry_after > MAX_DELAY_MS {
        return RetryDecision::Failed;
    }
    let base = 1_000_u64 << (attempt - 1);
    let jitter = job_id.wrapping_mul(137).wrapping_add(attempt as u64 * 53) % 251;
    let delay = (base + jitter).max(retry_after).min(MAX_DELAY_MS);
    match now_ms.checked_add(delay as i64) {
        Some(available_at) => RetryDecision::RetryAt(available_at),
        None => RetryDecision::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_are_staggered_reproducible_and_bounded() {
        let first = retry_decision(FailureClass::Retryable, 1, 1, 100);
        assert_eq!(first, retry_decision(FailureClass::Retryable, 1, 1, 100));
        assert_ne!(first, retry_decision(FailureClass::Retryable, 1, 2, 100));
        for attempt in 1..MAX_ATTEMPTS {
            for job_id in [0, 1, 2, u64::MAX] {
                let RetryDecision::RetryAt(at) =
                    retry_decision(FailureClass::Retryable, attempt, job_id, 100)
                else {
                    panic!("eligible transient error must retry");
                };
                let base = 1_000_i64 << (attempt - 1);
                assert!((100 + base..=100 + base + 250).contains(&at));
            }
        }
    }

    #[test]
    fn rate_limits_never_retry_before_requested_time() {
        assert_eq!(
            retry_decision(
                FailureClass::RateLimited {
                    retry_after_ms: 30_000
                },
                1,
                1,
                100
            ),
            RetryDecision::RetryAt(30_100)
        );
        assert_eq!(
            retry_decision(
                FailureClass::RateLimited {
                    retry_after_ms: 60_000
                },
                1,
                1,
                100
            ),
            RetryDecision::RetryAt(60_100)
        );
        assert_eq!(
            retry_decision(
                FailureClass::RateLimited {
                    retry_after_ms: 60_001
                },
                1,
                1,
                100
            ),
            RetryDecision::Failed
        );
        assert_eq!(
            retry_decision(
                FailureClass::RateLimited {
                    retry_after_ms: u64::MAX
                },
                1,
                1,
                100
            ),
            RetryDecision::Failed
        );
        let RetryDecision::RetryAt(at) =
            retry_decision(FailureClass::RateLimited { retry_after_ms: 0 }, 1, 1, 100)
        else {
            panic!("zero retry-after still uses backoff")
        };
        assert!(at >= 1_100);
    }

    #[test]
    fn terminal_cancelled_exhausted_and_overflow_do_not_retry() {
        for attempt in [i64::MIN, 0, 5, i64::MAX] {
            assert_eq!(
                retry_decision(FailureClass::Retryable, attempt, 1, 100),
                RetryDecision::Failed
            );
        }
        assert_eq!(
            retry_decision(FailureClass::Terminal, 1, 1, 100),
            RetryDecision::Failed
        );
        assert_eq!(
            retry_decision(FailureClass::Cancelled, 5, 1, 100),
            RetryDecision::Cancelled
        );
        assert_eq!(
            retry_decision(FailureClass::Retryable, 1, 1, -1),
            RetryDecision::Failed
        );
        assert_eq!(
            retry_decision(FailureClass::Retryable, 1, 1, i64::MAX),
            RetryDecision::Failed
        );
    }
}
