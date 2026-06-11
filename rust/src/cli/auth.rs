use chrono::{Duration, NaiveDateTime, Utc};

use crate::cli::{prompt_password, prompt_string};
use crate::config::FileConfig;
use crate::error::{CsError, Result};
use crate::provider::ClaudeProvider;

const EXPIRY_FORMAT: &str = "%a, %d %b %Y %H:%M:%S";

fn default_expiry() -> NaiveDateTime {
    use chrono::Timelike;
    // 30 days out, truncated to whole seconds (matches the strftime roundtrip
    // in the Python version)
    let expires = (Utc::now() + Duration::days(30)).naive_utc();
    expires.with_nanosecond(0).unwrap_or(expires)
}

fn parse_expiry(s: &str) -> Option<NaiveDateTime> {
    // Accept the "Tue, 10 Jun 2026 12:00:00 UTC" format (with or without
    // the trailing timezone name)
    let trimmed = s.trim().trim_end_matches(" UTC").trim_end_matches(" GMT");
    NaiveDateTime::parse_from_str(trimmed, EXPIRY_FORMAT).ok()
}

fn prompt_session_key_expiry() -> Result<NaiveDateTime> {
    loop {
        let default = format!("{} UTC", default_expiry().format(EXPIRY_FORMAT));
        let input = prompt_string(
            "Please enter the expires time for the sessionKey (optional)",
            Some(&default),
        )?;
        match parse_expiry(&input) {
            Some(dt) => return Ok(dt),
            None => println!(
                "The entered date does not match the required format. Please try again."
            ),
        }
    }
}

fn is_url_encoded(s: &str) -> bool {
    let re = regex::Regex::new(r"%[0-9A-Fa-f]{2}").unwrap();
    re.is_match(s)
}

fn display_login_instructions(api_url: &str) {
    println!("A session key is required to call: {api_url}");
    println!("To obtain your session key, please follow these steps:");
    println!("1. Open your web browser and go to https://claude.ai");
    println!("2. Log in to your Claude account if you haven't already");
    println!("3. Once logged in, open your browser's developer tools:");
    println!("   - Chrome/Edge: Press F12 or Ctrl+Shift+I (Cmd+Option+I on Mac)");
    println!("   - Firefox: Press F12 or Ctrl+Shift+I (Cmd+Option+I on Mac)");
    println!("   - Safari: Enable developer tools in Preferences > Advanced, then press Cmd+Option+I");
    println!("4. In the developer tools, go to the 'Application' tab (Chrome/Edge) or 'Storage' tab (Firefox)");
    println!("5. In the left sidebar, expand 'Cookies' and select 'https://claude.ai'");
    println!("6. Locate the cookie named 'sessionKey' and copy its value. Ensure that the value is not URL-encoded.");
}

fn validate_session_key(
    config: &FileConfig,
    provider_name: &str,
    session_key: &str,
    expires: NaiveDateTime,
) -> Result<()> {
    config.set_session_key(provider_name, session_key, expires)?;
    let base_url = config
        .get_str("claude_api_url")
        .unwrap_or_else(|| "https://claude.ai/api".to_string());
    let provider = ClaudeProvider::new(base_url, session_key.to_string());
    let organizations = provider.get_organizations()?;
    if organizations.is_empty() {
        return Err(CsError::Provider(
            "Unable to retrieve organization information".into(),
        ));
    }
    Ok(())
}

/// Reads the session key from the system clipboard (macOS `pbpaste`).
fn read_clipboard_session_key() -> Result<String> {
    let output = std::process::Command::new("pbpaste").output().map_err(|_| {
        CsError::Configuration(
            "--from-clipboard requires the macOS 'pbpaste' command. \
             Use --session-key or the interactive prompt instead."
                .into(),
        )
    })?;
    if !output.status.success() {
        return Err(CsError::Configuration("Failed to read the clipboard.".into()));
    }
    let key = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if key.is_empty() {
        return Err(CsError::Configuration(
            "Clipboard is empty. Copy the sessionKey cookie value first.".into(),
        ));
    }
    println!("Using session key from clipboard.");
    Ok(key)
}

pub fn login(
    config: &mut FileConfig,
    provider_name: &str,
    session_key: Option<String>,
    auto_approve: bool,
    from_clipboard: bool,
) -> Result<()> {
    let session_key = match (session_key, from_clipboard) {
        (Some(key), _) => Some(key),
        (None, true) => Some(read_clipboard_session_key()?),
        (None, false) => None,
    };

    if let Some(key) = session_key {
        // Non-interactive: a session key was provided directly
        if !key.starts_with("sk-ant") {
            return Err(CsError::Provider(
                "Invalid sessionKey format. Must start with 'sk-ant'".into(),
            ));
        }
        let expires = if auto_approve {
            default_expiry()
        } else {
            prompt_session_key_expiry()?
        };
        validate_session_key(config, provider_name, &key, expires)
            .map_err(|e| CsError::Provider(format!("Invalid session key: {e}")))?;
        println!("Successfully authenticated with {provider_name}. Session key stored globally.");
        return Ok(());
    }

    // Interactive login flow
    let api_url = config.get_str("claude_api_url").unwrap_or_default();
    display_login_instructions(&api_url);

    loop {
        let key = loop {
            let key = prompt_password("Please enter your sessionKey")?;
            if !key.starts_with("sk-ant") {
                println!("Invalid sessionKey format. Please make sure it starts with 'sk-ant'.");
                continue;
            }
            if is_url_encoded(&key) {
                println!("The session key appears to be URL-encoded. Please provide the decoded version.");
                continue;
            }
            break key;
        };
        let expires = prompt_session_key_expiry()?;

        match validate_session_key(config, provider_name, &key, expires) {
            Ok(()) => {
                println!(
                    "Successfully authenticated with {provider_name}. Session key stored globally."
                );
                return Ok(());
            }
            Err(e) => {
                println!("{e}");
                println!("Failed to retrieve organizations. Please enter a valid sessionKey.");
            }
        }
    }
}

pub fn logout(config: &FileConfig) -> Result<()> {
    config.clear_all_session_keys()?;
    println!("Logged out from all providers successfully.");
    Ok(())
}

pub fn ls(config: &FileConfig) -> Result<()> {
    let providers = config.get_providers_with_session_keys()?;
    if providers.is_empty() {
        println!("No authenticated providers found.");
    } else {
        println!("Authenticated providers:");
        for provider in providers {
            println!("  - {provider}");
        }
    }
    Ok(())
}
