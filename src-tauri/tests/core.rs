//! Real filesystem integration tests, including the serialized IPC contract.
use std::{fs, sync::Arc};
use diskwise_lib::{scanner::{walker::{walk, ScanOptions}, progress::ProgressCounters, aggregate::ScanIndex}, duplicates::grouping::{build_report, DuplicateOptions}, state::snapshot};

#[test]
fn scans_real_files_and_finds_content_duplicates_not_just_equal_sizes() {
    let root = std::env::temp_dir().join(format!("diskora-core-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("nested-文件")).unwrap();
    fs::write(root.join("a.bin"), b"same").unwrap();
    fs::write(root.join("b.bin"), b"same").unwrap();
    fs::write(root.join("c.bin"), b"diff").unwrap();
    fs::write(root.join("nested-文件/empty"), b"").unwrap();
    fs::write(root.join("nested-文件/small.bin"), [1, 2, 3]).unwrap();
    let options = ScanOptions { root_path: root.to_string_lossy().into_owned(), ..Default::default() };
    let counters = Arc::new(ProgressCounters::new());
    let run = walk(&options, &counters);
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert!(!run.hit_limit);
    let index = ScanIndex::from_run(&run);
    assert_eq!(index.total_bytes(), 15);
    assert_eq!(index.total_files(), 5);
    assert_eq!(index.total_folders(), 1);
    let report = build_report(&index.files, &DuplicateOptions { min_bytes: 0, ..Default::default() }, "report".into(), 0, |_, _| {});
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].copies.len(), 2);
    assert_eq!(report.groups[0].reclaimable_bytes, 4);
    let payload = serde_json::to_value(snapshot("scan", &counters, 1, None)).unwrap();
    assert_eq!(payload["scanId"], "scan");
    assert_eq!(payload["filesScanned"], 5);
    assert_eq!(payload["bytesAnalyzed"], 15);
    assert!(payload.get("status").is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_roots_report_errors_instead_of_successful_empty_scans() {
    let root = std::env::temp_dir().join(format!("diskora-missing-{}", std::process::id()));
    let run = walk(&ScanOptions { root_path: root.to_string_lossy().into_owned(), ..Default::default() }, &Arc::new(ProgressCounters::new()));
    assert!(!run.errors.is_empty());
    assert!(run.files.is_empty());
}

#[test]
fn storage_breakdown_reports_structured_categories_and_safe_status() {
    let breakdown = diskwise_lib::platform::inspect_storage_breakdown("C:\\");
    assert!(!breakdown.items.is_empty());
    // Every item has valid status and explanation
    for item in &breakdown.items {
        assert!(!item.id.is_empty());
        assert!(!item.name.is_empty());
        assert!(!item.description.is_empty());
        assert!(!item.actionable_explanation.is_empty());
    }
    // Recycle bin is represented
    assert!(breakdown.items.iter().any(|i| i.id == "recycle-bin"));
    // System restore / shadow copies are represented with system protection
    assert!(breakdown.items.iter().any(|i| i.id == "system-restore-vss" && i.is_system_protected));
}

#[test]
fn links_and_junctions_are_not_recursively_followed_or_double_counted() {
    let root = std::env::temp_dir().join(format!("diskora-links-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("target_dir")).unwrap();
    fs::write(root.join("target_dir/file.bin"), b"12345678").unwrap();

    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(root.join("target_dir"), root.join("link_dir"));

    let options = ScanOptions {
        root_path: root.to_string_lossy().into_owned(),
        follow_links: false,
        ..Default::default()
    };
    let counters = Arc::new(ProgressCounters::new());
    let run = walk(&options, &counters);
    let index = ScanIndex::from_run(&run);

    // Target file is only counted once
    assert_eq!(index.total_bytes(), 8);
    assert_eq!(index.total_files(), 1);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn free_tier_identity_tokens_resolve_back_to_real_paths_without_leaking_them() {
    use diskwise_lib::disclosure::{disclose, path_ref, ref_id};
    let root = std::env::temp_dir().join(format!("diskora-tokens-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("Videos")).unwrap();
    fs::write(root.join("Videos").join("clip.mp4"), b"video").unwrap();
    let options = ScanOptions { root_path: root.to_string_lossy().into_owned(), ..Default::default() };
    let counters = Arc::new(ProgressCounters::new());
    let run = walk(&options, &counters);
    let index = ScanIndex::from_run(&run);

    // The token the webview receives for the clip resolves back to the real file inside the index.
    let clip = index.files.iter().find(|file| file.name == "clip.mp4").expect("clip.mp4 is indexed");
    let real = clip.path.to_string_lossy().into_owned();
    let token = path_ref(&real);
    let id = ref_id(&token).expect("well-formed token");
    let resolved = index.path_for_ref(id).expect("the token resolves inside the scan");
    assert_eq!(path_ref(&resolved.to_string_lossy()), token);
    assert!(index.path_for_ref("0000000000000000").is_none());

    // The free-tier payload carries the token and a masked display string, never the real path.
    let mut payload = serde_json::json!({ "path": real, "name": "clip.mp4" });
    disclose(&mut payload, false);
    let text = payload.to_string();
    assert!(!text.contains(&root.file_name().unwrap().to_string_lossy().to_string()), "{text}");
    let display = payload["displayPath"].as_str().expect("masked display string");
    assert!(display.ends_with("...\\Videos\\clip.mp4") || display.ends_with("\\Videos\\clip.mp4"), "{display}");
    assert!(!display.contains("diskora-tokens"), "{display}");
    fs::remove_dir_all(root).unwrap();
}
