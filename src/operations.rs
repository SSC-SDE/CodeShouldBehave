use crate::{QueueStats, Reconciliation};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationalLimits {
    pub maximum_queued_jobs: usize,
    pub maximum_running_jobs: usize,
    pub maximum_oldest_queue_age_ms: u64,
    pub minimum_free_disk_bytes: u64,
}

impl Default for OperationalLimits {
    fn default() -> Self {
        Self {
            maximum_queued_jobs: 1_000,
            maximum_running_jobs: 32,
            maximum_oldest_queue_age_ms: 15 * 60 * 1_000,
            minimum_free_disk_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerResourceLimits {
    pub maximum_rss_bytes: u64,
    pub maximum_cpu_time_ms: u64,
    pub maximum_wall_time_ms: u64,
    pub cancellation_grace_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerResourceUsage {
    pub rss_bytes: u64,
    pub cpu_time_ms: u64,
    pub wall_time_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerLimit {
    Rss,
    CpuTime,
    WallTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerAction {
    Continue,
    Cancel { limit: WorkerLimit },
    Terminate { limit: WorkerLimit },
}

pub struct WorkerSupervisor {
    limits: WorkerResourceLimits,
    cancellation: Option<(WorkerLimit, u64)>,
    last_observed_at_ms: Option<u64>,
}

impl WorkerSupervisor {
    fn new(limits: WorkerResourceLimits) -> Self {
        Self {
            limits,
            cancellation: None,
            last_observed_at_ms: None,
        }
    }

    pub fn observe(
        &mut self,
        usage: WorkerResourceUsage,
        observed_at_ms: u64,
    ) -> Result<WorkerAction, String> {
        if self
            .last_observed_at_ms
            .is_some_and(|previous| observed_at_ms < previous)
        {
            return Err("Worker supervision clock moved backwards".into());
        }
        self.last_observed_at_ms = Some(observed_at_ms);
        if let Some((limit, deadline)) = self.cancellation {
            return Ok(if observed_at_ms >= deadline {
                WorkerAction::Terminate { limit }
            } else {
                WorkerAction::Cancel { limit }
            });
        }
        let violation = if usage.rss_bytes > self.limits.maximum_rss_bytes {
            Some(WorkerLimit::Rss)
        } else if usage.cpu_time_ms > self.limits.maximum_cpu_time_ms {
            Some(WorkerLimit::CpuTime)
        } else if usage.wall_time_ms > self.limits.maximum_wall_time_ms {
            Some(WorkerLimit::WallTime)
        } else {
            None
        };
        let Some(limit) = violation else {
            return Ok(WorkerAction::Continue);
        };
        let deadline = observed_at_ms
            .checked_add(self.limits.cancellation_grace_ms)
            .ok_or("Worker cancellation deadline overflow")?;
        self.cancellation = Some((limit, deadline));
        Ok(WorkerAction::Cancel { limit })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationalMode {
    Starting,
    Ready,
    Draining,
    Unavailable,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationalMetrics {
    pub accepted: u64,
    pub rejected: u64,
    pub completed: u64,
    pub partial: u64,
    pub superseded: u64,
    pub failed: u64,
    pub latency_ms: BTreeMap<String, Vec<u64>>,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub invalidation_paths: u64,
    pub invalidation_samples: Vec<u64>,
    pub worker_peak_rss_bytes: u64,
    pub provider_failures: BTreeMap<String, u64>,
    pub covered_files: u64,
    pub partial_coverage_files: u64,
    pub model_requests: u64,
    pub model_completed: u64,
    pub model_disabled: u64,
    pub model_failed: u64,
    pub model_input_units: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readiness {
    pub mode: OperationalMode,
    pub accepting_work: bool,
    pub reasons: Vec<String>,
    pub queue: QueueStats,
    pub free_disk_bytes: u64,
}

pub struct OperationalController {
    limits: OperationalLimits,
    worker_limits: WorkerResourceLimits,
    mode: OperationalMode,
    reconciled: bool,
    metrics: OperationalMetrics,
}

impl OperationalController {
    pub fn new(
        limits: OperationalLimits,
        worker_limits: WorkerResourceLimits,
    ) -> Result<Self, String> {
        if limits.maximum_queued_jobs == 0
            || limits.maximum_running_jobs == 0
            || limits.minimum_free_disk_bytes == 0
        {
            return Err("Operational limits must be positive".into());
        }
        if worker_limits.maximum_rss_bytes == 0
            || worker_limits.maximum_rss_bytes > 1024 * 1024 * 1024 * 1024
            || worker_limits.maximum_cpu_time_ms == 0
            || worker_limits.maximum_cpu_time_ms > 24 * 60 * 60 * 1_000
            || worker_limits.maximum_wall_time_ms == 0
            || worker_limits.maximum_wall_time_ms > 24 * 60 * 60 * 1_000
            || !(100..=30_000).contains(&worker_limits.cancellation_grace_ms)
        {
            return Err("Worker resource limits must be explicitly bounded".into());
        }
        Ok(Self {
            limits,
            worker_limits,
            mode: OperationalMode::Starting,
            reconciled: false,
            metrics: OperationalMetrics::default(),
        })
    }

    pub fn worker_supervisor(&self) -> WorkerSupervisor {
        WorkerSupervisor::new(self.worker_limits.clone())
    }

    pub fn finish_reconciliation(&mut self, result: &Reconciliation) -> Result<(), String> {
        if result.missing_references > 0 || result.corrupt_references > 0 {
            self.mode = OperationalMode::Unavailable;
            return Err("Artifact reconciliation found missing or corrupt references".into());
        }
        self.reconciled = true;
        self.mode = OperationalMode::Ready;
        Ok(())
    }

    pub fn begin_drain(&mut self) {
        self.mode = OperationalMode::Draining;
    }

    pub fn readiness(&self, queue: QueueStats, free_disk_bytes: u64) -> Readiness {
        let mut reasons = Vec::new();
        if !self.reconciled {
            reasons.push("startup reconciliation incomplete".into());
        }
        if self.mode == OperationalMode::Draining {
            reasons.push("service is draining".into());
        }
        if self.mode == OperationalMode::Unavailable {
            reasons.push("service is unavailable".into());
        }
        if queue.queued >= self.limits.maximum_queued_jobs {
            reasons.push("queue capacity exhausted".into());
        }
        if queue.running >= self.limits.maximum_running_jobs {
            reasons.push("worker capacity exhausted".into());
        }
        if queue.oldest_queued_age_ms > self.limits.maximum_oldest_queue_age_ms {
            reasons.push("queue age limit exceeded".into());
        }
        if free_disk_bytes < self.limits.minimum_free_disk_bytes {
            reasons.push("free disk reserve exhausted".into());
        }
        Readiness {
            mode: self.mode,
            accepting_work: self.mode == OperationalMode::Ready && reasons.is_empty(),
            reasons,
            queue,
            free_disk_bytes,
        }
    }

    pub fn admit(&mut self, queue: QueueStats, free_disk_bytes: u64) -> Result<(), String> {
        let readiness = self.readiness(queue, free_disk_bytes);
        if readiness.accepting_work {
            self.metrics.accepted += 1;
            Ok(())
        } else {
            self.metrics.rejected += 1;
            Err(format!(
                "Work admission rejected: {}",
                readiness.reasons.join(", ")
            ))
        }
    }

    pub fn record_terminal(
        &mut self,
        outcome: &str,
        latency_kind: &str,
        latency_ms: u64,
    ) -> Result<(), String> {
        let counter = match outcome {
            "completed" => self.metrics.completed,
            "partial" => self.metrics.partial,
            "superseded" => self.metrics.superseded,
            "failed" => self.metrics.failed,
            _ => return Err("Invalid operational outcome".into()),
        };
        let incremented = counter
            .checked_add(1)
            .ok_or("Operational counter overflow")?;
        self.record_latency(latency_kind, latency_ms)?;
        match outcome {
            "completed" => self.metrics.completed = incremented,
            "partial" => self.metrics.partial = incremented,
            "superseded" => self.metrics.superseded = incremented,
            "failed" => self.metrics.failed = incremented,
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn record_latency(&mut self, kind: &str, milliseconds: u64) -> Result<(), String> {
        if !matches!(
            kind,
            "event_to_ready"
                | "cold_index"
                | "warm_analysis"
                | "graph_expansion"
                | "source_preview"
                | "queue_wait"
                | "backup"
                | "restore"
        ) {
            return Err("Invalid latency kind".into());
        }
        let samples = self.metrics.latency_ms.entry(kind.into()).or_default();
        if samples.len() >= 10_000 {
            return Err("Latency sample limit reached".into());
        }
        samples.push(milliseconds);
        Ok(())
    }

    pub fn record_cache(&mut self, hit: bool) -> Result<(), String> {
        let counter = if hit {
            &mut self.metrics.cache_hits
        } else {
            &mut self.metrics.cache_misses
        };
        *counter = counter.checked_add(1).ok_or("Cache counter overflow")?;
        Ok(())
    }

    pub fn record_invalidation(&mut self, paths: usize) -> Result<(), String> {
        if paths > 1_000_000 || self.metrics.invalidation_samples.len() >= 10_000 {
            return Err("Invalidation sample exceeds its bound".into());
        }
        let paths = u64::try_from(paths).map_err(|_| "Invalidation size exceeds u64")?;
        self.metrics.invalidation_paths = self
            .metrics
            .invalidation_paths
            .checked_add(paths)
            .ok_or("Invalidation counter overflow")?;
        self.metrics.invalidation_samples.push(paths);
        Ok(())
    }

    pub fn record_worker_rss(&mut self, bytes: u64) {
        self.metrics.worker_peak_rss_bytes = self.metrics.worker_peak_rss_bytes.max(bytes);
    }

    pub fn record_provider_failure(&mut self, kind: &str) -> Result<(), String> {
        if !matches!(
            kind,
            "auth" | "throttled" | "timeout" | "transport" | "invalid_payload" | "unavailable"
        ) {
            return Err("Invalid provider failure kind".into());
        }
        let counter = self
            .metrics
            .provider_failures
            .entry(kind.into())
            .or_default();
        *counter = counter
            .checked_add(1)
            .ok_or("Provider failure counter overflow")?;
        Ok(())
    }

    pub fn record_coverage(&mut self, covered: usize, total: usize) -> Result<(), String> {
        if covered > total || total > 1_000_000 {
            return Err("Invalid coverage measurement".into());
        }
        let partial = u64::try_from(total - covered).map_err(|_| "Coverage exceeds u64")?;
        let covered = u64::try_from(covered).map_err(|_| "Coverage exceeds u64")?;
        let covered_files = self
            .metrics
            .covered_files
            .checked_add(covered)
            .ok_or("Coverage counter overflow")?;
        let partial_coverage_files = self
            .metrics
            .partial_coverage_files
            .checked_add(partial)
            .ok_or("Coverage counter overflow")?;
        self.metrics.covered_files = covered_files;
        self.metrics.partial_coverage_files = partial_coverage_files;
        Ok(())
    }

    pub fn record_model_usage(&mut self, outcome: &str, input_units: u64) -> Result<(), String> {
        if input_units > 1_000_000_000 || !matches!(outcome, "completed" | "disabled" | "failed") {
            return Err("Invalid model usage measurement".into());
        }
        let requests = self
            .metrics
            .model_requests
            .checked_add(1)
            .ok_or("Model usage counter overflow")?;
        let units = self
            .metrics
            .model_input_units
            .checked_add(input_units)
            .ok_or("Model usage counter overflow")?;
        let counter = match outcome {
            "completed" => &mut self.metrics.model_completed,
            "disabled" => &mut self.metrics.model_disabled,
            "failed" => &mut self.metrics.model_failed,
            _ => unreachable!(),
        };
        *counter = counter
            .checked_add(1)
            .ok_or("Model usage counter overflow")?;
        self.metrics.model_requests = requests;
        self.metrics.model_input_units = units;
        Ok(())
    }

    pub fn metrics(&self) -> &OperationalMetrics {
        &self.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker_limits() -> WorkerResourceLimits {
        WorkerResourceLimits {
            maximum_rss_bytes: 512 * 1024 * 1024,
            maximum_cpu_time_ms: 60_000,
            maximum_wall_time_ms: 120_000,
            cancellation_grace_ms: 1_000,
        }
    }

    #[test]
    fn reconciliation_capacity_disk_and_drain_gate_admission() {
        let mut controller = OperationalController::new(
            OperationalLimits {
                maximum_queued_jobs: 2,
                maximum_running_jobs: 1,
                maximum_oldest_queue_age_ms: 100,
                minimum_free_disk_bytes: 50,
            },
            worker_limits(),
        )
        .unwrap();
        assert!(controller.admit(QueueStats::default(), 100).is_err());
        controller
            .finish_reconciliation(&Reconciliation::default())
            .unwrap();
        controller.admit(QueueStats::default(), 100).unwrap();
        assert!(
            controller
                .admit(
                    QueueStats {
                        queued: 2,
                        ..QueueStats::default()
                    },
                    100
                )
                .is_err()
        );
        assert!(controller.admit(QueueStats::default(), 49).is_err());
        controller.begin_drain();
        assert!(controller.admit(QueueStats::default(), 100).is_err());
        assert_eq!(controller.metrics().accepted, 1);
        assert_eq!(controller.metrics().rejected, 4);
    }

    #[test]
    fn corrupt_reconciliation_blocks_readiness_and_metrics_are_bounded() {
        let mut controller =
            OperationalController::new(OperationalLimits::default(), worker_limits()).unwrap();
        assert!(
            controller
                .finish_reconciliation(&Reconciliation {
                    corrupt_references: 1,
                    ..Reconciliation::default()
                })
                .is_err()
        );
        assert_eq!(
            controller.readiness(QueueStats::default(), u64::MAX).mode,
            OperationalMode::Unavailable
        );
        controller
            .record_terminal("partial", "event_to_ready", 42)
            .unwrap();
        assert_eq!(controller.metrics().partial, 1);
        assert_eq!(controller.metrics().latency_ms["event_to_ready"], vec![42]);
    }

    #[test]
    fn operational_measurements_are_bounded_low_cardinality_and_fail_atomically() {
        let mut controller =
            OperationalController::new(OperationalLimits::default(), worker_limits()).unwrap();
        controller.record_latency("queue_wait", 18).unwrap();
        controller.record_cache(true).unwrap();
        controller.record_cache(false).unwrap();
        controller.record_invalidation(12).unwrap();
        controller.record_worker_rss(512 * 1024 * 1024);
        controller.record_worker_rss(256 * 1024 * 1024);
        controller.record_provider_failure("throttled").unwrap();
        controller.record_coverage(90, 100).unwrap();
        controller.record_model_usage("disabled", 0).unwrap();

        let before = controller.metrics().clone();
        assert!(
            controller
                .record_terminal("unknown", "queue_wait", 1)
                .is_err()
        );
        assert!(controller.record_latency("user-supplied-label", 1).is_err());
        assert!(controller.record_provider_failure("provider-name").is_err());
        assert!(controller.record_coverage(2, 1).is_err());
        assert!(controller.record_invalidation(1_000_001).is_err());
        assert!(controller.record_model_usage("enabled", 1).is_err());
        assert_eq!(controller.metrics(), &before);

        let metrics = controller.metrics();
        assert_eq!(metrics.latency_ms["queue_wait"], vec![18]);
        assert_eq!((metrics.cache_hits, metrics.cache_misses), (1, 1));
        assert_eq!(metrics.invalidation_samples, vec![12]);
        assert_eq!(metrics.invalidation_paths, 12);
        assert_eq!(metrics.worker_peak_rss_bytes, 512 * 1024 * 1024);
        assert_eq!(metrics.provider_failures["throttled"], 1);
        assert_eq!(
            (metrics.covered_files, metrics.partial_coverage_files),
            (90, 10)
        );
        assert_eq!(
            (
                metrics.model_requests,
                metrics.model_completed,
                metrics.model_disabled,
                metrics.model_failed,
                metrics.model_input_units,
            ),
            (1, 0, 1, 0, 0)
        );
    }

    #[test]
    fn worker_supervision_requires_limits_and_escalates_cancel_to_terminate() {
        assert!(
            OperationalController::new(
                OperationalLimits::default(),
                WorkerResourceLimits {
                    maximum_rss_bytes: 0,
                    ..worker_limits()
                },
            )
            .is_err()
        );
        let controller =
            OperationalController::new(OperationalLimits::default(), worker_limits()).unwrap();
        let mut supervisor = controller.worker_supervisor();
        let within = WorkerResourceUsage {
            rss_bytes: 256 * 1024 * 1024,
            cpu_time_ms: 30_000,
            wall_time_ms: 40_000,
        };
        assert_eq!(
            supervisor.observe(within, 10_000).unwrap(),
            WorkerAction::Continue
        );
        let exceeded = WorkerResourceUsage {
            rss_bytes: 512 * 1024 * 1024 + 1,
            ..within
        };
        assert_eq!(
            supervisor.observe(exceeded, 10_100).unwrap(),
            WorkerAction::Cancel {
                limit: WorkerLimit::Rss
            }
        );
        assert_eq!(
            supervisor.observe(within, 11_099).unwrap(),
            WorkerAction::Cancel {
                limit: WorkerLimit::Rss
            }
        );
        assert_eq!(
            supervisor.observe(within, 11_100).unwrap(),
            WorkerAction::Terminate {
                limit: WorkerLimit::Rss
            }
        );
        assert!(supervisor.observe(within, 11_099).is_err());

        for (usage, limit) in [
            (
                WorkerResourceUsage {
                    cpu_time_ms: 60_001,
                    ..within
                },
                WorkerLimit::CpuTime,
            ),
            (
                WorkerResourceUsage {
                    wall_time_ms: 120_001,
                    ..within
                },
                WorkerLimit::WallTime,
            ),
        ] {
            let mut supervisor = controller.worker_supervisor();
            assert_eq!(
                supervisor.observe(usage, 20_000).unwrap(),
                WorkerAction::Cancel { limit }
            );
        }
    }
}
