//! Cleanup assistant.
//!
//! Non-negotiables, enforced here rather than in the UI:
//!   - a finding is only ever *offered* for a folder the user explicitly configured,
//!   - every action lists the exact paths and byte totals before anything changes,
//!   - every action requires explicit confirmation (`confirmed: true` in the plan),
//!   - the Recycle Bin is the default; permanent deletion is a separate, clearly-labelled action,
//!   - every path is re-checked immediately before acting, and anything that changed is skipped,
//!   - failures and partial completion are reported, never swallowed,
//!   - nothing is ever deleted merely because it was selected or analysed.

pub mod execute;
pub mod rules;

pub use execute::{
    execute_plan, preview_plan, CleanupPathOutcome, CleanupPlan, CleanupPlanItem, CleanupPreview,
    CleanupPreviewItem, CleanupResult, CleanupResultItem,
};
pub use rules::{build_findings, CleanupFinding, CleanupRule, RecycleBinSummary};
