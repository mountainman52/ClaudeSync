use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use base64::engine::general_purpose::URL_SAFE;
use base64::Engine as _;
use fernet::Fernet;
use sha2::Sha256;

use crate::error::{CsError, Result};

/// Port of `SessionKeyManager`: encrypts the Claude session key with a
/// Fernet key derived (PBKDF2-HMAC-SHA256) from a local SSH private key.
pub struct SessionKeyManager {
    ssh_key_path: PathBuf,
}

impl SessionKeyManager {
    pub fn new(configured_path: Option<&str>) -> Result<Self> {
        let ssh_key_path = Self::find_ssh_key(configured_path)?;
        Ok(SessionKeyManager { ssh_key_path })
    }

    /// Locate an SSH private key without user interaction.
    /// Priority:
    ///   1. If configured_path points to a specific file, use it directly
    ///   2. If configured_path is a directory, search it alongside the default dir
    ///   3. Fall back to the default dir (~/.ssh) with default key names
    fn locate_ssh_key(configured_path: Option<&str>, default_ssh_dir: &std::path::Path) -> Option<PathBuf> {
        let key_names = ["id_ed25519", "id_ecdsa"];
        let mut search_dirs = vec![default_ssh_dir.to_path_buf()];

        if let Some(configured) = configured_path {
            let configured = PathBuf::from(configured);
            if configured.is_file() {
                return Some(configured);
            }
            if configured.is_dir() {
                if configured != default_ssh_dir {
                    search_dirs.insert(0, configured);
                }
            } else {
                log::warn!("Configured ssh_key_path not found: {}", configured.display());
            }
        }

        for dir in &search_dirs {
            for name in &key_names {
                let candidate = dir.join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Locate an SSH private key, prompting the user as a last resort.
    fn find_ssh_key(configured_path: Option<&str>) -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| CsError::Configuration("Cannot determine home directory".into()))?;
        if let Some(found) = Self::locate_ssh_key(configured_path, &home.join(".ssh")) {
            return Ok(found);
        }

        eprintln!("* No supported SSH key found. RSA keys are no longer supported.");
        eprintln!("* Please generate an Ed25519 key using the following command:");
        eprintln!("  ssh-keygen -t ed25519 -C \"your_email@example.com\"");
        eprintln!("* If you have NOT specified a custom ssh_key_path in config,");
        eprintln!("* have created a key, and are still seeing this message,");
        eprintln!("  be sure to name your key 'id_ed25519' or 'id_ecdsa' so it's found automatically.");
        eprintln!("* Or set ssh_key_path with the full key name in your .ctxsync/config.local.json");
        print!("Enter the full path to your new Ed25519 private key: ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        Ok(PathBuf::from(input.trim()))
    }

    /// Verifies the key is a supported type (ed25519 or ecdsa) via ssh-keygen.
    fn check_key_type(&self) -> Result<()> {
        let output = Command::new("ssh-keygen")
            .args(["-l", "-f"])
            .arg(&self.ssh_key_path)
            .output()
            .map_err(|e| {
                CsError::Other(format!(
                    "Failed to determine SSH key type. Make sure the key file is valid and accessible. ({e})"
                ))
            })?;
        if !output.status.success() {
            return Err(CsError::Other(
                "Failed to determine SSH key type. Make sure the key file is valid and accessible."
                    .into(),
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
        if stdout.contains("ecdsa") || stdout.contains("ed25519") {
            Ok(())
        } else {
            Err(CsError::Other(format!(
                "Unsupported key type for {}",
                self.ssh_key_path.display()
            )))
        }
    }

    fn derive_fernet_key(&self) -> Result<String> {
        let key_data = std::fs::read(&self.ssh_key_path)?;
        let mut derived = [0u8; 32];
        // Fixed salt and iteration count match the Python implementation so
        // keys encrypted by either version stay interchangeable.
        // Salt kept from the ClaudeSync days: changing it would orphan existing keys
        pbkdf2::pbkdf2_hmac::<Sha256>(&key_data, b"claudesync", 100_000, &mut derived);
        Ok(URL_SAFE.encode(derived))
    }

    pub fn encrypt_session_key(&self, session_key: &str) -> Result<(String, String)> {
        self.check_key_type()?;
        self.encrypt_symmetric(session_key)
    }

    fn encrypt_symmetric(&self, session_key: &str) -> Result<(String, String)> {
        let key = self.derive_fernet_key()?;
        let fernet = Fernet::new(&key)
            .ok_or_else(|| CsError::Other("Failed to construct Fernet key".into()))?;
        Ok((fernet.encrypt(session_key.as_bytes()), "symmetric".into()))
    }

    pub fn decrypt_session_key(&self, method: &str, encrypted: &str) -> Result<String> {
        if encrypted.is_empty() || method.is_empty() {
            return Err(CsError::Other("Missing encrypted key or method".into()));
        }
        if method != "symmetric" {
            return Err(CsError::Other(format!("Unknown encryption method: {method}")));
        }
        let key = self.derive_fernet_key()?;
        let fernet = Fernet::new(&key)
            .ok_or_else(|| CsError::Other("Failed to construct Fernet key".into()))?;
        let plain = fernet
            .decrypt(encrypted)
            .map_err(|e| CsError::Other(format!("Failed to decrypt session key: {e}")))?;
        String::from_utf8(plain).map_err(|e| CsError::Other(e.to_string()))
    }

    #[cfg(test)]
    fn with_key_path(path: &std::path::Path) -> Self {
        SessionKeyManager {
            ssh_key_path: path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ports of tests/test_session_key_manager.py (key discovery logic)

    #[test]
    fn configured_file_path_used_directly() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("my_custom_key");
        std::fs::write(&key_file, "fake-key-content").unwrap();

        let found = SessionKeyManager::locate_ssh_key(
            key_file.to_str(),
            &dir.path().join("unused_default"),
        );
        assert_eq!(found, Some(key_file));
    }

    #[test]
    fn configured_directory_searched_for_standard_names() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("id_ed25519");
        std::fs::write(&key_file, "fake-key-content").unwrap();

        let found = SessionKeyManager::locate_ssh_key(
            dir.path().to_str(),
            &dir.path().join("unused_default"),
        );
        assert_eq!(found, Some(key_file));
    }

    #[test]
    fn falls_back_to_default_ssh_dir() {
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        std::fs::create_dir(&ssh_dir).unwrap();
        let key_file = ssh_dir.join("id_ecdsa");
        std::fs::write(&key_file, "fake-key-content").unwrap();

        let found = SessionKeyManager::locate_ssh_key(None, &ssh_dir);
        assert_eq!(found, Some(key_file));
    }

    #[test]
    fn nonexistent_configured_path_falls_through_to_default() {
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        std::fs::create_dir(&ssh_dir).unwrap();
        let key_file = ssh_dir.join("id_ed25519");
        std::fs::write(&key_file, "fake-key-content").unwrap();

        let found = SessionKeyManager::locate_ssh_key(Some("/nonexistent/path"), &ssh_dir);
        assert_eq!(found, Some(key_file));
    }

    #[test]
    fn no_key_anywhere_returns_none() {
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        std::fs::create_dir(&ssh_dir).unwrap();

        assert_eq!(SessionKeyManager::locate_ssh_key(None, &ssh_dir), None);
    }

    #[test]
    fn symmetric_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("id_ed25519");
        std::fs::write(&key_file, b"fake ssh key material for tests").unwrap();
        let mgr = SessionKeyManager::with_key_path(&key_file);

        let (encrypted, method) = mgr.encrypt_symmetric("sk-ant-test-key").unwrap();
        assert_eq!(method, "symmetric");
        let decrypted = mgr.decrypt_session_key(&method, &encrypted).unwrap();
        assert_eq!(decrypted, "sk-ant-test-key");
    }
}
