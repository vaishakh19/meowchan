//! Storage category classification.
//!
//! The mapping is intentionally simple and explainable: it decides the colour of a segment in the
//! sunburst and the bucket shown in the breakdown. It never influences whether something is safe to
//! delete — that is a separate, explicit decision in `src/cleanup/`.

use std::path::Path;

pub const CATEGORY_SYSTEM: &str = "system";
pub const CATEGORY_APPLICATIONS: &str = "applications";
pub const CATEGORY_GAMES: &str = "games";
pub const CATEGORY_DOCUMENTS: &str = "documents";
pub const CATEGORY_MEDIA: &str = "media";
pub const CATEGORY_DEVELOPMENT: &str = "development";
pub const CATEGORY_DOWNLOADS: &str = "downloads";
pub const CATEGORY_CACHE: &str = "cache";
pub const CATEGORY_ARCHIVES: &str = "archives";
pub const CATEGORY_OTHER: &str = "other";
pub const CATEGORY_FREE: &str = "free";

const SYSTEM_PATHS: &[&str] = &[
    "\\windows\\",
    "\\winnt\\",
    "\\$windows.~bt\\",
    "\\system volume information\\",
    "\\$recycle.bin\\",
    "\\recovery\\",
    "\\perflogs\\",
    "\\programdata\\",
    "\\boot\\",
];

const CACHE_TOKENS: &[&str] = &[
    "cache",
    "caches",
    "\\temp\\",
    "\\tmp\\",
    "temporary",
    "\\crashdumps\\",
    "thumbnail",
    "\\webcache\\",
    "\\gpu cache\\",
    "\\shader cache\\",
    "\\code cache\\",
    "\\service worker\\",
    "\\incremental\\",
];

const DOWNLOAD_TOKENS: &[&str] = &["\\downloads\\", "\\download\\"];

const DEV_TOKENS: &[&str] = &[
    "\\node_modules\\",
    "\\.git\\",
    "\\.cargo\\",
    "\\.rustup\\",
    "\\target\\",
    "\\__pycache__\\",
    "\\.venv\\",
    "\\venv\\",
    "\\.gradle\\",
    "\\.m2\\",
    "\\.nuget\\",
    "\\packages\\",
    "\\vendor\\",
];

const APP_TOKENS: &[&str] = &[
    "\\program files",
    "\\program files (x86)",
    "\\programdata\\",
    "\\windowsapps\\",
    "\\appdata\\local\\programs",
];

const GAME_TOKENS: &[&str] = &[
    "\\steamapps\\",
    "\\steam library",
    "\\epic games\\",
    "\\xbox games\\",
    "\\games\\",
    "\\battle.net\\",
    "\\riot games\\",
    "\\ubisoft\\",
];

const MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "tiff", "heic", "raw", "svg", "mp4", "mkv", "avi",
    "mov", "wmv", "flv", "webm", "mp3", "flac", "wav", "aac", "ogg", "wma", "m4a",
];

const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso", "cab", "tgz", "zst",
];

const DOCUMENT_EXTENSIONS: &[&str] = &[
    "doc", "docx", "odt", "rtf", "txt", "md", "pdf", "xls", "xlsx", "ods", "csv", "ppt", "pptx",
    "odp", "epub", "pages", "numbers", "key",
];

const CODE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rb", "go", "java", "kt", "c", "h", "cpp",
    "hpp", "cs", "php", "swift", "m", "mm", "scala", "sh", "ps1", "bat", "sql", "html", "css",
    "scss", "sass", "less", "json", "yaml", "yml", "toml", "ini", "cfg", "lock",
];

const EXECUTABLE_EXTENSIONS: &[&str] = &["exe", "msi", "appx", "msix", "dll", "sys", "com"];

