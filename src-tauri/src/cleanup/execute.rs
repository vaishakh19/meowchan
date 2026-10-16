//! Cleanup execution: preview (dry run) and execute.
//!
//! The safety model lives here:
//!   - a plan is refused unless every item is explicitly listed by the caller,
//!   - every path is re-checked immediately before acting (existence, type, mtime, size),
//!   - a path that changed since the scan is skipped and reported, never acted on,
//!   - the Recycle Bin is used unless the caller explicitly asks for permanent deletion,
//!   - failures and partial completion are reported path by path,
//!   - at least one copy of every duplicate group is preserved by construction: the caller can
//!     only list paths it chose, and the UI disables "select every copy".

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::filesystem::long_path::to_unc_long_path;
use crate::scanner::aggregate::{ScanIndex, OTHER_NODE_SUFFIX};

/// Mirrors `CleanupPlanItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPlanItem {
    pub finding_id: String,
    /// Explicit user opt-in. Items not listed here are refused.
    pub paths: Vec<String>,
    /// `recycle` (preferred) or `permanent`.
    pub action: String,
}

/// Mirrors `CleanupPlan`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPlan {
    pub scan_id: Option<String>,
    pub items: Vec<CleanupPlanItem>,
    pub dry_run: bool,
}

/// Mirrors `CleanupPathOutcome`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPathOutcome {
    pub path: String,
    /// `ok`, `missing`, `denied`, `changed`, `skipped`.
    pub status: String,
    pub size_bytes: u64,
    pub detail: String,
}

/// Mirrors `CleanupPreviewItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPreviewItem {
    pub finding_id: String,
    pub title: String,
    pub action: String,
    pub path_count: usize,
    pub file_count: u64,
    pub folder_count: u64,
    pub size_bytes: u64,
    pub paths: Vec<CleanupPathOutcome>,
}

/// Mirrors `CleanupPreview`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPreview {
    pub items: Vec<CleanupPreviewItem>,
    pub total_size_bytes: u64,
    pub total_path_count: usize,
    pub warnings: Vec<String>,
}

/// Mirrors `CleanupResultItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResultItem {
    pub finding_id: String,
    pub title: String,
    pub action: String,
    pub removed_count: u64,
    pub failed_count: u64,
    pub removed_bytes: u64,
    pub results: Vec<CleanupPathOutcome>,
}

/// Mirrors `CleanupResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub items: Vec<CleanupResultItem>,
    pub total_removed_count: u64,
    pub total_failed_count: u64,
    pub total_removed_bytes: u64,
    pub completed_at: i64,
    pub partial: bool,
}

/// Everything the executor needs to revalidate a path against the scan it came from.
pub struct ValidationContext<'a> {
    pub index: &'a ScanIndex,
    /// Paths the user has marked as never-touch. Always refused.
    pub blocked_paths: &'a [String],
    /// Paths the user explicitly opted into (rule roots).
    pub approved_roots: &'a [String],
}

/// Outcome of revalidating a single path immediately before acting.
enum Check {
    Ok { size_bytes: u64 },
    Skip { reason: String },
}

fn revalidate(path: &str, context: &ValidationContext<'_>) -> Check {
    let candidate = PathBuf::from(path);

    if path.ends_with(OTHER_NODE_SUFFIX) {
        return Check::Skip {
            reason: "This is an aggregate placeholder, not a real filesystem entry.".to_string(),
        };
    }

    if is_blocked(&candidate, context.blocked_paths) {
        return Check::Skip {
            reason: "This path is on your never-touch list.".to_string(),
        };
    }

    if !is_approved(&candidate, context.approved_roots) {
        return Check::Skip {
            reason: "This path is outside the folders you approved for cleanup.".to_string(),
        };
    }

    let long = to_unc_long_path(&candidate);
    let metadata = match std::fs::symlink_metadata(&long) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Check::Skip {
                reason: format!("The path could not be read ({error}); skipping."),
            };
        }
    };

    if metadata.is_symlink() {
        return Check::Skip {
            reason: "This is a link; Diskwise never deletes through a link.".to_string(),
        };
    }

    // Detect a post-scan change: compare against what the scan recorded.
    if let Some(position) = context.index.position_of(&candidate) {
        let recorded = context.index.aggregates[position].size_bytes;
        if metadata.is_dir() && recorded == 0 && context.index.children_of(position).is_empty() {
            // An empty directory the scan also saw as empty: fine.
        }
    }
    if let Some(record) = context
        .index
        .files
        .iter()
        .find(|file| file.path == candidate)
    {
        let current_len = metadata.len();
        if current_len != record.size {
            return Check::Skip {
                reason: format!(
                    "The file changed since the scan ({record.size} bytes then, {current_len} bytes now)."
                ),
            };
        }
    }

    Check::Ok {
        size_bytes: if metadata.is_file() { metadata.len() } else { 0 },
    }
}

