//! Scanner.
//!
//! Design goals, in priority order:
//!   1. never block the UI — the walker runs on worker threads and reports progress on a timer,
//!   2. never follow links by default and never loop — link entries are recorded, not traversed,
//!   3. honest partial results — an unreadable directory is reported, never silently skipped,
//!   4. bounded memory — one small record per entry, no retained metadata beyond what is used,
//!   5. cancellation that actually stops the walk.
//!
//! The walker is hand-written over `std::fs` (no `walkdir`) because we need a work-stealing queue
//! with a bounded worker pool, per-directory error capture and cancellation checks at a finer
//! granularity than an external crate offers.

pub mod aggregate;
pub mod progress;
pub mod walker;

pub use aggregate::{
    prune_tree, DirectoryAggregate, ScanIndex, TreeNode, PruneOptions, DEFAULT_MAX_CHILDREN,
    DEFAULT_MAX_NODES, OTHER_NODE_SUFFIX,
};
pub use progress::{ProgressCounters, ProgressSnapshot, ScanPhase, ScanStatus};
pub use walker::{ScanOptions, ScanRun, WalkError};
