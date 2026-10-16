//! Filesystem primitives.
//!
//! The scanner needs three things that `std::fs` does not give directly: long-path (`\\?\`)
//! normalisation, a metadata snapshot that never follows links by default, and the storage
//! category classification used for the colour mapping in the UI. Keeping them here means the
//! walker stays small and the classification can be unit-tested without touching a disk.

pub mod classify;
pub mod entries;
pub mod long_path;

pub use classify::classify;
pub use entries::{read_metadata, EntryMeta, FileRecord, Kind};
pub use long_path::{is_long_path, normalize_long_path, to_unc_long_path};