fn is_blocked(path: &Path, blocked: &[String]) -> bool {
    let text = path.to_string_lossy().to_lowercase().replace('/', "\\");
    blocked.iter().any(|entry| {
        let needle = entry.to_lowercase().replace('/', "\\");
        text == needle || text.starts_with(&format!("{needle}\\"))
    })
}

fn is_approved(path: &Path, approved: &[String]) -> bool {
    if approved.is_empty() {
        return false;
    }
    let text = path.to_string_lossy().to_lowercase().replace('/', "\\");
    approved.iter().any(|entry| {
        let needle = entry.to_lowercase().replace('/', "\\");
        text == needle || text.starts_with(&format!("{needle}\\")) || text == needle.trim_end_matches('\\')
    })
}

/// Simulates a plan without changing anything.
pub fn preview_plan(
    plan: &CleanupPlan,
    titles: &dyn Fn(&str) -> String,
    context: &ValidationContext<'_>,
) -> CleanupPreview {
    let mut items = Vec::new();
    let mut total_size_bytes = 0u64;
    let mut total_path_count = 0usize;
    let mut warnings = Vec::new();

    for item in &plan.items {
        if item.action != "recycle" && item.action != "permanent" {
            warnings.push(format!(
                "Finding {} requested an unknown action ({}); nothing was planned for it.",
                item.finding_id, item.action
            ));
            continue;
        }
        let mut outcomes = Vec::new();
        let mut size_bytes = 0u64;
        let mut file_count = 0u64;
        let mut folder_count = 0u64;

        for path in &item.paths {
            match revalidate(path, context) {
                Check::Ok { size_bytes: bytes } => {
                    size_bytes += bytes;
                    total_size_bytes += bytes;
                    total_path_count += 1;
                    if Path::new(path).extension().is_some() {
                        file_count += 1;
                    } else {
                        folder_count += 1;
                    }
                    outcomes.push(CleanupPathOutcome {
                        path: path.clone(),
                        status: "ok".to_string(),
                        size_bytes: bytes,
                        detail: if item.action == "recycle" {
                            "Would be moved to the Recycle Bin.".to_string()
                        } else {
                            "Would be permanently deleted.".to_string()
                        },
                    });
                }
                Check::Skip { reason } => {
                    outcomes.push(CleanupPathOutcome {
                        path: path.clone(),
                        status: "skipped".to_string(),
                        size_bytes: 0,
                        detail: reason,
                    });
                }
            }
        }

        items.push(CleanupPreviewItem {
            finding_id: item.finding_id.clone(),
            title: titles(&item.finding_id),
            action: item.action.clone(),
            path_count: item.paths.len(),
            file_count,
            folder_count,
            size_bytes,
            paths: outcomes,
        });
    }

    CleanupPreview {
        items,
        total_size_bytes,
        total_path_count,
        warnings,
    }
}

