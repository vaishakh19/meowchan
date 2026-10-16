//! Rules → findings.
//!
//! A finding explains itself: what it is, why it was suggested, exactly which paths it covers and
//! how many bytes they add up to. Protected categories (documents, browser profiles, development
//! dependencies, package caches, application data) can only ever be informational.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::filesystem::classify;
use crate::scanner::aggregate::ScanIndex;

/// Mirrors `CleanupRule` in `src/types/contracts.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CleanupRule {
    pub id: String,
    pub label: String,
    pub category: String,
    pub risk: String,
    /// Absolute directory the rule applies to. Must be inside an approved root.
    pub path: String,
    /// `0` = any age.
    pub older_than_days: u32,
    /// `0` = any size.
    pub min_bytes: u64,
    /// Lower-case extensions to include. Empty = all.
    pub extensions: Vec<String>,
    pub recursive: bool,
    pub enabled: bool,
}

impl Default for CleanupRule {
    fn default() -> Self {
        CleanupRule {
            id: String::new(),
            label: String::new(),
            category: "custom".to_string(),
            risk: "review".to_string(),
            path: String::new(),
            older_than_days: 0,
            min_bytes: 0,
            extensions: Vec::new(),
            recursive: true,
            enabled: true,
        }
    }
}

/// Mirrors `CleanupFinding`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupFinding {
    pub id: String,
    pub category: String,
    pub risk: String,
    pub title: String,
    pub reason: String,
    /// Protected findings can only ever be informational.
    pub protected: bool,
    pub path: Option<String>,
    pub paths: Vec<String>,
    pub file_count: u64,
    pub folder_count: u64,
    pub size_bytes: u64,
    pub disk_size_bytes: Option<u64>,
    pub oldest_modified_at: Option<i64>,
    pub newest_modified_at: Option<i64>,
    pub rule_id: Option<String>,
}

/// Mirrors `RecycleBinSummary`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleBinSummary {
    pub item_count: u64,
    pub size_bytes: Option<u64>,
    pub supported: bool,
    pub detail: String,
}

const DAY_MS: i64 = 86_400_000;

/// Categories that may never be offered for deletion by a built-in rule.
const PROTECTED_CATEGORIES: &[&str] = &[
    "system",
    "applications",
    "documents",
    "development",
    "games",
];

/// Builds findings from the scan index and the user's rules.
///
/// Built-in suggestions only cover the Recycle Bin and anything inside a user-configured rule path.
/// Everything else the scan found is reported as informational with an explanation.
pub fn build_findings(index: &ScanIndex, rules: &[CleanupRule], now_ms: i64) -> Vec<CleanupFinding> {
    let mut findings: Vec<CleanupFinding> = Vec::new();

    for rule in rules.iter().filter(|rule| rule.enabled) {
        if let Some(finding) = finding_for_rule(index, rule, now_ms) {
            findings.push(finding);
        }
    }

    // The Recycle Bin is always offered as a low-risk suggestion when the platform reports it.
    let bin = crate::platform::recycle_bin(None);
    findings.push(CleanupFinding {
        id: "recycle-bin".to_string(),
        category: "recycle-bin".to_string(),
        risk: if bin.supported && bin.item_count > 0 {
            "low".to_string()
        } else {
            "informational".to_string()
        },
        title: "Recycle Bin".to_string(),
        reason: if bin.supported {
            "Emptying the Recycle Bin reclaims space immediately. Items stay recoverable until you empty it, and Windows asks for confirmation itself.".to_string()
        } else {
            "Windows did not report the Recycle Bin contents for this session.".to_string()
        },
        protected: false,
        path: None,
        paths: Vec::new(),
        file_count: 0,
        folder_count: 0,
        size_bytes: bin.size_bytes,
        disk_size_bytes: None,
        oldest_modified_at: None,
        newest_modified_at: None,
        rule_id: None,
    });

    // Informational summary of what Diskwise refuses to touch.
    let protected = summarise_protected(index);
    if protected.file_count > 0 || protected.folder_count > 0 {
        findings.push(CleanupFinding {
            id: "protected-summary".to_string(),
            category: "system".to_string(),
            risk: "informational".to_string(),
            title: "System, application and development data".to_string(),
            reason: format!(
                "Diskwise found {} files and {} folders in categories it will never offer for deletion: Windows system files, installed applications, documents, games and development dependencies. They are listed here so you can see where the space is, not because they should be removed.",
                protected.file_count, protected.folder_count
            ),
            protected: true,
            path: None,
            paths: Vec::new(),
            file_count: protected.file_count,
            folder_count: protected.folder_count,
            size_bytes: protected.size_bytes,
            disk_size_bytes: None,
            oldest_modified_at: None,
            newest_modified_at: None,
            rule_id: None,
        });
    }

    findings.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    findings
}

