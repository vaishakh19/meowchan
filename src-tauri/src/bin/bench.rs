//! Benchmark harness for the scanner's aggregation path.
//!
//! Run with `cargo run --release --bin bench -- [scenario]`. Prints measurements to stdout and
//! writes the same numbers to `target/bench-results.json` so a CI run can publish them.
//!
//! ## What this measures, and what it does not
//!
//! It measures `ScanIndex::from_run` + `prune_tree` over a **generated** walk result — the
//! aggregation half of the contract: turning a walk's flat tables into the bounded tree the UI
//! receives, with byte totals that stay exact.
//!
//! It does **not** measure the walker against a real NTFS volume. That is a different measurement
//! with different bottlenecks, and the `smoke-windows` CI job is what exercises the real walker.
//! Nothing in this file's output is a claim about real-volume throughput.
//!
//! Every number printed came from a run of this binary. There are no targets here and no estimates.

use std::path::PathBuf;
use std::time::Instant;

use diskwise_lib::filesystem::FileRecord;
use diskwise_lib::scanner::aggregate::{prune_tree, PruneOptions, ScanIndex, OTHER_NODE_SUFFIX};
use diskwise_lib::scanner::walker::{DirectoryRecord, ScanRun, WalkError};

/// xorshift32, so every run generates the identical tree.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() as usize) % bound
    }
}

/// One generated scenario.
struct Scenario {
    name: &'static str,
    files: usize,
    dirs: usize,
}

const SCENARIOS: &[Scenario] = &[
    Scenario { name: "small", files: 2_000, dirs: 200 },
    Scenario { name: "medium", files: 25_000, dirs: 2_000 },
    Scenario { name: "large", files: 100_000, dirs: 8_000 },
    // A realistic Windows volume: many files per folder, which is what trips the child cut and
    // produces the synthetic "Other" node.
    Scenario { name: "wide", files: 120_000, dirs: 400 },
    Scenario { name: "huge", files: 250_000, dirs: 20_000 },
];

/// Builds a flat walk result shaped exactly like a real one.
///
/// `ScanIndex::from_run` resolves directory parents by path and rolls files into their parent
/// directory by `parent_path`, so the generator has to produce consistent paths — which is the only
/// thing that makes this a fair measurement of the real aggregation.
fn generate(scenario: &Scenario, seed: u32) -> ScanRun {
    let mut rng = Rng(seed);
    let extensions = ["exe", "dll", "log", "tmp", "png", "mp4", "zip", "txt"];

    let mut directories: Vec<DirectoryRecord> = Vec::with_capacity(scenario.dirs);
    directories.push(DirectoryRecord {
        path: PathBuf::from("C:\\"),
        name: "C:".to_string(),
        parent: None,
        depth: 0,
        modified_at: Some(1_700_000_000_000),
        created_at: Some(1_600_000_000_000),
        access_denied: false,
    });

    // A chain of directories, each parented to a randomly chosen earlier one. This produces the same
    // depth distribution a real volume has, including the long tail that used to make the pruner's
    // depth search expensive.
    let mut paths: Vec<PathBuf> = Vec::with_capacity(scenario.dirs);
    paths.push(PathBuf::from("C:\\"));
    for index in 1..scenario.dirs {
        let parent = rng.below(paths.len());
        let path = paths[parent].join(format!("d{index}"));
        directories.push(DirectoryRecord {
            name: format!("d{index}"),
            path: path.clone(),
            parent: None,
            depth: directories[parent].depth + 1,
            modified_at: Some(1_700_000_000_000),
            created_at: Some(1_600_000_000_000),
            access_denied: false,
        });
        paths.push(path);
    }

    let mut files: Vec<FileRecord> = Vec::with_capacity(scenario.files);
    for index in 0..scenario.files {
        let parent = rng.below(paths.len());
        let extension = extensions[rng.below(extensions.len())];
        let size = 1024 + (rng.next() as u64 % (64 * 1024 * 1024));
        let parent_path = paths[parent].clone();
        files.push(FileRecord {
            path: parent_path.join(format!("f{index}.{extension}")),
            name: format!("f{index}.{extension}"),
            size,
            disk_size: Some((size + 4095) / 4096 * 4096),
            modified_at: Some(1_700_000_000_000),
            extension: Some(extension.to_string()),
            parent_path,
            depth: directories[parent].depth + 1,
        });
    }

    ScanRun {
        files,
        directories,
        errors: Vec::<WalkError>::new(),
        hit_limit: false,
        cancelled: false,
        elapsed_ms: 0,
    }
}

/// The exact byte total of a run: the sum of every file's size.
fn exact_bytes(run: &ScanRun) -> u64 {
    run.files.iter().map(|file| file.size).sum()
}