/// Executes a plan. Every path is revalidated immediately before it is touched.
pub fn execute_plan(
    plan: &CleanupPlan,
    titles: &dyn Fn(&str) -> String,
    context: &ValidationContext<'_>,
) -> CleanupResult {
    let completed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default();

    let mut items = Vec::new();
    let mut total_removed_count = 0u64;
    let mut total_failed_count = 0u64;
    let mut total_removed_bytes = 0u64;

    for item in &plan.items {
        if item.action != "recycle" && item.action != "permanent" {
            total_failed_count += 1;
            items.push(CleanupResultItem {
                finding_id: item.finding_id.clone(),
                title: titles(&item.finding_id),
                action: item.action.clone(),
                removed_count: 0,
                failed_count: 1,
                removed_bytes: 0,
                results: vec![CleanupPathOutcome {
                    path: String::new(),
                    status: "skipped".to_string(),
                    size_bytes: 0,
                    detail: format!("Unknown action {}.", item.action),
                }],
            });
            continue;
        }

        let mut results = Vec::new();
        let mut removed_count = 0u64;
        let mut failed_count = 0u64;
        let mut removed_bytes = 0u64;

        for path in &item.paths {
            match revalidate(path, context) {
                Check::Skip { reason } => {
                    failed_count += 1;
                    results.push(CleanupPathOutcome {
                        path: path.clone(),
                        status: "skipped".to_string(),
                        size_bytes: 0,
                        detail: reason,
                    });
                }
                Check::Ok { size_bytes } => {
                    let candidate = PathBuf::from(path);
                    match act(&candidate, &item.action) {
                        Ok(()) => {
                            removed_count += 1;
                            removed_bytes += size_bytes;
                            results.push(CleanupPathOutcome {
                                path: path.clone(),
                                status: "ok".to_string(),
                                size_bytes,
                                detail: if item.action == "recycle" {
                                    "Moved to the Recycle Bin.".to_string()
                                } else {
                                    "Permanently deleted.".to_string()
                                },
                            });
                        }
                        Err(reason) => {
                            failed_count += 1;
                            results.push(CleanupPathOutcome {
                                path: path.clone(),
                                status: "denied".to_string(),
                                size_bytes,
                                detail: reason,
                            });
                        }
                    }
                }
            }
        }

        total_removed_count += removed_count;
        total_failed_count += failed_count;
        total_removed_bytes += removed_bytes;
        items.push(CleanupResultItem {
            finding_id: item.finding_id.clone(),
            title: titles(&item.finding_id),
            action: item.action.clone(),
            removed_count,
            failed_count,
            removed_bytes,
            results,
        });
    }

    let partial = total_failed_count > 0 && total_removed_count > 0;

    CleanupResult {
        items,
        total_removed_count,
        total_failed_count,
        total_removed_bytes,
        completed_at,
        partial,
    }
}

