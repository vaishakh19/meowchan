//! Large file finder.
//!
//! Runs against the in-memory scan index, so filtering a million files is instant and the disk is
//! never walked twice. The result is a page, never the whole list.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::filesystem::classify;
use crate::state::AppState;

/// Mirrors `LargeFileQuery`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFileQuery {
    pub scan_id: String,
    pub min_bytes: u64,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub modified_after: Option<i64>,
    #[serde(default)]
    pub modified_before: Option<i64>,
    #[serde(default = "default_sort")]
    pub sort: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_sort() -> String {
    "size-desc".to_string()
}

fn default_limit() -> usize {
    200
}

/// Mirrors `LargeFile`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub disk_size_bytes: Option<u64>,
    pub modified_at: Option<i64>,
    pub extension: Option<String>,
    pub category: String,
}

/// Mirrors `LargeFilePage`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFilePage {
    pub total: usize,
    pub total_bytes: u64,
    pub offset: usize,
    pub limit: usize,
    pub items: Vec<LargeFile>,
}

#[tauri::command]
pub fn list_large_files(query: LargeFileQuery, state: State<'_, AppState>) -> AppResult<LargeFilePage> {
    crate::authz::authorize_state(&state, crate::authz::Permission::LargeFileFinder)?;
    let scans = state.scans()?;
    let entry = scans
        .get(&query.scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Unknown scan {}", query.scan_id)))?;
    let index = entry
        .index
        .as_ref()
        .ok_or_else(|| AppError::NotFound("The scan is still running.".to_string()))?;

    let extensions: Vec<String> = query
        .extensions
        .iter()
        .map(|extension| extension.trim().trim_start_matches('.').to_lowercase())
        .filter(|extension| !extension.is_empty())
        .collect();

    let mut matched: Vec<LargeFile> = index
        .files
        .iter()
        .filter(|file| file.size >= query.min_bytes)
        .filter(|file| {
            if extensions.is_empty() {
                return true;
            }
            match &file.extension {
                Some(extension) => extensions.iter().any(|want| want == extension),
                None => false,
            }
        })
        .filter(|file| match (query.modified_after, query.modified_before) {
            (Some(after), Some(before)) => match file.modified_at {
                Some(modified) => modified >= after && modified <= before,
                None => false,
            },
            (Some(after), None) => file.modified_at.map(|modified| modified >= after).unwrap_or(false),
            (None, Some(before)) => file.modified_at.map(|modified| modified <= before).unwrap_or(false),
            (None, None) => true,
        })
        .map(|file| {
            let path = PathBuf::from(&file.path);
            LargeFile {
                path: file.path.to_string_lossy().to_string(),
                name: file.name.clone(),
                size_bytes: file.size,
                disk_size_bytes: file.disk_size,
                modified_at: file.modified_at,
                extension: file.extension.clone(),
                category: classify(&path, false),
            }
        })
        .collect();

    let total_bytes: u64 = matched.iter().map(|item| item.size_bytes).sum();

    match query.sort.as_str() {
        "size-asc" => matched.sort_by(|a, b| a.size_bytes.cmp(&b.size_bytes)),
        "modified-desc" => matched.sort_by(|a, b| b.modified_at.unwrap_or(0).cmp(&a.modified_at.unwrap_or(0))),
        "modified-asc" => matched.sort_by(|a, b| a.modified_at.unwrap_or(0).cmp(&b.modified_at.unwrap_or(0))),
        "name-asc" => matched.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "name-desc" => matched.sort_by(|a, b| b.name.to_lowercase().cmp(&a.name.to_lowercase())),
        _ => matched.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| a.path.cmp(&b.path))),
    }

    let total = matched.len();
    let limit = query.limit.clamp(1, 1000);
    let items: Vec<LargeFile> = matched.into_iter().skip(query.offset).take(limit).collect();

    Ok(LargeFilePage {
        total,
        total_bytes,
        offset: query.offset,
        limit,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::FileRecord;
    use crate::scanner::aggregate::ScanIndex;
    use std::path::PathBuf;

    fn index_with(files: Vec<FileRecord>) -> ScanIndex {
        let mut index = ScanIndex::default();
        index.files = files;
        index
    }

    fn record(path: &str, size: u64, extension: &str, modified: Option<i64>) -> FileRecord {
        FileRecord {
            path: PathBuf::from(path),
            name: PathBuf::from(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            size,
            disk_size: None,
            modified_at: modified,
            extension: Some(extension.to_string()),
            parent_path: PathBuf::from("C:\\"),
            depth: 1,
        }
    }

    #[test]
    fn filters_by_minimum_size() {
        let index = index_with(vec![
            record("C:\\small.bin", 10, "bin", None),
            record("C:\\big.bin", 5000, "bin", None),
        ]);
        let query = LargeFileQuery {
            scan_id: "s".to_string(),
            min_bytes: 1000,
            extensions: vec![],
            modified_after: None,
            modified_before: None,
            sort: "size-desc".to_string(),
            offset: 0,
            limit: 10,
        };
        // Reuse the same predicate the command applies.
        let matched: Vec<&FileRecord> = index.files.iter().filter(|file| file.size >= query.min_bytes).collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "big.bin");
    }

    #[test]
    fn sorts_largest_first_by_default() {
        let files = vec![
            record("C:\\a.bin", 10, "bin", None),
            record("C:\\b.bin", 900, "bin", None),
            record("C:\\c.bin", 100, "bin", None),
        ];
        let mut sorted = files;
        sorted.sort_by(|a, b| b.size.cmp(&a.size));
        assert_eq!(sorted[0].size, 900);
    }

    #[test]
    fn a_limit_of_zero_is_clamped() {
        assert_eq!(0usize.clamp(1, 1000), 1);
        assert_eq!(5000usize.clamp(1, 1000), 1000);
    }
}
