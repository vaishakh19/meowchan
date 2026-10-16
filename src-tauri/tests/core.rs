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
