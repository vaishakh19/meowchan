//! Progress reporting.
//!
//! Counters live in atomics so worker threads can update them without contending on a lock. The
//! emitter thread reads a consistent-enough snapshot roughly every 100 ms — the numbers are
//! monotonic per counter, so a snapshot is always self-consistent in the direction that matters
//! (nothing ever goes backwards).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Mirrors the `ScanPhase` union in `src/types/contracts.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanPhase {
    Preparing,
    Walking,
    Hashing,
    Aggregating,
    Finalizing,
    Done,
    Cancelled,
    Error,
}

impl ScanPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScanPhase::Preparing => "preparing",
            ScanPhase::Walking => "walking",
            ScanPhase::Hashing => "hashing",
            ScanPhase::Aggregating => "aggregating",
            ScanPhase::Finalizing => "finalizing",
            ScanPhase::Done => "done",
            ScanPhase::Cancelled => "cancelled",
            ScanPhase::Error => "error",
        }
    }
}

/// Mirrors `ScanStatus` in `src/types/contracts.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanStatus {
    Running,
    Complete,
    Partial,
    Cancelled,
    Error,
}

impl ScanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScanStatus::Running => "running",
            ScanStatus::Complete => "complete",
            ScanStatus::Partial => "partial",
            ScanStatus::Cancelled => "cancelled",
            ScanStatus::Error => "error",
        }
    }
}

/// Shared counters. Cheap to clone (everything is behind an `Arc`).
#[derive(Debug, Default)]
pub struct ProgressCounters {
    pub files: AtomicU64,
    pub folders: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicUsize,
    pub cancelled: AtomicBool,
    current_path: Mutex<String>,
    phase: Mutex<ScanPhase>,
}

impl ProgressCounters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&self, bytes: u64) {
        self.files.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn add_folder(&self) {
        self.folders.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn set_current_path(&self, path: &str) {
        if let Ok(mut guard) = self.current_path.lock() {
            *guard = path.to_string();
        }
    }

    pub fn set_phase(&self, phase: ScanPhase) {
        if let Ok(mut guard) = self.phase.lock() {
            *guard = phase;
        }
    }

    pub fn files(&self) -> u64 {
        self.files.load(Ordering::Relaxed)
    }

    pub fn folders(&self) -> u64 {
        self.folders.load(Ordering::Relaxed)
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn error_count(&self) -> usize {
        self.errors.load(Ordering::Relaxed)
    }

    pub fn current_path(&self) -> String {
        self.current_path
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn phase(&self) -> ScanPhase {
        self.phase.lock().map(|guard| *guard).unwrap_or(ScanPhase::Preparing)
    }
}

/// A single progress reading.
#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub phase: ScanPhase,
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    pub errors: usize,
    pub current_path: String,
    pub elapsed_ms: u64,
    pub eta_ms: Option<u64>,
    pub percent: Option<f64>,
}

/// ETA: only meaningful once more than 5% of the expected total has been seen.
pub fn estimate(analyzed: u64, expected: Option<u64>, elapsed_ms: u64) -> (Option<f64>, Option<u64>) {
    let expected = match expected {
        Some(value) if value > 0 => value,
        _ => return (None, None),
    };
    let percent = (analyzed as f64 / expected as f64) * 100.0;
    if percent <= 5.0 {
        return (Some(percent), None);
    }
    let remaining = expected.saturating_sub(analyzed);
    let rate = analyzed as f64 / (elapsed_ms.max(1) as f64);
    if rate <= 0.0 {
        return (Some(percent), None);
    }
    let eta = (remaining as f64 / rate) as u64;
    (Some(percent.min(100.0)), Some(eta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_is_unknown_below_five_percent() {
        let (percent, eta) = estimate(10, Some(1000), 1000);
        assert_eq!(percent, Some(1.0));
        assert_eq!(eta, None);
    }

    #[test]
    fn eta_is_provided_above_five_percent() {
        let (percent, eta) = estimate(500, Some(1000), 1000);
        assert!(percent.unwrap() >= 50.0);
        assert_eq!(eta, Some(1000));
    }

    #[test]
    fn eta_is_unknown_without_an_expected_total() {
        assert_eq!(estimate(500, None, 1000), (None, None));
        assert_eq!(estimate(500, Some(0), 1000), (None, None));
    }

    #[test]
    fn counters_are_monotonic_and_cancellable() {
        let counters = ProgressCounters::new();
        counters.add_file(100);
        counters.add_file(50);
        counters.add_folder();
        counters.add_error();
        assert_eq!(counters.files(), 2);
        assert_eq!(counters.bytes(), 150);
        assert_eq!(counters.folders(), 1);
        assert_eq!(counters.error_count(), 1);
        assert!(!counters.is_cancelled());
        counters.cancel();
        assert!(counters.is_cancelled());
    }

    #[test]
    fn phases_and_statuses_match_the_wire_names() {
        assert_eq!(ScanPhase::Walking.as_str(), "walking");
        assert_eq!(ScanPhase::Done.as_str(), "done");
        assert_eq!(ScanStatus::Partial.as_str(), "partial");
        assert_eq!(ScanStatus::Cancelled.as_str(), "cancelled");
    }
}
