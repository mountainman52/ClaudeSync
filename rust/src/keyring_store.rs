//! Session keys in the operating system's secure credential store: the
//! macOS Keychain (via the Security framework), Windows Credential Manager,
//! or the Linux kernel keyring.
//!
//! Compared to the file backend (Fernet encryption keyed off the user's SSH
//! private key with a fixed salt), the OS store encrypts at rest, unlocks
//! with the login session, and gates access per application.

use chrono::NaiveDateTime;
use serde_json::{json, Value};

use crate::config::parse_iso_datetime;
use crate::error::{CsError, Result};

const SERVICE: &str = "ctxsync";
/// The service name from before the tool was renamed; items stored under it
/// are migrated to the new name on first read.
const LEGACY_SERVICE: &str = "claudesync";

/// Platforms where the credential store is reliable enough to be the
/// default. (The Linux kernel keyring does not survive reboots, so it is
/// opt-in via `session_key_storage = "keyring"` there.)
pub fn is_default_platform() -> bool {
    cfg!(target_os = "macos")
}

fn entry(provider: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, provider)
        .map_err(|e| CsError::Other(format!("Credential store unavailable: {e}")))
}

/// If no item exists under the new service name, moves any item stored under
/// the legacy "claudesync" service to the new one. Best-effort: failures
/// leave the legacy item in place.
fn migrate_legacy_entry(provider: &str) {
    let Ok(new_entry) = keyring::Entry::new(SERVICE, provider) else {
        return;
    };
    if new_entry.get_password().is_ok() {
        return;
    }
    let Ok(legacy) = keyring::Entry::new(LEGACY_SERVICE, provider) else {
        return;
    };
    if let Ok(payload) = legacy.get_password() {
        if new_entry.set_password(&payload).is_ok() {
            let _ = legacy.delete_credential();
        }
    }
}

fn encode_payload(session_key: &str, expiry: NaiveDateTime) -> String {
    json!({
        "session_key": session_key,
        "session_key_expiry": expiry.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
    })
    .to_string()
}

fn decode_payload(payload: &str) -> Option<(String, NaiveDateTime)> {
    let v: Value = serde_json::from_str(payload).ok()?;
    let key = v.get("session_key")?.as_str()?.to_string();
    let expiry = parse_iso_datetime(v.get("session_key_expiry")?.as_str()?)?;
    Some((key, expiry))
}

pub fn store(provider: &str, session_key: &str, expiry: NaiveDateTime) -> Result<()> {
    entry(provider)?
        .set_password(&encode_payload(session_key, expiry))
        .map_err(|e| {
            CsError::Other(format!(
                "Failed to store session key in the credential store: {e}"
            ))
        })
}

/// Returns the stored session key if present and unexpired.
pub fn retrieve(provider: &str) -> Result<Option<(String, NaiveDateTime)>> {
    migrate_legacy_entry(provider);
    match entry(provider)?.get_password() {
        Ok(payload) => Ok(decode_payload(&payload)
            .filter(|(_, expiry)| chrono::Utc::now().naive_utc() <= *expiry)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(CsError::Other(format!(
            "Failed to read session key from the credential store: {e}"
        ))),
    }
}

/// Removes the stored session key; returns whether one existed.
pub fn delete(provider: &str) -> Result<bool> {
    match entry(provider)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(CsError::Other(format!(
            "Failed to delete session key from the credential store: {e}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        let expiry = parse_iso_datetime("2099-09-26T17:07:53").unwrap();
        let payload = encode_payload("sk-ant-test", expiry);
        let (key, parsed) = decode_payload(&payload).unwrap();
        assert_eq!(key, "sk-ant-test");
        assert_eq!(parsed, expiry);
    }

    #[test]
    fn rejects_malformed_payloads() {
        assert!(decode_payload("not json").is_none());
        assert!(decode_payload("{\"session_key\": \"x\"}").is_none());
    }
}
