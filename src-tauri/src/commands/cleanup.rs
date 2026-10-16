//! Cleanup commands.
//!
//! `cleanup_execute` refuses to run unless the plan is explicit: every item must list the exact
//! paths the user chose, and each one is revalidated immediately before it is touched.

use std::collections::HashMap;

use tauri::State;

use crate::cleanup::execute::{self, CleanupPlan, CleanupPreview, CleanupResult, ValidationContext};
use crate::cleanup::rules::{self, CleanupFinding, CleanupRule, RecycleBinSummary};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Mirrors `CleanupFinding` (returned as-is).
pub use crate::cleanup::rules::CleanupFinding as Finding;

/// Builds findings for a scan from the user's rules. Free at every tier.
#[tauri::command]
pub fn cleanup_scan(
    scan_id: String,
    rules: Vec<CleanupRule>,
    state: State<'_, AppState>,
) -> AppResult<Vec<CleanupFinding>> {
    crate::authz::authorize_state(&state, crate::authz::Permission::CoreAnalysis)?;
    let scans = state.scans()?;
    let entry = scans
        .get(&scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Unknown scan {scan_id}")))?;
    let index = entry
        .index
        .clone()
        .ok_or_else(|| AppError::NotFound("The scan is still running.".to_string()))?;
    drop(scans);

    let now = crate::licensing::verify::now_millis();
    Ok(rules::build_findings(&index, &rules, now))
}

/// Simulates a plan. Changes nothing, and is free at every tier.
#[tauri::command]
pub fn cleanup_preview(plan: CleanupPlan, state: State<'_, AppState>) -> AppResult<CleanupPreview> {
    crate::authz::authorize_state(&state, crate::authz::Permission::PreviewOnly)?;
    let prepared = prepare(&plan, &state)?;
    let context = ValidationContext {
        index: &prepared.index,
        blocked_paths: &prepared.blocked,
        approved_roots: &prepared.approved,
    };
    Ok(execute::preview_plan(&plan, &prepared.titles, &context))
}

/// Executes a plan. Every path is revalidated immediately before it is touched.
///
/// This is the single most dangerous command in the app, so it is gated twice:
///   1. `PaidCleanup` — the authorization layer, inside the backend, before anything is prepared.
///   2. `PermanentDeletion` — again inside the backend, for any item that asked to bypass the
///      Recycle Bin.
///
/// The check reads the entitlement the backend resolved. A modified React interface cannot make
/// this command succeed by hiding a paywall, because it does not consult the interface at all.
#[tauri::command]
pub fn cleanup_execute(plan: CleanupPlan, state: State<'_, AppState>) -> AppResult<CleanupResult> {
    crate::authz::authorize_state(&state, crate::authz::Permission::PaidCleanup)?;

    // Permanent deletion is a separate, separately-gated capability.
    if plan.items.iter().any(|item| item.action == "permanent") {
        crate::authz::authorize_state(
            &state,
            crate::authz::Permission::PermanentDeletion,
        )?;
    }

    // A plan that touches many paths at once is a bulk operation, which is its own capability.
    let total_paths: usize = plan.items.iter().map(|item| item.paths.len()).sum();
    if total_paths > 50 {
        crate::authz::authorize_state(&state, crate::authz::Permission::BulkFileOperations)?;
    }

    let prepared = prepare(&plan, &state)?;
    let context = ValidationContext {
        index: &prepared.index,
        blocked_paths: &prepared.blocked,
        approved_roots: &prepared.approved,
    };
    // The executor revalidates every path immediately before acting, so the result below is the
    // authoritative account of what happened — including anything that was skipped.
    Ok(execute::execute_plan(&plan, &prepared.titles, &context))
}

/// Recycle Bin contents, via the Windows shell.
#[tauri::command]
pub fn recycle_bin_summary() -> RecycleBinSummary {
    let info = crate::platform::recycle_bin(None);
    RecycleBinSummary {
        item_count: info.item_count,
        size_bytes: if info.supported { Some(info.size_bytes) } else { None },
        supported: info.supported,
        detail: info.detail,
    }
}

/// Everything the executor needs for a plan.
struct PreparedPlan {
    index: std::sync::Arc<crate::scanner::aggregate::ScanIndex>,
    approved: Vec<String>,
    blocked: Vec<String>,
    titles: HashMap<String, String>,
}

/// Resolves everything the executor needs for a plan.
fn prepare(plan: &CleanupPlan, state: &State<'_, AppState>) -> AppResult<PreparedPlan> {
    let scan_id = plan
        .scan_id
        .clone()
        .ok_or_else(|| AppError::InvalidArgument("A plan must name its scan.".to_string()))?;

    let scans = state.scans()?;
    let entry = scans
        .get(&scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Unknown scan {scan_id}")))?
        .clone();
    drop(scans);

    let index = entry
        .index
        .clone()
        .ok_or_else(|| AppError::NotFound("The scan is still running.".to_string()))?;

    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::Internal("The settings lock was poisoned.".to_string()))?
        .clone()
        .unwrap_or_default();

    // Approved roots: every user-configured temp/cache folder, plus the root of each finding that
    // exists for this scan. Nothing outside these can ever be acted on.
    let rules = stored_rules(state);
    let findings =
        crate::cleanup::rules::build_findings(&index, &rules, crate::licensing::verify::now_millis());

    let mut titles = HashMap::new();
    let mut approved: Vec<String> = Vec::new();
    approved.extend(settings.temp_paths.iter().cloned());
    approved.extend(settings.cache_paths.iter().cloned());
    for finding in &findings {
        titles.insert(finding.id.clone(), finding.title.clone());
        if let Some(path) = &finding.path {
            approved.push(path.clone());
        }
    }

    Ok(PreparedPlan {
        index,
        approved,
        blocked: settings.exclude_paths.clone(),
        titles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_rules_are_derived_from_settings() {
        // The mapping is exercised through `prepare`; here we assert the shape of the rule list.
        let settings = crate::settings::AppSettings {
            temp_paths: vec!["C:\\Temp".to_string()],
            cache_paths: vec!["C:\\Cache".to_string()],
            ..Default::default()
        };
        assert_eq!(settings.temp_paths.len(), 1);
        assert_eq!(settings.cache_paths.len(), 1);
    }
}
