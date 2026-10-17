//! Path disclosure: the single chokepoint where filesystem paths leave the native backend.
//!
//! Free (no valid licence) users may scan and view results, but they must never receive a
//! real absolute path. Every payload that crosses the IPC boundary (command results and
//! progress events) and every error message passes through this module:
//!
//! * `path`, `rootPath`, `userPath`, `currentPath`, `parentPath`, `baselineRoot` and
//!   `currentRoot` are replaced by an opaque identity token (`ref:<id>`). The token is the
//!   same FNV id the tree already exposes, and the backend resolves it back to the real path
//!   for navigation. The token itself reveals nothing about the directory layout.
//! * A masked display string (`C:\...\Videos\clip.mp4`) is added next to each of them as
//!   `displayPath` (for `path`) or `<key>Display` (for the others).
//! * `dataDir` and `databasePath` are masked in place.
//! * Any other string that contains an absolute path (for example an error message) is redacted.
//!
//! Pro users receive the unchanged payload, so the frontend can always read `displayPath ?? path`.
//!
//! The decision is made from one process-wide flag. It is set only by the licensing module,
//! which derives it from Dodo Payments verification (or the documented offline grace window).
//! Every command and event calls the same functions, so no route can bypass the check.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::scanner::aggregate::stable_path_id;
use crate::state::{AppState, ScanEntry};

/// Prefix of every identity token handed to the webview in place of a real path.
pub const REF_PREFIX: &str = "ref:";

/// Keys whose string value is an absolute filesystem path.
const PATH_KEYS: [&str; 7] = [
    "path",
    "rootPath",
    "userPath",
    "currentPath",
    "parentPath",
    "baselineRoot",
    "currentRoot",
];

/// Keys that are masked in place with no identity token (application data locations).
const MASK_ONLY_KEYS: [&str; 2] = ["dataDir", "databasePath"];

static FULL_PATHS: AtomicBool = AtomicBool::new(false);

/// Set by the licensing module after every verification. `true` only for a valid Pro licence.
pub fn set_full_paths(enabled: bool) {
    FULL_PATHS.store(enabled, Ordering::SeqCst);
}

/// True when the current licence allows real paths to be shown, copied or revealed.
pub fn full_paths_allowed() -> bool {
    FULL_PATHS.load(Ordering::SeqCst)
}

/// The identity token for a real path. Stable for the same path, opaque to the webview.
pub fn path_ref(path: &str) -> String {
    format!("{REF_PREFIX}{}", stable_path_id(Path::new(path)))
}

/// The hex id inside a token, or `None` when the input is a real path or a malformed token.
pub fn ref_id(token: &str) -> Option<&str> {
    let id = token.strip_prefix(REF_PREFIX)?;
    let well_formed = !id.is_empty() && id.len() <= 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit());
    if well_formed {
        Some(id)
    } else {
        None
    }
}

/// Masks a Windows path for display, keeping only the drive and the last one or two segments.
///
/// `C:\Users\bob\Videos\clip.mp4` becomes `C:\...\Videos\clip.mp4`. Shallow paths keep their
/// final segment, so `C:\Users\bob\clip.mp4` becomes `C:\...\clip.mp4`. UNC paths hide the
/// server and share names entirely.
pub fn mask_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let bytes = trimmed.as_bytes();
    let unc = trimmed.starts_with("\\\\") || trimmed.starts_with("//");
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let parts: Vec<&str> = trimmed
        .split(['\\', '/'])
        .filter(|part| !part.is_empty())
        .collect();

    if unc {
        return match parts.last() {
            Some(last) if parts.len() > 2 => format!("\\\\...\\{last}"),
            _ => "\\\\...".to_string(),
        };
    }

    let (head, tail): (String, &[&str]) = if drive {
        (
            format!("{}:\\", char::from(bytes[0]).to_ascii_uppercase()),
            parts.get(1..).unwrap_or_default(),
        )
    } else {
        (String::new(), &parts[..])
    };

    if tail.is_empty() {
        return if head.is_empty() { "...".to_string() } else { head };
    }
    let keep = if tail.len() > 3 { 2 } else { 1 };
    if tail.len() <= keep {
        return format!("{head}{}", tail.join("\\"));
    }
    let kept = tail[tail.len() - keep..].join("\\");
    format!("{head}...\\{kept}")
}

