use std::process::{Command, Stdio};

use crate::cli::prompt_usize;
use crate::error::{CsError, Result};

/// Set up automated synchronization at regular intervals
/// (port of the `schedule` command).
pub fn schedule(interval: Option<u32>) -> Result<()> {
    let interval = match interval {
        Some(i) => i as usize,
        None => prompt_usize("Enter sync interval in minutes", Some(5))?,
    };

    let claudesync_path = which_claudesync().ok_or_else(|| {
        CsError::Configuration(
            "Error: claudesync not found in PATH. Please ensure it's installed correctly.".into(),
        )
    })?;

    if cfg!(windows) {
        setup_windows_task(&claudesync_path, interval);
    } else {
        setup_unix_cron(&claudesync_path, interval)?;
    }
    Ok(())
}

fn which_claudesync() -> Option<String> {
    // Prefer the current executable; fall back to PATH lookup
    if let Ok(exe) = std::env::current_exe() {
        return Some(exe.to_string_lossy().to_string());
    }
    let output = Command::new("which").arg("claudesync").output().ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

fn setup_windows_task(claudesync_path: &str, interval: usize) {
    println!("Windows Task Scheduler setup:");
    let command = format!(
        "schtasks /create /tn \"ClaudeSync\" /tr \"{claudesync_path} push\" /sc minute /mo {interval}"
    );
    println!("Run this command to create the task:\n{command}");
    println!("\nTo remove the task, run: schtasks /delete /tn \"ClaudeSync\" /f");
}

fn setup_unix_cron(claudesync_path: &str, interval: usize) -> Result<()> {
    // Read existing crontab (an empty one is fine)
    let existing = Command::new("crontab")
        .arg("-l")
        .output()
        .map(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout).to_string()
            } else {
                String::new()
            }
        })
        .unwrap_or_default();

    let new_entry = format!("*/{interval} * * * * {claudesync_path} push # ClaudeSync");
    let mut crontab = existing;
    if !crontab.is_empty() && !crontab.ends_with('\n') {
        crontab.push('\n');
    }
    crontab.push_str(&new_entry);
    crontab.push('\n');

    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| CsError::Other(format!("Failed to run crontab: {e}")))?;
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| CsError::Other("Failed to open crontab stdin".into()))?
        .write_all(crontab.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        return Err(CsError::Other("Failed to install crontab entry".into()));
    }

    println!("Cron job created successfully! It will run every {interval} minutes.");
    println!("\nTo remove the cron job, run: crontab -e and remove the line for ClaudeSync");
    Ok(())
}
