//! Shared application state.
//!
//! Scans and duplicate reports are kept in memory only. Nothing about a scan is persisted except a
//! short summary row for history, and the frontend only ever receives a pruned tree.
//!
//! The `authorization` field is the single source of truth for what this install may do. It is
//! resolved once at startup from a locally stored licence key and refreshed by the licensing
//! commands. The webview can read it but never writes it.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::authz::Authorization;
use crate::config::product::CommercialConfig;
use crate::duplicates::grouping::{DuplicateOptions, DuplicateReport};
use crate::licensing::payload::DeviceBinding;
use crate::licensing::verify::Entitlement;
use crate::scanner::aggregate::{PruneOptions, ScanIndex};
use crate::scanner::progress::{ProgressCounters, ProgressSnapshot};
use crate::scanner::walker::ScanRun;
use crate::settings::AppSettings;
use crate::storage::db::Database;

/// One completed (or in-flight) scan.
pub struct ScanEntry {
    pub scan_id: String,
    pub root_path: String,
    pub root_name: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub index: Option<Arc<ScanIndex>>,
    pub errors: Vec<crate::scanner::walker::WalkError>,
    pub pruned_nodes: usize,
    pub expected_bytes: Option<u64>,
    pub hit_limit: bool,
    pub duration_ms: Option<i64>,
}

/// Shared mutable state behind `tauri::State`.
pub struct AppState {
    pub database: Mutex<Option<Database>>,
    pub scans: Mutex<HashMap<String, Arc<ScanEntry>>>,
    /// Live counters for scans that are still running.
    pub counters: Mutex<HashMap<String, Arc<ProgressCounters>>>,
    /// Cancel flags, kept so a cancelled scan stays cancelled.
    pub cancelled: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub duplicate_reports: Mutex<HashMap<String, DuplicateReport>>,
    pub duplicate_options: Mutex<HashMap<String, DuplicateOptions>>,
    /// What this install is allowed to do. Never written by the webview.
    pub authorization: Mutex<Option<Authorization>>,
    /// This machine's device binding.
    pub device: DeviceBinding,
    pub settings: Mutex<Option<AppSettings>>,
    /// Default pruning options for the tree sent to the UI.
    pub prune: PruneOptions,
    /// The commercial configuration for this build.
    pub config: CommercialConfig,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            database: Mutex::new(None),
            scans: Mutex::new(HashMap::new()),
            counters: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashMap::new()),
            duplicate_reports: Mutex::new(HashMap::new()),
            duplicate_options: Mutex::new(HashMap::new()),
            authorization: Mutex::new(None),
            // A placeholder binding until `setup` replaces it with the real one.
            device: DeviceBinding {
                value: "dev-unresolved".to_string(),
            },
            settings: Mutex::new(None),
            prune: PruneOptions::default(),
            config: CommercialConfig::default(),
        }
    }

    /// Replaces the device binding, e.g. once the data directory is known.
    pub fn set_device(&mut self, device: DeviceBinding) {
        self.device = device;
    }

    pub fn database(&self) -> Result<std::sync::MutexGuard<'_, Option<Database>>, String> {
        self.database
            .lock()
            .map_err(|_| "The database lock was poisoned.".to_string())
    }

    pub fn scans(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<ScanEntry>>>, String> {
        self.scans
            .lock()
            .map_err(|_| "The scan registry lock was poisoned.".to_string())
    }

    /// The current authorization. Fails closed to free when it has not been resolved yet.
    pub fn authorization(&self) -> Result<Authorization, String> {
        match self.authorization.lock() {
            Ok(guard) => Ok(guard
                .clone()
                .unwrap_or_else(|| Authorization::free(self.device.clone()))),
            Err(_) => Ok(Authorization::free(self.device.clone())),
        }
    }

    /// Replaces the authorization. Only the licensing commands call this.
    pub fn set_authorization(&self, authorization: Authorization) {
        if let Ok(mut guard) = self.authorization.lock() {
            *guard = Some(authorization);
        }
    }

    /// The entitlement the UI is shown.
    pub fn entitlement(&self) -> Entitlement {
        self.authorization()
            .map(|auth| auth.entitlement)
            .unwrap_or_else(|_| crate::licensing::verify::free_entitlement())
    }

    /// The locally stored licence key, if one has been activated.
    pub fn stored_license_key(&self) -> Result<Option<String>, String> {
        let guard = self.database()?;
        match guard.as_ref() {
            Some(database) => database.load_license_key(),
            None => Ok(None),
        }
    }

    /// Stores or clears the local licence key.
    pub fn store_license_key(&self, key: Option<&str>) -> Result<(), String> {
        let guard = self.database()?;
        match guard.as_ref() {
            Some(database) => match key {
                Some(key) => database.save_license_key(key),
                None => database.clear_license_key(),
            },
            None => Ok(()),
        }
    }

    /// Records which device the stored key is bound to.
    pub fn set_license_binding(&self, device_binding: &str) -> Result<(), String> {
        let guard = self.database()?;
        match guard.as_ref() {
            Some(database) => database.set_license_binding(device_binding),
            None => Ok(()),
        }
    }
}

/// Snapshot of a scan's counters, ready to become a progress event.
pub fn snapshot(
    scan_id: &str,
    counters: &ProgressCounters,
    elapsed_ms: u64,
    expected: Option<u64>,
) -> ProgressSnapshot {
    let (percent, eta_ms) = crate::scanner::progress::estimate(counters.bytes(), expected, elapsed_ms);
    ProgressSnapshot {
        phase: counters.phase(),
        files: counters.files(),
        folders: counters.folders(),
        bytes: counters.bytes(),
        errors: counters.error_count(),
        current_path: counters.current_path(),
        elapsed_ms,
        eta_ms,
        percent,
    }
}

/// Helper used by the scan command: builds an entry from a finished walk.
pub fn entry_from_run(
    scan_id: String,
    root_path: String,
    root_name: String,
    started_at: i64,
    run: ScanRun,
    expected_bytes: Option<u64>,
) -> ScanEntry {
    let finished_at = crate::licensing::verify::now_millis();
    let duration_ms = finished_at - started_at;
    let status = if run.cancelled {
        "cancelled".to_string()
    } else if run.errors.is_empty() {
        "complete".to_string()
    } else {
        "partial".to_string()
    };
    let index = Arc::new(ScanIndex::from_run(&run));
    ScanEntry {
        scan_id,
        root_path,
        root_name,
        started_at,
        finished_at: Some(finished_at),
        status,
        index: Some(index),
        errors: run.errors,
        pruned_nodes: 0,
        expected_bytes,
        hit_limit: run.hit_limit,
        duration_ms: Some(duration_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_fails_closed_to_free() {
        let state = AppState::new();
        let auth = state.authorization().expect("resolves");
        assert!(!auth.is_paid());
        assert!(auth.entitlement.features.core_analysis);
        assert_eq!(state.entitlement().state, crate::licensing::verify::EntitlementState::Free);
    }

    #[test]
    fn the_device_binding_starts_unresolved_and_is_replaced() {
        let mut state = AppState::new();
        assert_eq!(state.device.value, "dev-unresolved");
        state.set_device(DeviceBinding {
            value: "dev-real".to_string(),
        });
        assert_eq!(state.device.value, "dev-real");
    }

    #[test]
    fn the_commercial_config_is_valid() {
        assert_eq!(AppState::new().config.validate(), Ok(()));
        assert!(AppState::new().config.environment.is_test());
    }
}