struct ProtectedSummary {
    file_count: u64,
    folder_count: u64,
    size_bytes: u64,
}

fn summarise_protected(index: &ScanIndex) -> ProtectedSummary {
    let mut summary = ProtectedSummary {
        file_count: 0,
        folder_count: 0,
        size_bytes: 0,
    };
    for (position, directory) in index.directories.iter().enumerate() {
        if position == 0 {
            continue;
        }
        let category = classify(&directory.path, true);
        if PROTECTED_CATEGORIES.contains(&category.as_str()) {
            summary.folder_count += 1;
            summary.size_bytes += index.aggregates[position].size_bytes;
        }
    }
    for file in &index.files {
        let category = classify(&file.path, false);
        if PROTECTED_CATEGORIES.contains(&category.as_str()) {
            summary.file_count += 1;
            summary.size_bytes += file.size;
        }
    }
    summary
}

fn finding_for_rule(index: &ScanIndex, rule: &CleanupRule, now_ms: i64) -> Option<CleanupFinding> {
    let root = PathBuf::from(&rule.path);
    let position = index.position_of(&root)?;
    let cutoff = if rule.older_than_days == 0 {
        None
    } else {
        Some(now_ms - rule.older_than_days as i64 * DAY_MS)
    };

    let mut paths: Vec<String> = Vec::new();
    let mut size_bytes = 0u64;
    let mut file_count = 0u64;
    let mut folder_count = 0u64;
    let mut oldest: Option<i64> = None;
    let mut newest: Option<i64> = None;

    // Walk the subtree (iteratively) collecting the files the rule selects.
    let mut stack = vec![position];
    while let Some(current) = stack.pop() {
        for file in index.files_of(current) {
            if let Some(cutoff) = cutoff {
                match file.modified_at {
                    Some(modified) if modified <= cutoff => {}
                    // A file with no timestamp is not provably old, so it is left alone.
                    _ => continue,
                }
            }
            if rule.min_bytes > 0 && file.size < rule.min_bytes {
                continue;
            }
            if !rule.extensions.is_empty() {
                match &file.extension {
                    Some(extension) if rule.extensions.iter().any(|e| e == extension) => {}
                    _ => continue,
                }
            }
            paths.push(file.path.to_string_lossy().to_string());
            size_bytes += file.size;
            file_count += 1;
            if let Some(modified) = file.modified_at {
                oldest = Some(oldest.map_or(modified, |value: i64| value.min(modified)));
                newest = Some(newest.map_or(modified, |value: i64| value.max(modified)));
            }
        }
        if rule.recursive {
            for child in index.children_of(current) {
                stack.push(*child);
            }
        }
    }

    if file_count == 0 {
        return None;
    }

    // A rule is never allowed to lower itself below "needs review", and a protected category can
    // only ever be informational.
    let protected = PROTECTED_CATEGORIES.contains(&rule.category.as_str())
        || rule.category == "system"
        || rule.category == "applications";
    let risk = if protected {
        "informational".to_string()
    } else {
        match rule.risk.as_str() {
            "low" => "low".to_string(),
            _ => "review".to_string(),
        }
    };

    let reason = build_reason(rule, file_count, folder_count_hint(&paths), cutoff, now_ms);

    Some(CleanupFinding {
        id: format!("rule-{}", rule.id),
        category: rule.category.clone(),
        risk,
        title: rule.label.clone(),
        reason,
        protected,
        path: Some(rule.path.clone()),
        paths,
        file_count,
        folder_count,
        size_bytes,
        disk_size_bytes: None,
        oldest_modified_at: oldest,
        newest_modified_at: newest,
        rule_id: Some(rule.id.clone()),
    })
}