/// True when an absolute Windows path (drive or UNC) starts at `index`.
fn absolute_path_starts_at(chars: &[char], index: usize) -> bool {
    if index > 0 {
        let previous = chars[index - 1];
        if previous.is_alphanumeric() || previous == '\\' || previous == ':' {
            return false;
        }
    }
    if index + 2 >= chars.len() {
        return false;
    }
    let drive = chars[index].is_ascii_alphabetic() && chars[index + 1] == ':' && chars[index + 2] == '\\';
    let unc = chars[index] == '\\' && chars[index + 1] == '\\' && chars[index + 2].is_alphanumeric();
    drive || unc
}

/// Masks every absolute path embedded in free text such as an error message.
///
/// A path runs until a quote, angle bracket, pipe or line break. The rest of the sentence is
/// kept, so the message stays readable and only the directory names are removed.
pub fn redact_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if absolute_path_starts_at(&chars, index) {
            let mut end = index;
            while end < chars.len() && !matches!(chars[end], '"' | '\'' | '<' | '>' | '|' | '\r' | '\n') {
                end += 1;
            }
            let candidate: String = chars[index..end].iter().collect();
            output.push_str(&mask_path(candidate.trim_end()));
            index = end;
        } else {
            output.push(chars[index]);
            index += 1;
        }
    }
    output
}

fn display_key(key: &str) -> String {
    if key == "path" {
        "displayPath".to_string()
    } else {
        format!("{key}Display")
    }
}

/// Rewrites a JSON payload in place. `full = true` leaves it untouched (Pro).
pub fn disclose(value: &mut Value, full: bool) {
    if full {
        return;
    }
    disclose_in_place(value);
}

fn disclose_in_place(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let mut displays: Vec<(String, Value)> = Vec::new();
            for (key, child) in map.iter_mut() {
                let key_name: &str = key.as_str();
                let text = child.as_str().map(str::to_owned);
                match text {
                    Some(raw) if PATH_KEYS.contains(&key_name) && !raw.is_empty() => {
                        displays.push((display_key(key_name), Value::String(mask_path(&raw))));
                        *child = Value::String(path_ref(&raw));
                    }
                    Some(raw) if MASK_ONLY_KEYS.contains(&key_name) => {
                        *child = Value::String(mask_path(&raw));
                    }
                    Some(raw) => {
                        *child = Value::String(redact_text(&raw));
                    }
                    None => disclose_in_place(child),
                }
            }
            for (key, display) in displays {
                map.insert(key, display);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                disclose_in_place(item);
            }
        }
        _ => {}
    }
}

/// Serializes a result for the webview, applying the current licence's disclosure rule.
pub fn disclosed<T: Serialize>(value: &T) -> AppResult<Value> {
    let mut json = serde_json::to_value(value)?;
    disclose(&mut json, full_paths_allowed());
    Ok(json)
}

/// Turns an identity token from the webview back into a real path.
///
/// Real paths pass through unchanged, so existing callers keep working. Tokens are looked up
/// in `scan_hint` first, then in every scan this session knows about, then in persisted scan
/// history (so a root from an earlier session can still be rescanned).
pub fn resolve_path(state: &AppState, scan_hint: Option<&str>, input: &str) -> AppResult<String> {
    let Some(id) = ref_id(input) else {
        return Ok(input.to_string());
    };
    let scans = state.scans()?;
    let hinted = scan_hint.and_then(|hint| scans.get(hint));
    let others = scans
        .iter()
        .filter(|(key, _)| Some(key.as_str()) != scan_hint)
        .map(|(_, entry)| entry);
    for entry in hinted.into_iter().chain(others) {
        if let Some(path) = resolve_in_entry(entry, input, id)? {
            return Ok(path);
        }
    }
    drop(scans);
    if let Some(path) = resolve_persisted_root(state, input) {
        return Ok(path);
    }
    Err(AppError::NotFound(
        "That item is no longer in the scan. Rescan the drive to refresh the results.".to_string(),
    ))
}

fn resolve_in_entry(entry: &Arc<ScanEntry>, token: &str, id: &str) -> AppResult<Option<String>> {
    if path_ref(&entry.root_path) == token {
        return Ok(Some(entry.root_path.clone()));
    }
    let Some(index) = entry.index.as_ref() else {
        return Ok(None);
    };
    let guard = index
        .read()
        .map_err(|_| AppError::Internal("The scan index lock was poisoned.".to_string()))?;
    Ok(guard
        .path_for_ref(id)
        .map(|path| path.to_string_lossy().to_string()))
}