/// Walks a pruned tree and checks that no folder reports more bytes than its children sum to, and
/// that the root still reports exactly what the source did.
///
/// Returns `(nodes, over_reporting, synthetic_nodes)`.
fn audit(
    node: &diskwise_lib::scanner::aggregate::TreeNode,
    expected_root_bytes: u64,
) -> (usize, usize, usize) {
    fn walk(
        node: &diskwise_lib::scanner::aggregate::TreeNode,
        over_reporting: &mut usize,
        synthetic: &mut usize,
    ) -> usize {
        let mut nodes = 1;
        if node.children.is_empty() {
            return nodes;
        }
        let child_sum: u64 = node.children.iter().map(|child| child.size_bytes).sum();
        if child_sum > node.size_bytes {
            *over_reporting += 1;
        }
        for child in &node.children {
            if child.name.ends_with(OTHER_NODE_SUFFIX) {
                *synthetic += 1;
            }
            nodes += walk(child, over_reporting, synthetic);
        }
        nodes
    }

    let mut over_reporting = 0;
    let mut synthetic = 0;
    let nodes = walk(node, &mut over_reporting, &mut synthetic);
    assert_eq!(
        node.size_bytes, expected_root_bytes,
        "the pruned root must report exactly the source byte total"
    );
    (nodes, over_reporting, synthetic)
}

/// Median of `runs` timings, in milliseconds.
fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    samples[samples.len() / 2]
}

fn main() {
    let filter = std::env::args().nth(1);
    let prune = PruneOptions::default();
    let runs = 5usize;

    println!("Diskwise aggregation benchmark");
    println!("==============================");
    println!(
        "prune options: maxChildren={} maxNodes={}",
        prune.max_children, prune.max_nodes
    );
    println!();
    println!("scenario    files     dirs   input files    index ms    prune ms   pruned   over   synth");
    println!("---------  -------  -------  ------------  ----------  ----------  -------  -----  -----");

    let mut report = String::from("{\n  \"node\": \"rust\",\n  \"results\": [\n");
    let mut first = true;

    for scenario in SCENARIOS {
        if let Some(filter) = &filter {
            if filter != scenario.name {
                continue;
            }
        }

        let run = generate(scenario, 0x2f6e2b1);
        let expected = exact_bytes(&run);

        // Index construction.
        let mut index_samples = Vec::with_capacity(runs);
        let mut index = None;
        for _ in 0..runs {
            let started = Instant::now();
            let built = ScanIndex::from_run(&run);
            index_samples.push(started.elapsed().as_secs_f64() * 1000.0);
            index = Some(built);
        }
        let index = index.expect("built");
        let index_ms = median(index_samples);

        // Pruning, from the index's root (position 0 after the path sort puts "C:\" first, so the
        // root is located by path rather than assumed).
        let root = index
            .position_of(std::path::Path::new("C:\\"))
            .expect("the generated tree has a root");

        let mut prune_samples = Vec::with_capacity(runs);
        let mut pruned = None;
        for _ in 0..runs {
            let started = Instant::now();
            let tree = prune_tree(&index, root, prune.clone());
            prune_samples.push(started.elapsed().as_secs_f64() * 1000.0);
            pruned = Some(tree);
        }
        let pruned = pruned.expect("pruned");
        let prune_ms = median(prune_samples);

        let (nodes, over_reporting, synthetic) = audit(&pruned.tree, expected);

        println!(
            "{:<9}  {:>7}  {:>7}  {:>12}  {:>10.1}  {:>10.1}  {:>7}  {:>5}  {:>5}",
            scenario.name,
            scenario.files,
            scenario.dirs,
            run.files.len(),
            index_ms,
            prune_ms,
            nodes,
            over_reporting,
            synthetic,
        );

        if over_reporting > 0 {
            eprintln!(
                "FAIL {}: {} folders report more bytes than their children sum to",
                scenario.name, over_reporting
            );
            std::process::exit(1);
        }

        if !first {
            report.push_str(",\n");
        }
        first = false;
        report.push_str(&format!(
            "    {{\"scenario\": \"{}\", \"files\": {}, \"dirs\": {}, \"indexMs\": {:.2}, \"pruneMs\": {:.2}, \"prunedNodes\": {}, \"overReporting\": {}, \"syntheticNodes\": {}, \"exactBytes\": {}}}",
            scenario.name,
            scenario.files,
            scenario.dirs,
            index_ms,
            prune_ms,
            nodes,
            over_reporting,
            synthetic,
            expected
        ));
    }

    report.push_str("\n  ]\n}\n");
    println!();
    println!("All invariants held: the root byte total is exact and no folder over-reports.");
    println!("Every number above came from a run of this binary. Nothing here is a target.");

    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let path = PathBuf::from(target).join("bench-results.json");
    if std::fs::write(&path, report).is_ok() {
        println!("Wrote {}", path.display());
    }
}
