use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::Value;
use walkdir::WalkDir;

use crate::config::FileConfig;
use crate::error::{CsError, Result};
use crate::provider::{get_provider, ClaudeProvider};

/// MD5 of content after normalizing line endings to `\n` and trimming.
/// Kept for API parity with the Python `utils` module.
#[allow(dead_code)]
pub fn normalize_and_calculate_md5(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    compute_md5_hash(normalized.trim())
}

pub fn compute_md5_hash(content: &str) -> String {
    format!("{:x}", md5::compute(content.as_bytes()))
}

/// Loads .gitignore-style patterns from a file into a matcher.
fn load_ignore_file(base_path: &Path, name: &str) -> Option<Gitignore> {
    let path = base_path.join(name);
    if !path.exists() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(base_path);
    builder.add(path);
    builder.build().ok()
}

pub fn load_gitignore(base_path: &Path) -> Option<Gitignore> {
    load_ignore_file(base_path, ".gitignore")
}

pub fn load_claudeignore(base_path: &Path) -> Option<Gitignore> {
    load_ignore_file(base_path, ".claudeignore")
}

/// Builds a matcher from raw gitwildmatch patterns (used for file categories).
fn build_pattern_spec(base_path: &Path, patterns: &[String]) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(base_path);
    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .map_err(|e| CsError::Configuration(format!("Invalid pattern '{pattern}': {e}")))?;
    }
    builder
        .build()
        .map_err(|e| CsError::Configuration(format!("Invalid patterns: {e}")))
}

fn matches(spec: &Option<Gitignore>, rel_path: &Path, is_dir: bool) -> bool {
    match spec {
        Some(s) => s
            .matched_path_or_any_parents(rel_path, is_dir)
            .is_ignore(),
        None => false,
    }
}

