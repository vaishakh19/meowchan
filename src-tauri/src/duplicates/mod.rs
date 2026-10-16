//! Duplicate detection.
//!
//! Staged, and never based on filenames:
//!   1. group candidate files by exact size — no I/O, removes the vast majority immediately,
//!   2. hash the survivors (SHA-256 by default; BLAKE3 is faster when the user opts in),
//!   3. optionally verify byte-for-byte before anything is offered for removal.
//!
//! Hashing runs on the shared rayon pool with bounded memory (a 1 MiB buffer reused per file), so a
//! multi-gigabyte file never lands in memory at once. Groups are ordered by reclaimable space, and
//! the app never preselects a copy: at least one copy of every group is always kept.

pub mod grouping;
pub mod hashing;
pub mod verify;

pub use grouping::{group_by_size, DuplicateGroup, DuplicateOptions, DuplicateReport, DuplicateFile};
pub use hashing::{hash_file, HashAlgorithm};
pub use verify::{verify_identical, VerifyResult};
