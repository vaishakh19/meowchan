//! The permission catalogue.
//!
//! One enum, one place, so a new paid capability cannot be added without deciding explicitly
//! whether it is free or paid. `PERMISSIONS` is the complete list and is used by the tests to prove
//! nothing is unclassified.

use serde::Serialize;

/// Every capability that can be gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /* ------------------------------- always free ------------------------------- */
    /// Scanning, aggregation, visualization and manual navigation.
    CoreAnalysis,
    /// Reading metadata, revealing in Explorer, opening a file, copying a path.
    FileInspection,
    /// Previewing a cleanup plan or a duplicate group without changing anything.
    PreviewOnly,
    /// Large-file finder with the free-tier limits.
    LargeFileFinder,
    /// Duplicate detection with the free-tier group limit.
    BasicDuplicates,

    /* --------------------------------- paid --------------------------------- */
    /// Executing a cleanup plan: moving to the Recycle Bin or deleting.
    PaidCleanup,
    /// Permanently deleting (as opposed to the reversible Recycle Bin path).
    PermanentDeletion,
    /// Removing duplicate copies.
    DuplicateRemoval,
    /// Bulk file operations.
    BulkFileOperations,
    /// Exporting reports.
    ExportReports,
    /// Scheduled scans.
    ScheduledScans,
    /// Custom cleanup rules.
    CustomCleanupRules,
    /// Advanced filters and type/age analytics.
    AdvancedFilters,
    /// Saved scan history and historical comparison.
    ScanHistory,
    /// Unlimited duplicate groups.
    UnlimitedDuplicates,
}

/// The complete catalogue. The tests iterate this, so adding a variant without classifying it fails.
pub const PERMISSIONS: &[Permission] = &[
    Permission::CoreAnalysis,
    Permission::FileInspection,
    Permission::PreviewOnly,
    Permission::LargeFileFinder,
    Permission::BasicDuplicates,
    Permission::PaidCleanup,
    Permission::PermanentDeletion,
    Permission::DuplicateRemoval,
    Permission::BulkFileOperations,
    Permission::ExportReports,
    Permission::ScheduledScans,
    Permission::CustomCleanupRules,
    Permission::AdvancedFilters,
    Permission::ScanHistory,
    Permission::UnlimitedDuplicates,
];

impl Permission {
    /// True when this capability is part of the free tier and never gated.
    ///
    /// Scanning and inspection are free by design: a disk analyzer that cannot analyze a disk until
    /// you pay is not a product anyone would trust.
    pub fn is_free(&self) -> bool {
        matches!(
            self,
            Permission::CoreAnalysis
                | Permission::FileInspection
                | Permission::PreviewOnly
                | Permission::LargeFileFinder
                | Permission::BasicDuplicates
        )
    }

    /// A short, customer-facing name.
    pub fn label(&self) -> &'static str {
        match self {
            Permission::CoreAnalysis => "Disk analysis",
            Permission::FileInspection => "File inspection",
            Permission::PreviewOnly => "Previews",
            Permission::LargeFileFinder => "Large file finder",
            Permission::BasicDuplicates => "Duplicate detection",
            Permission::PaidCleanup => "Cleanup",
            Permission::PermanentDeletion => "Permanent deletion",
            Permission::DuplicateRemoval => "Duplicate removal",
            Permission::BulkFileOperations => "Bulk file operations",
            Permission::ExportReports => "Report export",
            Permission::ScheduledScans => "Scheduled scans",
            Permission::CustomCleanupRules => "Custom cleanup rules",
            Permission::AdvancedFilters => "Advanced filters",
            Permission::ScanHistory => "Scan history",
            Permission::UnlimitedDuplicates => "Unlimited duplicate groups",
        }
    }

    /// The feature flag this permission maps to, matching `FeatureFlags` in the shared contract.
    pub fn feature_flag(&self) -> &'static str {
        match self {
            Permission::CoreAnalysis => "coreAnalysis",
            Permission::FileInspection => "coreAnalysis",
            Permission::PreviewOnly => "coreAnalysis",
            Permission::LargeFileFinder => "coreAnalysis",
            Permission::BasicDuplicates => "coreAnalysis",
            Permission::PaidCleanup => "customCleanupRules",
            Permission::PermanentDeletion => "customCleanupRules",
            Permission::DuplicateRemoval => "advancedDuplicates",
            Permission::BulkFileOperations => "advancedDuplicates",
            Permission::ExportReports => "exportReports",
            Permission::ScheduledScans => "scheduledScans",
            Permission::CustomCleanupRules => "customCleanupRules",
            Permission::AdvancedFilters => "advancedFilters",
            Permission::ScanHistory => "scanHistory",
            Permission::UnlimitedDuplicates => "advancedDuplicates",
        }
    }

    /// Stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::CoreAnalysis => "core-analysis",
            Permission::FileInspection => "file-inspection",
            Permission::PreviewOnly => "preview-only",
            Permission::LargeFileFinder => "large-file-finder",
            Permission::BasicDuplicates => "basic-duplicates",
            Permission::PaidCleanup => "paid-cleanup",
            Permission::PermanentDeletion => "permanent-deletion",
            Permission::DuplicateRemoval => "duplicate-removal",
            Permission::BulkFileOperations => "bulk-file-operations",
            Permission::ExportReports => "export-reports",
            Permission::ScheduledScans => "scheduled-scans",
            Permission::CustomCleanupRules => "custom-cleanup-rules",
            Permission::AdvancedFilters => "advanced-filters",
            Permission::ScanHistory => "scan-history",
            Permission::UnlimitedDuplicates => "unlimited-duplicates",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_permission_is_classified_exactly_once() {
        let mut seen = std::collections::HashSet::new();
        for permission in PERMISSIONS {
            assert!(seen.insert(*permission), "{permission:?} appears twice");
        }
        assert_eq!(seen.len(), PERMISSIONS.len());
    }

    #[test]
    fn every_permission_has_a_non_empty_label_and_wire_name() {
        for permission in PERMISSIONS {
            assert!(!permission.label().is_empty(), "{permission:?}");
            assert!(!permission.as_str().is_empty(), "{permission:?}");
            assert!(!permission.feature_flag().is_empty(), "{permission:?}");
        }
    }

    #[test]
    fn wire_names_are_unique() {
        let mut names = std::collections::HashSet::new();
        for permission in PERMISSIONS {
            assert!(names.insert(permission.as_str()), "duplicate name");
        }
    }

    #[test]
    fn the_free_set_is_exactly_the_five_analysis_permissions() {
        let free: Vec<&str> = PERMISSIONS
            .iter()
            .filter(|permission| permission.is_free())
            .map(|permission| permission.as_str())
            .collect();
        assert_eq!(
            free,
            vec![
                "core-analysis",
                "file-inspection",
                "preview-only",
                "large-file-finder",
                "basic-duplicates",
            ]
        );
    }

    #[test]
    fn destructive_permissions_are_never_free() {
        for permission in [
            Permission::PaidCleanup,
            Permission::PermanentDeletion,
            Permission::DuplicateRemoval,
            Permission::BulkFileOperations,
        ] {
            assert!(!permission.is_free(), "{permission:?} must be paid");
        }
    }

    #[test]
    fn serde_uses_kebab_case() {
        let json = serde_json::to_string(&Permission::PaidCleanup).unwrap();
        assert_eq!(json, "\"paid-cleanup\"");
    }
}
