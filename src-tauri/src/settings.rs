//! Stored preferences. Unknown legacy fields are ignored; old databases are retained.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub theme: String,
    pub max_children_per_node: u32,
    pub follow_links: bool,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self { theme: "system".into(), max_children_per_node: 100, follow_links: false }
    }
}
impl AppSettings {
    pub fn normalized(mut self) -> Self {
        self.max_children_per_node = self.max_children_per_node.clamp(2, 1000);
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") { self.theme = "system".into(); }
        self
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_preferences_are_read_without_resetting_the_theme() {
        let settings: AppSettings = serde_json::from_str(r#"{"theme":"dark","crashReporting":true}"#).unwrap();
        assert_eq!(settings.theme, "dark");
        assert!(!settings.follow_links);
    }
    #[test]
    fn invalid_values_are_normalized() {
        let settings = AppSettings { theme: "invalid".into(), max_children_per_node: 0, follow_links: false }.normalized();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.max_children_per_node, 2);
    }
}
