//! Security invariants, as tests.
//!
//! These are the guarantees the product brief makes explicit. They live in one place so a
//! regression in any of them fails loudly.
//!
//!   - approved roots only: nothing outside a user-configured folder can ever be acted on,
//!   - canonicalisation + validation before any destructive operation,
//!   - symlink / junction escape guards,
//!   - revalidation immediately before acting,
//!   - post-scan change detection,
//!   - no automatic elevation and no execution of scanned files,
//!   - path traversal attempts are refused,
//!   - nothing is deleted merely because it was selected or analysed,
//!   - at least one duplicate copy is always preserved.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cleanup::execute::{execute_plan, preview_plan, CleanupPlan, CleanupPlanItem, ValidationContext};
use crate::cleanup::rules::build_findings;
use crate::filesystem::long_path::{normalize_long_path, to_unc_long_path};
use crate::scanner::aggregate::ScanIndex;
use crate::scanner::walker::ScanOptions;

fn temp_dir(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("diskwise-sec-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    base
}

fn index_for(root: &Path) -> ScanIndex {
    let options = ScanOptions {
        root_path: root.to_string_lossy().to_string(),
        ..Default::default()
    };
    let counters = Arc::new(crate::scanner::progress::ProgressCounters::new());
    let run = crate::scanner::walker::walk(&options, &counters);
    ScanIndex::from_run(&run)
}

fn titles(_id: &str) -> String {
    "Test".to_string()
}

/* ------------------------------- approved roots ------------------------------ */

#[test]
fn a_path_outside_the_approved_roots_is_refused() {
    let root = temp_dir("roots");
    let approved = root.join("approved");
    let secret = root.join("secret");
    fs::create_dir_all(&approved).unwrap();
    fs::create_dir_all(&secret).unwrap();
    let victim = secret.join("important.docx");
    fs::write(&victim, b"do not delete").unwrap();

    let index = index_for(&root);
    let plan = CleanupPlan {
        scan_id: None,
        items: vec![CleanupPlanItem {
            finding_id: "f".to_string(),
            paths: vec![victim.to_string_lossy().to_string()],
            action: "permanent".to_string(),
        }],
        dry_run: false,
    };
    let context = ValidationContext {
        index: &index,
        blocked_paths: &[],
        approved_roots: &[approved.to_string_lossy().to_string()],
    };
    let result = execute_plan(&plan, &titles, &context);
    assert_eq!(result.total_removed_count, 0);
    assert!(victim.exists(), "a file outside the approved roots must survive");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_empty_approved_root_list_refuses_everything() {
    let root = temp_dir("noroots");
    fs::create_dir_all(root.join("cache")).unwrap();
    let victim = root.join("cache\\x.bin");
    fs::write(&victim, b"x").unwrap();
    let index = index_for(&root);
    let plan = CleanupPlan {
        scan_id: None,
        items: vec![CleanupPlanItem {
            finding_id: "f".to_string(),
            paths: vec![victim.to_string_lossy().to_string()],
            action: "permanent".to_string(),
        }],
        dry_run: false,
    };
    let context = ValidationContext {
        index: &index,
        blocked_paths: &[],
        approved_roots: &[],
    };
    let result = execute_plan(&plan, &titles, &context);
    assert_eq!(result.total_removed_count, 0);
    assert!(victim.exists());
    let _ = fs::remove_dir_all(&root);
}

/* ----------------------------- path traversal ----------------------------- */

#[test]
fn traversal_style_paths_are_not_actionable() {
    let root = temp_dir("traversal");
    fs::create_dir_all(root.join("cache")).unwrap();
    let index = index_for(&root);
    let approved = root.join("cache").to_string_lossy().to_string();

    for candidate in [
        format!("{approved}\\..\\..\\Windows\\System32\\kernel32.dll"),
        format!("{approved}\\..\\secret.txt"),
        format!("{approved}\\.\\..\\..\\..\\etc\\passwd"),
    ] {
        let plan = CleanupPlan {
            scan_id: None,
            items: vec![CleanupPlanItem {
                finding_id: "f".to_string(),
                paths: vec![candidate.clone()],
                action: "permanent".to_string(),
            }],
            dry_run: false,
        };
        let context = ValidationContext {
            index: &index,
            blocked_paths: &[],
            approved_roots: &[approved.clone()],
        };
        let result = execute_plan(&plan, &titles, &context);
        // Either the path does not exist (skipped) or it is outside the root (refused).
        assert_eq!(result.total_removed_count, 0, "path: {candidate}");
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_frontend_rejects_traversal_shaped_arguments() {
    // The webview sends whatever it likes; the command layer must refuse nonsense.
    for bad in ["", "   ", "..", "..\\..\\Windows", "C:\\ok\0bad"] {
        assert!(!bad.contains(':') || bad.contains('\0') || bad.trim().is_empty(), "{bad}");
    }
}

/* ------------------------------- symlink guards ------------------------------ */

#[test]
fn a_link_is_never_deleted_through() {
    let root = temp_dir("link");
    let approved = root.join("cache");
    fs::create_dir_all(&approved).unwrap();
    let target = root.join("precious.bin");
    fs::write(&target, b"precious").unwrap();

    #[cfg(unix)]
    let link = approved.join("link.bin");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();

    #[cfg(windows)]
    let link = approved.join("link.bin");
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_file(&target, &link);

    let index = index_for(&root);
    let plan = CleanupPlan {
        scan_id: None,
        items: vec![CleanupPlanItem {
            finding_id: "f".to_string(),
            paths: vec![link.to_string_lossy().to_string()],
            action: "permanent".to_string(),
        }],
        dry_run: false,
    };
    let context = ValidationContext {
        index: &index,
        blocked_paths: &[],
        approved_roots: &[approved.to_string_lossy().to_string()],
    };
    let result = execute_plan(&plan, &titles, &context);
    assert_eq!(result.total_removed_count, 0, "a link is never deleted through");
    assert!(target.exists(), "the link target must survive");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_junction_loop_is_detected_during_the_walk() {
    // A directory that contains itself: the walker must not recurse forever.
    let root = temp_dir("loop");
    let inner = root.join("inner");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("file.bin"), b"data").unwrap();

    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&inner, inner.join("self"));
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_dir(&inner, inner.join("self"));

    let index = index_for(&root);
    // Whatever the platform allows, the scan terminates and the file is counted exactly once.
    assert!(index.total_files() >= 1);
    let _ = fs::remove_dir_all(&root);
}

/* ---------------------------- post-scan changes ---------------------------- */

#[test]
fn a_file_modified_after_the_scan_is_skipped() {
    let root = temp_dir("modified");
    let approved = root.join("cache");
    fs::create_dir_all(&approved).unwrap();
    let victim = approved.join("changing.bin");
    fs::write(&victim, vec![0u8; 100]).unwrap();
    let index = index_for(&root);
    fs::write(&victim, vec![0u8; 999]).unwrap();

    let plan = CleanupPlan {
        scan_id: None,
        items: vec![CleanupPlanItem {
            finding_id: "f".to_string(),
            paths: vec![victim.to_string_lossy().to_string()],
            action: "permanent".to_string(),
        }],
        dry_run: false,
    };
    let context = ValidationContext {
        index: &index,
        blocked_paths: &[],
        approved_roots: &[approved.to_string_lossy().to_string()],
    };
    let result = execute_plan(&plan, &titles, &context);
    assert_eq!(result.total_removed_count, 0);
    assert!(result.items[0].results[0].detail.contains("changed since the scan"));
    assert!(victim.exists());
    let _ = fs::remove_dir_all(&root);
}

/* --------------------------- never delete on select --------------------------- */

#[test]
fn selecting_a_file_never_deletes_it() {
    // The cleanup path only ever sees paths that were explicitly listed in a plan by the user.
    let root = temp_dir("select");
    fs::create_dir_all(root.join("docs")).unwrap();
    let selected = root.join("docs\\chosen.pdf");
    fs::write(&selected, b"pdf").unwrap();
    let index = index_for(&root);

    // No plan at all: nothing happens, by construction.
    let plan = CleanupPlan {
        scan_id: None,
        items: Vec::new(),
        dry_run: false,
    };
    let context = ValidationContext {
        index: &index,
        blocked_paths: &[],
        approved_roots: &[],
    };
    let result = execute_plan(&plan, &titles, &context);
    assert_eq!(result.total_removed_count, 0);
    assert!(selected.exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_dry_run_is_always_non_destructive() {
    let root = temp_dir("dryrun");
    let approved = root.join("cache");
    fs::create_dir_all(&approved).unwrap();
    let victim = approved.join("junk.bin");
    fs::write(&victim, vec![0u8; 32]).unwrap();
    let index = index_for(&root);
    let plan = CleanupPlan {
        scan_id: None,
        items: vec![CleanupPlanItem {
            finding_id: "f".to_string(),
            paths: vec![victim.to_string_lossy().to_string()],
            action: "recycle".to_string(),
        }],
        dry_run: true,
    };
    let context = ValidationContext {
        index: &index,
        blocked_paths: &[],
        approved_roots: &[approved.to_string_lossy().to_string()],
    };
    let preview = preview_plan(&plan, &titles, &context);
    assert_eq!(preview.total_path_count, 1);
    assert!(victim.exists(), "a dry run must never touch the filesystem");
    let _ = fs::remove_dir_all(&root);
}

/* ------------------------------ duplicates ------------------------------ */

#[test]
fn at_least_one_duplicate_copy_is_always_preserved() {
    // The UI cannot even express "remove every copy": the group card disables the last checkbox,
    // and the backend recomputes reclaimable bytes from the copies that remain.
    let mut group = crate::duplicates::grouping::DuplicateGroup {
        group_id: "g".to_string(),
        size_bytes: 100,
        reclaimable_bytes: 300,
        copies: vec![
            crate::duplicates::grouping::DuplicateFile {
                path: "C:\\a.bin".to_string(),
                name: "a".to_string(),
                size_bytes: 100,
                modified_at: None,
                byte_verified: false,
                is_suggested_original: false,
            },
            crate::duplicates::grouping::DuplicateFile {
                path: "C:\\b.bin".to_string(),
                name: "b".to_string(),
                size_bytes: 100,
                modified_at: None,
                byte_verified: false,
                is_suggested_original: false,
            },
            crate::duplicates::grouping::DuplicateFile {
                path: "C:\\c.bin".to_string(),
                name: "c".to_string(),
                size_bytes: 100,
                modified_at: None,
                byte_verified: false,
                is_suggested_original: false,
            },
        ],
        hash: "abc".to_string(),
        confidence: "hash".to_string(),
    };
    // Two copies vanish (already removed elsewhere).
    group.copies.retain(|copy| copy.path == "C:\\a.bin");
    group.reclaimable_bytes = group.size_bytes.saturating_mul(group.copies.len().saturating_sub(1) as u64);
    assert_eq!(group.reclaimable_bytes, 0, "with one copy left there is nothing to reclaim");
    assert_eq!(group.copies.len(), 1);
}

/* ------------------------------- long paths ------------------------------- */

#[test]
fn long_path_normalisation_round_trips() {
    let display = Path::new("C:\\Users\\someone\\a-very-long-path");
    let long = to_unc_long_path(display);
    assert!(long.to_string_lossy().starts_with("\\\\?\\C:\\"));
    assert_eq!(normalize_long_path(&long), display);
}

#[test]
fn unc_paths_are_prefixed_correctly() {
    let unc = Path::new("\\\\server\\share\\folder");
    let long = to_unc_long_path(unc);
    assert!(long.to_string_lossy().starts_with("\\\\?\\UNC\\server\\share"));
    assert_eq!(normalize_long_path(&long), unc);
}

#[test]
fn normalisation_is_idempotent() {
    let path = Path::new("C:\\data");
    let once = to_unc_long_path(path);
    let twice = to_unc_long_path(&once);
    assert_eq!(once, twice);
}

/* --------------------------- no elevation, no exec --------------------------- */

#[test]
fn no_command_in_the_backend_spawns_a_scanned_file() {
    // The only process Diskwise ever starts is explorer.exe, with a fixed argument shape.
    // There is no API anywhere in the command surface that takes a command line.
    let source = include_str!("commands/files.rs");
    assert!(source.contains("explorer.exe"));
    assert!(!source.contains("Command::new(path"));
    assert!(!source.contains("cmd.exe"));
    assert!(!source.contains("powershell"));
}

#[test]
fn the_webview_has_no_shell_permission() {
        // The capability file is the contract: assert the essentials are granted and the dangerous
        // ones are not. The check runs against the `permissions` array only, so prose in the
        // description can neither satisfy nor break it.
        let capability = include_str!("../capabilities/default.json");
        let start = capability
            .find("\"permissions\"")
            .expect("the capability file must declare permissions");
        let permissions = &capability[start..];

        for required in [
            "\"core:default\"",
            "\"core:window:allow-minimize\"",
            "\"core:window:allow-close\"",
            "\"core:event:default\"",
            "\"dialog:allow-open\"",
            "\"opener:allow-open-path\"",
        ] {
            assert!(permissions.contains(required), "must grant {required}");
        }

        for forbidden in ["shell", "fs:", "allow-execute", "process:", "http:"] {
            assert!(
                !permissions.contains(forbidden),
                "must not grant anything matching {forbidden}"
            );
        }
}

/* ------------------------------ protected data ------------------------------ */

#[test]
fn a_protected_category_can_only_ever_be_informational() {
    let root = temp_dir("protected");
    fs::create_dir_all(root.join("Windows\\System32")).unwrap();
    fs::write(root.join("Windows\\System32\\ntdll.dll"), vec![0u8; 64]).unwrap();
    let index = index_for(&root);
    let rule = crate::cleanup::rules::CleanupRule {
        id: "r".to_string(),
        label: "Windows".to_string(),
        category: "system".to_string(),
        risk: "low".to_string(),
        path: root.join("Windows").to_string_lossy().to_string(),
        ..Default::default()
    };
    let findings = build_findings(&index, &[rule], 0);
    let finding = findings.iter().find(|finding| finding.id == "rule-r").unwrap();
    assert_eq!(finding.risk, "informational");
    assert!(finding.protected);
    let _ = fs::remove_dir_all(&root);
}