/// Roots of persisted scan history. Only a root can be recovered this way; contents are not.
fn resolve_persisted_root(state: &AppState, token: &str) -> Option<String> {
    let guard = state.database().ok()?;
    let database = guard.as_ref()?;
    let rows = database.list_scan_history(500).ok()?;
    rows.into_iter().map(|row| row.root_path).find(|root| path_ref(root) == token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_deep_paths_to_drive_parent_and_name() {
        assert_eq!(mask_path("C:\\Users\\bob\\Videos\\clip.mp4"), "C:\\...\\Videos\\clip.mp4");
        assert_eq!(
            mask_path("C:\\Users\\bob\\Documents\\Work\\report.docx"),
            "C:\\...\\Work\\report.docx"
        );
    }

    #[test]
    fn shallow_paths_never_show_the_profile_name() {
        assert_eq!(mask_path("C:\\Users\\bob\\clip.mp4"), "C:\\...\\clip.mp4");
        assert_eq!(mask_path("C:\\Users\\bob"), "C:\\...\\bob");
        assert_eq!(mask_path("C:\\clip.mp4"), "C:\\clip.mp4");
        assert_eq!(mask_path("D:\\"), "D:\\");
        assert!(!mask_path("C:\\Users\\bob\\Videos\\clip.mp4").contains("bob"));
    }

    #[test]
    fn unc_paths_hide_server_and_share() {
        assert_eq!(mask_path("\\\\nas01\\family\\photos\\a.jpg"), "\\\\...\\a.jpg");
        assert_eq!(mask_path("\\\\nas01\\family"), "\\\\...");
        assert!(!mask_path("\\\\nas01\\family\\photos\\a.jpg").contains("nas01"));
    }

    #[test]
    fn redacts_absolute_paths_inside_messages() {
        let message = "Access denied: C:\\Users\\bob\\Secret\\notes.txt (os error 5)";
        let redacted = redact_text(message);
        assert!(!redacted.contains("Users"), "{redacted}");
        assert!(!redacted.contains("bob"), "{redacted}");
        assert!(redacted.starts_with("Access denied: "));
        assert!(redacted.ends_with("(os error 5)"), "{redacted}");
        assert_eq!(redact_text("Scan finished with 2 errors."), "Scan finished with 2 errors.");
    }

    #[test]
    fn identity_tokens_round_trip_and_reject_malformed_input() {
        let token = path_ref("C:\\Users\\bob\\clip.mp4");
        assert!(token.starts_with(REF_PREFIX));
        assert_eq!(ref_id(&token), Some(token.trim_start_matches(REF_PREFIX)));
        assert_eq!(ref_id("C:\\Users\\bob"), None);
        assert_eq!(ref_id("ref:"), None);
        assert_eq!(ref_id("ref:../../etc"), None);
    }

    #[test]
    fn free_payloads_contain_no_real_path_anywhere() {
        let mut payload = serde_json::json!({
            "scanId": "s1",
            "rootPath": "C:\\Users\\bob",
            "tree": {
                "id": "0123456789abcdef",
                "path": "C:\\Users\\bob\\Videos\\clip.mp4",
                "name": "clip.mp4",
                "kind": "file",
                "children": [
                    { "path": "C:\\Users\\bob\\Videos\\other.mp4", "name": "other.mp4", "children": [] }
                ]
            },
            "errors": [{ "path": "C:\\Users\\bob\\Secret", "message": "Access denied: C:\\Users\\bob\\Secret" }],
            "currentPath": "C:\\Users\\bob\\Music",
            "dataDir": "C:\\Users\\bob\\AppData\\Roaming\\diskwise",
        });
        disclose(&mut payload, false);
        let text = payload.to_string();
        assert!(!text.contains("Users"), "{text}");
        assert!(!text.contains("AppData"), "{text}");
        assert!(!text.contains("Roaming"), "{text}");
        assert_eq!(payload["tree"]["name"], "clip.mp4");
        assert!(payload["tree"]["path"].as_str().unwrap().starts_with(REF_PREFIX));
        assert_eq!(payload["tree"]["displayPath"], "C:\\...\\Videos\\clip.mp4");
        assert_eq!(payload["rootPathDisplay"], "C:\\...\\bob");
        assert_eq!(payload["currentPathDisplay"], "C:\\...\\Music");
        assert_eq!(payload["tree"]["children"][0]["displayPath"], "C:\\...\\Videos\\other.mp4");
    }

    #[test]
    fn pro_payloads_are_left_exactly_as_they_are() {
        let original = serde_json::json!({ "path": "C:\\Users\\bob\\clip.mp4", "name": "clip.mp4" });
        let mut payload = original.clone();
        disclose(&mut payload, true);
        assert_eq!(payload, original);
    }
}
