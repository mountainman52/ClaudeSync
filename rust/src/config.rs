use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use chrono::NaiveDateTime;
use serde_json::{json, Map, Value};

use crate::error::{CsError, Result};
use crate::keyring_store;
use crate::session_key::SessionKeyManager;

/// Providers the CLI knows about (used to enumerate credential-store keys,
/// which cannot be listed the way `*.key` files can).
const KNOWN_PROVIDERS: [&str; 1] = ["claude.ai"];

/// Where session keys are stored (config key `session_key_storage`).
#[derive(PartialEq)]
enum SessionKeyStorage {
    /// OS credential store on macOS, file elsewhere
    Auto,
    /// Always the OS credential store (Keychain / Credential Manager / keyutils)
    Keyring,
    /// Always the SSH-key-encrypted file (compatible with the Python version)
    File,
}

/// Returns the default configuration (port of `BaseConfigManager._get_default_config`).
pub fn default_config() -> Map<String, Value> {
    let v = json!({
        "log_level": "INFO",
        "upload_delay": 0.5,
        "max_file_size": 32 * 1024,
        "two_way_sync": false,
        "prune_remote_files": true,
        "claude_api_url": "https://claude.ai/api",
        "compression_algorithm": "none",
        // "auto" (OS credential store on macOS, file elsewhere),
        // "keychain"/"keyring", or "file"
        "session_key_storage": "auto",
        "submodule_detect_filenames": [
            "pom.xml",
            "build.gradle",
            "package.json",
            "setup.py",
            "Cargo.toml",
            "go.mod",
        ],
        "file_categories": {
            "all_files": {
                "description": "All files not ignored",
                "patterns": ["*"],
            },
            "all_source_code": {
                "description": "All source code files",
                "patterns": [
                    "*.java", "*.py", "*.js", "*.ts", "*.c", "*.cpp", "*.h", "*.hpp",
                    "*.go", "*.rs",
                ],
            },
            "production_code": {
                "description": "Production source code",
                "patterns": [
                    "**/src/**/*.java", "**/*.py", "**/*.js", "**/*.ts", "**/*.vue",
                ],
            },
            "test_code": {
                "description": "Test source code",
                "patterns": [
                    "**/test/**/*.java", "**/tests/**/*.py", "**/test_*.py",
                    "**/*Test.java",
                ],
            },
            "build_config": {
                "description": "Build configuration files",
                "patterns": [
                    "**/pom.xml", "**/build.gradle", "**/package.json", "**/setup.py",
                    "**/Cargo.toml", "**/go.mod", "**/pyproject.toml",
                    "**/requirements.txt", "**/*.tf", "**/*.yaml", "**/*.yml",
                    "**/*.properties",
                ],
            },
            "uberproject_java": {
                "description": "Uberproject Java + Javascript",
                "patterns": [
                    "**/src/**/*.java", "**/*.py", "**/*.js", "**/*.ts", "**/*.vue",
                    "**/pom.xml", "**/build.gradle", "**/package.json", "**/setup.py",
                    "**/Cargo.toml", "**/go.mod", "**/pyproject.toml",
                    "**/requirements.txt", "**/*.tf", "**/*.yaml", "**/*.yml",
                    "**/*.properties",
                ],
            },
        },
    });
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// Python truthiness, used to mirror `local.get(key) or global.get(key, default)`.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Port of `FileConfigManager`: global config in ~/.claudesync/config.json and
/// local config in <project>/.claudesync/config.local.json. Session keys are
/// stored encrypted in ~/.claudesync/<provider>.key.
pub struct FileConfig {
    pub global_config: Map<String, Value>,
    pub local_config: Map<String, Value>,
    pub global_config_dir: PathBuf,
    global_config_file: PathBuf,
    pub local_config_dir: Option<PathBuf>,
}

impl FileConfig {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir()
            .ok_or_else(|| CsError::Configuration("Cannot determine home directory".into()))?;
        let global_config_dir = home.join(".claudesync");
        let global_config_file = global_config_dir.join("config.json");

        let mut cfg = FileConfig {
            global_config: Map::new(),
            local_config: Map::new(),
            global_config_dir,
            global_config_file,
            local_config_dir: None,
        };
        cfg.global_config = cfg.load_global_config()?;
        cfg.load_local_config()?;
        Ok(cfg)
    }

    fn load_global_config(&self) -> Result<Map<String, Value>> {
        if !self.global_config_file.exists() {
            fs::create_dir_all(&self.global_config_dir)?;
            return Ok(default_config());
        }
        let text = fs::read_to_string(&self.global_config_file)?;
        let mut config: Map<String, Value> = serde_json::from_str(&text)?;
        for (key, value) in default_config() {
            config.entry(key).or_insert(value);
        }
        Ok(config)
    }

    /// Finds the nearest ancestor directory containing a `.claudesync` folder,
    /// excluding `~/.claudesync` itself.
    fn find_local_config_dir(max_depth: usize) -> Option<PathBuf> {
        let home = dirs::home_dir().unwrap_or_default();
        let mut current = std::env::current_dir().ok()?;
        let mut depth = 0;
        loop {
            let claudesync_dir = current.join(".claudesync");
            if claudesync_dir.is_dir() && claudesync_dir != home.join(".claudesync") {
                return Some(current);
            }
            let parent = current.parent()?.to_path_buf();
            if parent == current {
                return None;
            }
            current = parent;
            depth += 1;
            if depth > max_depth {
                return None;
            }
        }
    }

    fn load_local_config(&mut self) -> Result<()> {
        self.local_config_dir = Self::find_local_config_dir(100);
        if let Some(dir) = self.local_config_dir.clone() {
            let local_file = dir.join(".claudesync").join("config.local.json");
            if local_file.exists() {
                let text = fs::read_to_string(&local_file)?;
                self.local_config = serde_json::from_str(&text)?;

                // Normalize Windows-style paths in submodules
                let mut needs_save = false;
                if let Some(Value::Array(submodules)) = self.local_config.get_mut("submodules") {
                    for sm in submodules.iter_mut() {
                        if let Some(rel) = sm.get("relative_path").and_then(Value::as_str) {
                            if rel.contains('\\') {
                                let fixed = rel.replace('\\', "/");
                                sm["relative_path"] = Value::String(fixed);
                                needs_save = true;
                            }
                        }
                    }
                }
                if needs_save {
                    self.save_local_config()?;
                }
            }
        }
        Ok(())
    }

    pub fn get_local_path(&self) -> Option<PathBuf> {
        self.local_config_dir.clone()
    }

    /// Local config first (Python truthiness), then global.
    pub fn get(&self, key: &str) -> Option<Value> {
        if let Some(v) = self.local_config.get(key) {
            if is_truthy(v) {
                return Some(v.clone());
            }
        }
        self.global_config.get(key).cloned()
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        self.get(key)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    pub fn get_f64(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
    }

    pub fn get_u64(&self, key: &str, default: u64) -> u64 {
        self.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
    }

    pub fn set(&mut self, key: &str, value: Value, local: bool) -> Result<()> {
        if local {
            if key == "local_path" {
                if let Some(path) = value.as_str() {
                    let dir = PathBuf::from(path);
                    fs::create_dir_all(dir.join(".claudesync"))?;
                    self.local_config_dir = Some(dir);
                }
            }
            self.local_config.insert(key.to_string(), value);
            self.save_local_config()
        } else {
            self.global_config.insert(key.to_string(), value);
            self.save_global_config()
        }
    }

    pub fn save_global_config(&self) -> Result<()> {
        let text = serde_json::to_string_pretty(&self.global_config)?;
        fs::write(&self.global_config_file, text)?;
        Ok(())
    }

    pub fn save_local_config(&self) -> Result<()> {
        if let Some(dir) = &self.local_config_dir {
            let local_file = dir.join(".claudesync").join("config.local.json");
            if let Some(parent) = local_file.parent() {
                fs::create_dir_all(parent)?;
            }
            let text = serde_json::to_string_pretty(&self.local_config)?;
            fs::write(local_file, text)?;
        }
        Ok(())
    }

    fn session_key_storage(&self) -> SessionKeyStorage {
        match self.get_str("session_key_storage").as_deref() {
            Some("keychain") | Some("keyring") => SessionKeyStorage::Keyring,
            Some("file") => SessionKeyStorage::File,
            _ => SessionKeyStorage::Auto,
        }
    }

    pub fn set_session_key(
        &self,
        provider: &str,
        session_key: &str,
        expiry: NaiveDateTime,
    ) -> Result<()> {
        match self.session_key_storage() {
            SessionKeyStorage::Keyring => {
                keyring_store::store(provider, session_key, expiry)?;
                // Remove the weaker file-encrypted copy now that the OS
                // credential store holds the key
                let _ = self.remove_session_key_file(provider);
                Ok(())
            }
            SessionKeyStorage::File => self.set_session_key_file(provider, session_key, expiry),
            SessionKeyStorage::Auto => {
                if keyring_store::is_default_platform() {
                    match keyring_store::store(provider, session_key, expiry) {
                        Ok(()) => {
                            let _ = self.remove_session_key_file(provider);
                            return Ok(());
                        }
                        Err(e) => {
                            log::warn!("Credential store unavailable ({e}); falling back to file storage");
                        }
                    }
                }
                self.set_session_key_file(provider, session_key, expiry)
            }
        }
    }

    fn set_session_key_file(
        &self,
        provider: &str,
        session_key: &str,
        expiry: NaiveDateTime,
    ) -> Result<()> {
        let manager = SessionKeyManager::new(self.get_str("ssh_key_path").as_deref())?;
        let (encrypted, method) = manager.encrypt_session_key(session_key)?;

        fs::create_dir_all(&self.global_config_dir)?;
        let key_file = self.global_config_dir.join(format!("{provider}.key"));
        let data = json!({
            "session_key": encrypted,
            "session_key_encryption_method": method,
            "session_key_expiry": expiry.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
        });
        fs::write(key_file, serde_json::to_string(&data)?)?;
        Ok(())
    }

    fn remove_session_key_file(&self, provider: &str) -> Result<()> {
        let key_file = self.global_config_dir.join(format!("{provider}.key"));
        if key_file.exists() {
            fs::remove_file(key_file)?;
        }
        Ok(())
    }

    /// Returns (session_key, expiry) if a valid, unexpired key exists.
    pub fn get_session_key(&self, provider: &str) -> Result<Option<(String, NaiveDateTime)>> {
        match self.session_key_storage() {
            SessionKeyStorage::Keyring => keyring_store::retrieve(provider),
            SessionKeyStorage::File => self.get_session_key_file(provider),
            SessionKeyStorage::Auto => {
                if keyring_store::is_default_platform() {
                    match keyring_store::retrieve(provider) {
                        Ok(Some(found)) => return Ok(Some(found)),
                        Ok(None) => {} // fall through to a pre-existing file key
                        Err(e) => log::warn!("Credential store read failed: {e}"),
                    }
                }
                self.get_session_key_file(provider)
            }
        }
    }

    fn get_session_key_file(&self, provider: &str) -> Result<Option<(String, NaiveDateTime)>> {
        let key_file = self.global_config_dir.join(format!("{provider}.key"));
        if !key_file.exists() {
            return Ok(None);
        }
        let data: Value = serde_json::from_str(&fs::read_to_string(&key_file)?)?;
        let encrypted = data.get("session_key").and_then(Value::as_str);
        let method = data
            .get("session_key_encryption_method")
            .and_then(Value::as_str);
        let expiry_str = data.get("session_key_expiry").and_then(Value::as_str);

        let (encrypted, expiry_str) = match (encrypted, expiry_str) {
            (Some(e), Some(x)) => (e, x),
            _ => return Ok(None),
        };

        let expiry = parse_iso_datetime(expiry_str)
            .ok_or_else(|| CsError::Configuration(format!("Invalid expiry: {expiry_str}")))?;
        // Expiry timestamps are stored as naive UTC, so compare against UTC
        // "now". (The Python version compared against naive local time,
        // skewing expiry by the user's UTC offset.)
        if chrono::Utc::now().naive_utc() > expiry {
            return Ok(None);
        }

        let manager = SessionKeyManager::new(self.get_str("ssh_key_path").as_deref())?;
        match manager.decrypt_session_key(method.unwrap_or(""), encrypted) {
            Ok(key) => Ok(Some((key, expiry))),
            Err(e) => {
                log::error!("Failed to decrypt session key: {e}");
                Ok(None)
            }
        }
    }

    pub fn clear_all_session_keys(&self) -> Result<()> {
        if self.global_config_dir.is_dir() {
            for entry in fs::read_dir(&self.global_config_dir)? {
                let entry = entry?;
                if entry.path().extension().map(|e| e == "key").unwrap_or(false) {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        for provider in KNOWN_PROVIDERS {
            // Best-effort: the credential store may be absent on this platform
            let _ = keyring_store::delete(provider);
        }
        Ok(())
    }

    pub fn get_active_provider(&self) -> Option<String> {
        self.local_config
            .get("active_provider")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
    }

    pub fn get_providers_with_session_keys(&self) -> Result<Vec<String>> {
        // Candidates: any *.key file plus the providers the credential store
        // might hold (the store can't be enumerated by service)
        let mut candidates: BTreeSet<String> =
            KNOWN_PROVIDERS.iter().map(|p| p.to_string()).collect();
        if self.global_config_dir.is_dir() {
            for entry in fs::read_dir(&self.global_config_dir)? {
                let path = entry?.path();
                if path.extension().map(|e| e == "key").unwrap_or(false) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        candidates.insert(stem.to_string());
                    }
                }
            }
        }
        let mut providers = Vec::new();
        for candidate in candidates {
            if self.get_session_key(&candidate)?.is_some() {
                providers.push(candidate);
            }
        }
        Ok(providers)
    }

    pub fn get_default_category(&self) -> Option<String> {
        self.get_str("default_sync_category")
    }

    pub fn set_default_category(&mut self, category: &str) -> Result<()> {
        self.set(
            "default_sync_category",
            Value::String(category.to_string()),
            true,
        )
    }

    pub fn add_file_category(
        &mut self,
        name: &str,
        description: &str,
        patterns: Vec<String>,
    ) -> Result<()> {
        let categories = self
            .global_config
            .entry("file_categories".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(map) = categories {
            map.insert(
                name.to_string(),
                json!({ "description": description, "patterns": patterns }),
            );
        }
        self.save_global_config()
    }

    pub fn remove_file_category(&mut self, name: &str) -> Result<()> {
        if let Some(Value::Object(map)) = self.global_config.get_mut("file_categories") {
            if map.remove(name).is_some() {
                return self.save_global_config();
            }
        }
        Ok(())
    }

    pub fn update_file_category(
        &mut self,
        name: &str,
        description: Option<&str>,
        patterns: Option<Vec<String>>,
    ) -> Result<()> {
        if let Some(Value::Object(map)) = self.global_config.get_mut("file_categories") {
            if let Some(Value::Object(cat)) = map.get_mut(name) {
                if let Some(d) = description {
                    cat.insert("description".to_string(), Value::String(d.to_string()));
                }
                if let Some(p) = patterns {
                    cat.insert("patterns".to_string(), json!(p));
                }
                return self.save_global_config();
            }
        }
        Ok(())
    }
}

/// Parse ISO-8601 datetimes as produced by Python's `datetime.isoformat()`.
pub fn parse_iso_datetime(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt);
        }
    }
    // Fall back to RFC3339 with timezone (e.g. trailing Z or +00:00)
    crate::utils::parse_rfc3339_utc(s).map(|dt| dt.naive_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_keys() {
        let cfg = default_config();
        assert_eq!(cfg["log_level"], "INFO");
        assert_eq!(cfg["claude_api_url"], "https://claude.ai/api");
        assert_eq!(cfg["max_file_size"], 32 * 1024);
        assert!(cfg["file_categories"]
            .as_object()
            .unwrap()
            .contains_key("all_source_code"));
    }

    #[test]
    fn parses_python_isoformat() {
        assert!(parse_iso_datetime("2026-06-10T12:34:56.789012").is_some());
        assert!(parse_iso_datetime("2026-06-10T12:34:56").is_some());
        assert!(parse_iso_datetime("2026-06-10T12:34:56+00:00").is_some());
    }
}