fn folder_count_hint(paths: &[String]) -> u64 {
    paths
        .iter()
        .filter_map(|path| Path::new(path).parent())
        .collect::<std::collections::HashSet<_>>()
        .len() as u64
}

fn build_reason(rule: &CleanupRule, file_count: u64, folder_count: u64, cutoff: Option<i64>, _now: i64) -> String {
    let mut parts: Vec<String> = vec![format!(
        "{} files in {} folder(s) inside {} match your rule.",
        file_count, folder_count, rule.path
    )];
    if let Some(cutoff) = cutoff {
        let days = rule.older_than_days;
        parts.push(format!("Only files last modified more than {days} days ago are included."));
        let _ = cutoff;
    }
    if rule.min_bytes > 0 {
        parts.push(format!("Only files at least {} bytes are included.", rule.min_bytes));
    }
    if !rule.extensions.is_empty() {
        parts.push(format!("Only these extensions: {}.", rule.extensions.join(", ")));
    }
    parts.push("Nothing here is deleted automatically.".to_string());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::aggregate::ScanIndex;
    use crate::scanner::walker::{ScanOptions, WalkError};
    use std::fs;
    use std::sync::Arc;

    fn temp_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("diskwise-cleanup-{name}-{}", std::process::id()));
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

    fn now() -> i64 {
        1_700_000_000_000
    }

    #[test]
    fn a_rule_only_selects_files_inside_its_own_directory() {
        let root = temp_dir("scope");
        fs::create_dir_all(root.join("temp")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("temp\\junk.tmp"), vec![0u8; 100]).unwrap();
        fs::write(root.join("docs\\report.pdf"), vec![0u8; 500]).unwrap();

        let index = scan_index(&root);
        let rule = CleanupRule {
            id: "r1".to_string(),
            label: "Temporary files".to_string(),
            category: "temp".to_string(),
            risk: "low".to_string(),
            path: root.join("temp").to_string_lossy().to_string(),
            ..Default::default()
        };
        let findings = build_findings(&index, &[rule], now());
        let finding = findings
            .iter()
            .find(|finding| finding.id == "rule-r1")
            .expect("a finding for the rule");
        assert_eq!(finding.file_count, 1);
        assert_eq!(finding.size_bytes, 100);
        assert!(finding.paths[0].contains("temp"));
        assert!(!finding.paths.iter().any(|path| path.contains("docs")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_age_rule_ignores_recent_files() {
        let root = temp_dir("age");
        fs::create_dir_all(root.join("cache")).unwrap();
        let old = root.join("cache\\old.bin");
        let new = root.join("cache\\new.bin");
        fs::write(&old, vec![0u8; 10]).unwrap();
        fs::write(&new, vec![0u8; 10]).unwrap();
        // Make one file old by touching its timestamp far in the past.
        let past = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        let file = fs::OpenOptions::new().write(true).open(&old).unwrap();
        let _ = file.set_times(fs::FileTimes::new().set_modified(past).set_accessed(past));

        let index = scan_index(&root);
        let rule = CleanupRule {
            id: "r2".to_string(),
            label: "Old cache".to_string(),
            category: "cache".to_string(),
            risk: "low".to_string(),
            path: root.join("cache").to_string_lossy().to_string(),
            older_than_days: 365,
            ..Default::default()
        };
        let findings = build_findings(&index, &[rule], now());
        let finding = findings
            .iter()
            .find(|finding| finding.id == "rule-r2")
            .expect("a finding for the age rule");
        assert_eq!(finding.file_count, 1, "only the old file matches");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_protected_category_is_always_informational() {
        let root = temp_dir("protected");
        fs::create_dir_all(root.join("Windows\\System32")).unwrap();
        fs::write(root.join("Windows\\System32\\kernel.dll"), vec![0u8; 42]).unwrap();

        let index = scan_index(&root);
        let rule = CleanupRule {
            id: "r3".to_string(),
            label: "Windows".to_string(),
            category: "system".to_string(),
            risk: "low".to_string(),
            path: root.join("Windows").to_string_lossy().to_string(),
            ..Default::default()
        };
        let findings = build_findings(&index, &[rule], now());
        let finding = findings
            .iter()
            .find(|finding| finding.id == "rule-r3")
            .expect("a finding for the protected rule");
        assert_eq!(finding.risk, "informational");
        assert!(finding.protected);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_rule_outside_the_scan_produces_no_finding() {
        let root = temp_dir("outside");
        fs::write(root.join("a.bin"), vec![0u8; 4]).unwrap();
        let index = scan_index(&root);
        let rule = CleanupRule {
            id: "r4".to_string(),
            label: "Elsewhere".to_string(),
            path: "Z:\\not-scanned".to_string(),
            ..Default::default()
        };
        assert!(build_findings(&index, &[rule], now())
            .iter()
            .all(|finding| finding.id != "rule-r4"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_rule_with_no_matches_produces_no_finding() {
        let root = temp_dir("empty");
        fs::create_dir_all(root.join("cache")).unwrap();
        fs::write(root.join("cache\\keep.bin"), vec![0u8; 10]).unwrap();
        let index = scan_index(&root);
        let rule = CleanupRule {
            id: "r5".to_string(),
            label: "Big files".to_string(),
            path: root.join("cache").to_string_lossy().to_string(),
            min_bytes: 10_000,
            ..Default::default()
        };
        assert!(build_findings(&index, &[rule], now())
            .iter()
            .all(|finding| finding.id != "rule-r5"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_recycle_bin_finding_is_always_present() {
        let root = temp_dir("bin");
        fs::write(root.join("a.bin"), vec![0u8; 4]).unwrap();
        let index = scan_index(&root);
        let findings = build_findings(&index, &[], now());
        assert!(findings.iter().any(|finding| finding.id == "recycle-bin"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn findings_are_sorted_by_size_descending() {
        let root = temp_dir("sorted");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a\\small.bin"), vec![0u8; 10]).unwrap();
        fs::write(root.join("b\\big.bin"), vec![0u8; 10_000]).unwrap();
        let index = scan_index(&root);
        let rules = vec![
            CleanupRule {
                id: "a".to_string(),
                label: "A".to_string(),
                path: root.join("a").to_string_lossy().to_string(),
                ..Default::default()
            },
            CleanupRule {
                id: "b".to_string(),
                label: "B".to_string(),
                path: root.join("b").to_string_lossy().to_string(),
                ..Default::default()
            },
        ];
        let findings = build_findings(&index, &rules, now());
        let sizes: Vec<u64> = findings.iter().map(|finding| finding.size_bytes).collect();
        assert!(sizes.windows(2).all(|pair| pair[0] >= pair[1]));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_scan_index_still_reports_the_recycle_bin() {
        let index = ScanIndex::default();
        let findings = build_findings(&index, &[], now());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "recycle-bin");
    }

    #[test]
    fn walk_errors_are_never_invented() {
        let error = WalkError {
            path: "C:\\x".to_string(),
            kind: "not-found".to_string(),
            message: "gone".to_string(),
        };
        assert_eq!(error.kind, "not-found");
    }
}