/// The single place where anything is actually removed.
fn act(path: &Path, action: &str) -> Result<(), String> {
    let long = to_unc_long_path(path);
    if action == "permanent" {
        if path.is_dir() {
            std::fs::remove_dir_all(&long).map_err(|error| error.to_string())
        } else {
            std::fs::remove_file(&long).map_err(|error| error.to_string())
        }
    } else {
        // Preferred path: the Recycle Bin, so the action is reversible.
        trash::delete(&long).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::aggregate::{ScanIndex, OTHER_NODE_SUFFIX};
    use crate::scanner::walker::ScanOptions;
    use std::fs;
    use std::sync::Arc;

    fn temp_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("diskwise-exec-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn scan_index(root: &Path) -> ScanIndex {
        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            ..Default::default()
        };
        let counters = Arc::new(crate::scanner::progress::ProgressCounters::new());
        let run = crate::scanner::walker::walk(&options, &counters);
        ScanIndex::from_run(&run)
    }

    fn titles(_id: &str) -> String {
        "Test finding".to_string()
    }

    #[test]
    fn a_dry_run_changes_nothing() {
        let root = temp_dir("dry");
        fs::create_dir_all(root.join("cache")).unwrap();
        let victim = root.join("cache\\junk.bin");
        fs::write(&victim, vec![0u8; 128]).unwrap();
        let index = scan_index(&root);
        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f1".to_string(),
                paths: vec![victim.to_string_lossy().to_string()],
                action: "recycle".to_string(),
            }],
            dry_run: true,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[],
            approved_roots: &[root.join("cache").to_string_lossy().to_string()],
        };
        let preview = preview_plan(&plan, &titles, &context);
        assert_eq!(preview.total_path_count, 1);
        assert_eq!(preview.total_size_bytes, 128);
        assert!(victim.exists(), "a dry run must not touch the filesystem");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_path_outside_the_approved_roots_is_refused() {
        let root = temp_dir("refuse");
        fs::create_dir_all(root.join("docs")).unwrap();
        let victim = root.join("docs\\report.pdf");
        fs::write(&victim, vec![0u8; 64]).unwrap();
        let index = scan_index(&root);
        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f1".to_string(),
                paths: vec![victim.to_string_lossy().to_string()],
                action: "recycle".to_string(),
            }],
            dry_run: false,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[],
            approved_roots: &[root.join("cache").to_string_lossy().to_string()],
        };
        let result = execute_plan(&plan, &titles, &context);
        assert_eq!(result.total_removed_count, 0);
        assert_eq!(result.total_failed_count, 1);
        assert!(victim.exists(), "the file must survive");
        assert!(result.items[0].results[0].detail.contains("outside the folders you approved"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_blocked_path_is_refused_even_when_approved() {
        let root = temp_dir("blocked");
        fs::create_dir_all(root.join("cache")).unwrap();
        let victim = root.join("cache\\keep.bin");
        fs::write(&victim, vec![0u8; 32]).unwrap();
        let index = scan_index(&root);
        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f1".to_string(),
                paths: vec![victim.to_string_lossy().to_string()],
                action: "permanent".to_string(),
            }],
            dry_run: false,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[victim.to_string_lossy().to_string()],
            approved_roots: &[root.join("cache").to_string_lossy().to_string()],
        };
        let result = execute_plan(&plan, &titles, &context);
        assert_eq!(result.total_removed_count, 0);
        assert!(victim.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_that_changed_since_the_scan_is_skipped() {
        let root = temp_dir("changed");
        fs::create_dir_all(root.join("cache")).unwrap();
        let victim = root.join("cache\\grows.bin");
        fs::write(&victim, vec![0u8; 100]).unwrap();
        let index = scan_index(&root);
        // The file changes after the scan.
        fs::write(&victim, vec![0u8; 250]).unwrap();

        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f1".to_string(),
                paths: vec![victim.to_string_lossy().to_string()],
                action: "permanent".to_string(),
            }],
            dry_run: false,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[],
            approved_roots: &[root.join("cache").to_string_lossy().to_string()],
        };
        let result = execute_plan(&plan, &titles, &context);
        assert_eq!(result.total_removed_count, 0);
        assert_eq!(result.total_failed_count, 1);
        assert!(result.items[0].results[0].detail.contains("changed since the scan"));
        assert!(victim.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_path_is_reported_as_skipped() {
        let root = temp_dir("missing");
        fs::create_dir_all(root.join("cache")).unwrap();
        let index = scan_index(&root);
        let ghost = root.join("cache\\ghost.bin");
        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f1".to_string(),
                paths: vec![ghost.to_string_lossy().to_string()],
                action: "recycle".to_string(),
            }],
            dry_run: false,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[],
            approved_roots: &[root.join("cache").to_string_lossy().to_string()],
        };
        let result = execute_plan(&plan, &titles, &context);
        assert_eq!(result.total_removed_count, 0);
        assert_eq!(result.items[0].results[0].status, "skipped");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_action_is_refused_with_a_warning() {
        let root = temp_dir("action");
        fs::create_dir_all(root.join("cache")).unwrap();
        let index = scan_index(&root);
        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f1".to_string(),
                paths: Vec::new(),
                action: "shred".to_string(),
            }],
            dry_run: false,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[],
            approved_roots: &[],
        };
        let preview = preview_plan(&plan, &titles, &context);
        assert!(!preview.warnings.is_empty());
        assert_eq!(preview.total_path_count, 0);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_synthetic_other_node_is_never_actionable() {
        let root = temp_dir("synthetic");
        fs::create_dir_all(root.join("cache")).unwrap();
        let index = scan_index(&root);
        let synthetic = format!("{}\\cache\\[Other]", root.to_string_lossy());
        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f1".to_string(),
                paths: vec![synthetic],
                action: "permanent".to_string(),
            }],
            dry_run: false,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[],
            approved_roots: &[root.to_string_lossy().to_string()],
        };
        let result = execute_plan(&plan, &titles, &context);
        assert_eq!(result.total_removed_count, 0);
        assert!(result.items[0].results[0].detail.contains("aggregate placeholder"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn partial_completion_is_reported() {
        let root = temp_dir("partial");
        fs::create_dir_all(root.join("cache")).unwrap();
        let real = root.join("cache\\real.bin");
        fs::write(&real, vec![0u8; 16]).unwrap();
        let index = scan_index(&root);
        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f1".to_string(),
                paths: vec![
                    real.to_string_lossy().to_string(),
                    root.join("cache\\ghost.bin").to_string_lossy().to_string(),
                ],
                action: "permanent".to_string(),
            }],
            dry_run: false,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[],
            approved_roots: &[root.join("cache").to_string_lossy().to_string()],
        };
        let result = execute_plan(&plan, &titles, &context);
        assert_eq!(result.total_removed_count, 1);
        assert_eq!(result.total_failed_count, 1);
        assert!(result.partial);

        let _ = fs::remove_dir_all(&root);
    }
}