/// Heuristic: a file is text if its first 8KiB contain no NUL byte.
pub fn is_text_file(file_path: &Path) -> bool {
    let mut buf = [0u8; 8192];
    match fs::File::open(file_path) {
        Ok(mut f) => match f.read(&mut buf) {
            Ok(n) => !buf[..n].contains(&0),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

fn should_process_file(
    config: &FileConfig,
    file_path: &Path,
    filename: &str,
    gitignore: &Option<Gitignore>,
    claudeignore: &Option<Gitignore>,
    rel_path: &Path,
) -> bool {
    let max_file_size = config.get_u64("max_file_size", 32 * 1024);
    match fs::metadata(file_path) {
        Ok(m) if m.len() > max_file_size => return false,
        Err(_) => return false,
        _ => {}
    }
    // Skip temporary editor files
    if filename.ends_with('~') {
        return false;
    }
    if matches(gitignore, rel_path, false) || matches(claudeignore, rel_path, false) {
        return false;
    }
    is_text_file(file_path)
}

/// Reads a file as UTF-8 and computes its MD5 hash; None if unreadable/binary.
fn process_file(file_path: &Path) -> Option<String> {
    let bytes = fs::read(file_path).ok()?;
    let content = String::from_utf8(bytes).ok()?;
    Some(compute_md5_hash(&content))
}

/// Port of `get_local_files`: walks `local_path` applying gitignore,
/// claudeignore, category patterns, size and text-file filters.
pub fn get_local_files(
    config: &FileConfig,
    local_path: &Path,
    category: Option<&str>,
    include_submodules: bool,
) -> Result<BTreeMap<String, String>> {
    let gitignore = load_gitignore(local_path);
    let claudeignore = load_claudeignore(local_path);
    let mut files = BTreeMap::new();

    let exclude_dirs: HashSet<&str> = [
        ".git",
        ".svn",
        ".hg",
        ".bzr",
        "_darcs",
        "CVS",
        "claude_chats",
        ".claudesync",
    ]
    .into_iter()
    .collect();

    let categories = config
        .get("file_categories")
        .unwrap_or(Value::Object(Default::default()));
    let patterns: Vec<String> = match category {
        Some(cat) => {
            let entry = categories.get(cat).ok_or_else(|| {
                CsError::Configuration(format!("Invalid category: {cat}"))
            })?;
            entry["patterns"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default()
        }
        None => vec!["*".to_string()],
    };
    let spec = build_pattern_spec(local_path, &patterns)?;

    let submodule_paths: HashSet<String> = config
        .get("submodules")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|sm| sm.get("relative_path").and_then(Value::as_str))
        .map(|s| s.to_string())
        .collect();

    let local_path_owned = local_path.to_path_buf();
    let gitignore_dirs = gitignore.clone();
    let claudeignore_dirs = claudeignore.clone();
    let walker = WalkDir::new(local_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| {
            if entry.depth() == 0 || !entry.file_type().is_dir() {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            if exclude_dirs.contains(name.as_ref()) {
                return false;
            }
            let rel = match entry.path().strip_prefix(&local_path_owned) {
                Ok(r) => r,
                Err(_) => return true,
            };
            if !include_submodules
                && submodule_paths.contains(&rel.to_string_lossy().replace('\\', "/"))
            {
                return false;
            }
            !(matches(&gitignore_dirs, rel, true) || matches(&claudeignore_dirs, rel, true))
        });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(local_path)
            .unwrap_or(entry.path());
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let filename = entry.file_name().to_string_lossy();

        if !spec.matched_path_or_any_parents(rel, false).is_ignore() {
            continue;
        }
        if should_process_file(config, entry.path(), &filename, &gitignore, &claudeignore, rel) {
            if let Some(hash) = process_file(entry.path()) {
                files.insert(rel_str, hash);
            }
        }
    }

    Ok(files)
}

/// Detects submodules by indicator filenames (pom.xml, Cargo.toml, ...),
/// respecting .gitignore and .claudeignore. Returns (relative_path, filename).
pub fn detect_submodules(
    base_path: &Path,
    submodule_detect_filenames: &[String],
) -> Vec<(String, String)> {
    let gitignore = load_gitignore(base_path);
    let claudeignore = load_claudeignore(base_path);
    let mut submodules = Vec::new();

    let mut walker = WalkDir::new(base_path).follow_links(false).into_iter();
    while let Some(entry) = walker.next() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_dir() {
            continue;
        }
        let rel_root = entry
            .path()
            .strip_prefix(base_path)
            .unwrap_or(entry.path())
            .to_path_buf();

        if entry.depth() > 0
            && (matches(&gitignore, &rel_root, true) || matches(&claudeignore, &rel_root, true))
        {
            walker.skip_current_dir();
            continue;
        }

        let dir_files: HashSet<String> = fs::read_dir(entry.path())
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();

        for filename in submodule_detect_filenames {
            if dir_files.contains(filename) {
                let relative_path = rel_root.to_string_lossy().replace('\\', "/");
                if !relative_path.is_empty() && relative_path != "." {
                    let file_rel = rel_root.join(filename);
                    if matches(&gitignore, &file_rel, false)
                        || matches(&claudeignore, &file_rel, false)
                    {
                        continue;
                    }
                    submodules.push((relative_path, filename.clone()));
                }
                break; // One indicator per directory is enough
            }
        }
    }

    submodules
}

/// Port of `validate_and_get_provider`: checks org/project/provider/session
/// key requirements, then returns a provider instance.
pub fn validate_and_get_provider(
    config: &FileConfig,
    require_org: bool,
    require_project: bool,
) -> Result<ClaudeProvider> {
    if require_org && config.get_str("active_organization_id").is_none() {
        return Err(CsError::Configuration(
            "No active organization set. Please select an organization (claudesync organization set)."
                .into(),
        ));
    }
    if require_project && config.get_str("active_project_id").is_none() {
        return Err(CsError::Configuration(
            "No active project set. Please select or create a project (claudesync project set)."
                .into(),
        ));
    }
    let active_provider = config.get_active_provider().ok_or_else(|| {
        CsError::Configuration(
            "No active provider set. Please select a provider for this project.".into(),
        )
    })?;
    let session = config.get_session_key(&active_provider)?;
    if session.is_none() {
        return Err(CsError::Configuration(format!(
            "No valid session key found for {active_provider}. Please log in again."
        )));
    }
    get_provider(config, &active_provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_normalizes_line_endings() {
        let unix = "line one\nline two\n";
        let windows = "line one\r\nline two\r\n";
        let old_mac = "line one\rline two\r";
        assert_eq!(
            normalize_and_calculate_md5(unix),
            normalize_and_calculate_md5(windows)
        );
        assert_eq!(
            normalize_and_calculate_md5(unix),
            normalize_and_calculate_md5(old_mac)
        );
    }

    #[test]
    fn detects_text_vs_binary() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("a.txt");
        let binary = dir.path().join("b.bin");
        fs::write(&text, "hello world").unwrap();
        fs::write(&binary, [0u8, 1, 2, 3]).unwrap();
        assert!(is_text_file(&text));
        assert!(!is_text_file(&binary));
    }
}
