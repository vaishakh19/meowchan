//! Platform layer.
//!
//! Everything genuinely Windows-specific lives behind these small functions so the rest of the
//! backend stays portable and unit-testable. On non-Windows targets a portable fallback provides
//! the same functions with honest "not available" answers, which is what lets the scanner, tree
//! and duplicate tests run on any machine (including the Linux CI runners).

#[cfg(windows)]
mod win32;

#[cfg(not(windows))]
mod portable;

#[cfg(windows)]
pub use win32::*;

#[cfg(not(windows))]
pub use portable::*;
