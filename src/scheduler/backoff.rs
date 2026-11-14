//! Pure retry timing; the scheduler retains attempt limits and lease ownership.

pub(super) fn available_at(now_ms: i64, attempt: i64) -> i64 {
    let exponent = u32::try_from(attempt.saturating_sub(1))
        .unwrap_or(31)
        .min(6);
    let delay = 1_000_i64.saturating_mul(1_i64 << exponent).min(60_000);
    now_ms.saturating_add(delay)
}

#[cfg(test)]
mod tests {
    use super::available_at;

    #[test]
    fn retry_times_preserve_existing_exponential_schedule() {
        for (attempt, delay) in [(1, 1_000), (2, 2_000), (3, 4_000), (4, 8_000)] {
            assert_eq!(available_at(10_000, attempt), 10_000 + delay);
        }
    }

    #[test]
    fn delay_is_capped_and_timestamp_cannot_wrap() {
        assert_eq!(available_at(10_000, 100), 70_000);
        assert_eq!(available_at(i64::MAX - 500, 1), i64::MAX);
    }
}