/// Classifies a path. `is_folder` distinguishes a directory from a file at the same path.
pub fn classify(path: &Path, is_folder: bool) -> String {
    let lowered = path.to_string_lossy().to_lowercase().replace('/', "\\");
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if contains_any(&lowered, SYSTEM_PATHS) {
        return CATEGORY_SYSTEM.to_string();
    }
    if contains_any(&lowered, GAME_TOKENS) {
        return CATEGORY_GAMES.to_string();
    }
    if contains_any(&lowered, CACHE_TOKENS) {
        return CATEGORY_CACHE.to_string();
    }
    if contains_any(&lowered, DOWNLOAD_TOKENS) {
        return CATEGORY_DOWNLOADS.to_string();
    }
    if contains_any(&lowered, DEV_TOKENS) {
        return CATEGORY_DEVELOPMENT.to_string();
    }
    if contains_any(&lowered, APP_TOKENS) {
        return CATEGORY_APPLICATIONS.to_string();
    }

    if is_folder {
        return CATEGORY_OTHER.to_string();
    }

    if MEDIA_EXTENSIONS.contains(&extension.as_str()) {
        return CATEGORY_MEDIA.to_string();
    }
    if ARCHIVE_EXTENSIONS.contains(&extension.as_str()) {
        return CATEGORY_ARCHIVES.to_string();
    }
    if CODE_EXTENSIONS.contains(&extension.as_str()) || EXECUTABLE_EXTENSIONS.contains(&extension.as_str()) {
        return CATEGORY_DEVELOPMENT.to_string();
    }
    if DOCUMENT_EXTENSIONS.contains(&extension.as_str()) {
        return CATEGORY_DOCUMENTS.to_string();
    }
    CATEGORY_OTHER.to_string()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classifies_windows_as_system() {
        let path = PathBuf::from("C:\\Windows\\System32\\kernel32.dll");
        assert_eq!(classify(&path, false), CATEGORY_SYSTEM);
    }

    #[test]
    fn classifies_downloads() {
        let path = PathBuf::from("C:\\Users\\me\\Downloads\\installer.exe");
        assert_eq!(classify(&path, false), CATEGORY_DOWNLOADS);
    }

    #[test]
    fn classifies_node_modules_as_development() {
        let path = PathBuf::from("D:\\app\\node_modules\\react\\index.js");
        assert_eq!(classify(&path, false), CATEGORY_DEVELOPMENT);
    }

    #[test]
    fn classifies_media_by_extension() {
        let path = PathBuf::from("D:\\media\\holiday.mp4");
        assert_eq!(classify(&path, false), CATEGORY_MEDIA);
    }

    #[test]
    fn classifies_documents_by_extension() {
        let path = PathBuf::from("D:\\docs\\report.pdf");
        assert_eq!(classify(&path, false), CATEGORY_DOCUMENTS);
    }

    #[test]
    fn classifies_archives_by_extension() {
        let path = PathBuf::from("D:\\backup\\2024.zip");
        assert_eq!(classify(&path, false), CATEGORY_ARCHIVES);
    }

    #[test]
    fn caches_are_caches_even_in_program_files() {
        // Program Files wins for executables, but a cache folder inside it is still a cache.
        let path = PathBuf::from("C:\\Program Files\\App\\Cache\\blob.dat");
        assert_eq!(classify(&path, false), CATEGORY_CACHE);
    }

    #[test]
    fn unknown_extension_falls_back_to_other() {
        let path = PathBuf::from("D:\\stuff\\thing.qqq");
        assert_eq!(classify(&path, false), CATEGORY_OTHER);
    }

    #[test]
    fn free_space_is_never_a_classified_category() {
        // `free` describes unallocated space, which the scanner never sees as an entry.
        assert_eq!(CATEGORY_FREE, "free");
        let path = PathBuf::from("D:\\anything.bin");
        assert_ne!(classify(&path, false), CATEGORY_FREE);
    }

    #[test]
    fn plain_folders_are_other() {
        let path = PathBuf::from("D:\\projects");
        assert_eq!(classify(&path, true), CATEGORY_OTHER);
    }
}
