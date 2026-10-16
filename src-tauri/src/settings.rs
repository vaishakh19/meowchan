//! Application settings.
//!
//! Mirrors `AppSettings` in `src/types/contracts.ts`. Stored as JSON in SQLite; a missing row
//! produces these defaults.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: String,
    pub default_scan_root: Option<String>,
    pub max_children_per_node: u32,
    pub follow_links: bool,
    pub confirm_before_cleanup: bool,
    pub prefer_recycle_bin: bool,
    pub large_unused_days: u32,
    pub cache_paths: Vec<String>,
    pub temp_paths: Vec<String>,
    pub exclude_paths: Vec<String>,
    pub crash_reporting: bool,
    pub duplicate_group_limit: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            theme: "system".to_string(),
            default_scan_root: None,
            max_children_per_node: 100,
            follow_links: false,
            confirm_before_cleanup: true,
            prefer_recycle_bin: true,
            large_unused_days: 180,
            cache_paths: Vec::new(),
            temp_paths: Vec::new(),
            exclude_paths: Vec::new(),
            crash_reporting: false,
            duplicate_group_limit: 25,
        }
    }
}

impl AppSettings {
    /// Sanity-clamps user-supplied values so a bad settings file cannot break scanning.
    pub fn normalized(mut self) -> Self {
        if self.max_children_per_node < 2 {
            self.max_children_per_node = 2;
        }
        if self.max_children_per_node > 1000 {
            self.max_children_per_node = 1000;
        }
        if self.large_unused_days > 3650 {
            self.large_unused_days = 3650;
        }
        self.cache_paths.retain(|path| !path.trim().is_empty());
        self.temp_paths.retain(|path| !path.trim().is_empty());
        self.exclude_paths.retain(|path| !path.trim().is_empty());
        if self.duplicate_group_limit > 10_000 {
            self.duplicate_group_limit = 10_000;
        }
        match self.theme.as_str() {
            "light" | "dark" | "system" => {}
            _ => self.theme = "system".to_string(),
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative() {
        let settings = AppSettings::default();
        assert_eq!(settings.theme, "system");
        assert!(!settings.follow_links);
        assert!(settings.confirm_before_cleanup);
        assert!(settings.prefer_recycle_bin);
        assert!(!settings.crash_reporting);
        assert_eq!(settings.max_children_per_node, 100);
    }

    #[test]
    fn normalization_clamps_out_of_range_values() {
        let mut settings = AppSettings::default();
        settings.max_children_per_node = 0;
        settings.large_unused_days = 99_999;
        settings.theme = "neon".to_string();
        settings.cache_paths = vec!["C:\\cache".to_string(), "   ".to_string()];
        let normalized = settings.normalized();
        assert_eq!(normalized.max_children_per_node, 2);
        assert_eq!(normalized.large_unused_days, 3650);
        assert_eq!(normalized.theme, "system");
        assert_eq!(normalized.cache_paths.len(), 1);
    }

    #[test]
    fn serde_uses_camel_case() {
        let json = serde_json::to_string(&AppSettings::default()).unwrap();
        assert!(json.contains("\"followLinks\""));
        assert!(json.contains("\"maxChildrenPerNode\""));
        assert!(!json.contains("follow_links"));
    }

    #[test]
    fn round_trips_through_json() {
        let settings = AppSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.max_children_per_node, settings.max_children_per_node);
    }
}
