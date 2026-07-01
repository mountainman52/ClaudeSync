use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::cli::prompt_usize;
use crate::error::{CsError, Result};

const LAUNCHD_LABEL: &str = "com.ctxsync.push";
const CRON_MARKER: &str = "# ctxsync";
// Names used before the tool was renamed; `schedule --remove` still cleans
// these up.
const LEGACY_LAUNCHD_LABEL: &str = "com.claudesync.push";
const LEGACY_CRON_MARKER: &str = "# ClaudeSync";

/// Set up (or remove) automated synchronization at regular intervals.
/// Uses launchd on macOS, cron elsewhere on Unix, and prints Task Scheduler
/// instructions on Windows.
pub fn schedule(interval: Option<u32>, remove: bool) -> Result<()> {
    if remove {
        return remove_schedule();
    }

    let interval = match interval {
        Some(i) => i as usize,
        None => prompt_usize("Enter sync interval in minutes", Some(5))?,
    };

    let ctxsync_path = which_ctxsync().ok_or_else(|| {
        CsError::Configuration(
            "Error: ctxsync not found in PATH. Please ensure it's installed correctly.".into(),
        )
    })?;
    // Scheduled jobs start in an arbitrary directory, so anchor the push to
    // the project the user scheduled from. (The Python version omitted this,
    // so its scheduled sync never found a project.)
    let workdir = std::env::current_dir()?;

    if cfg!(target_os = "macos") {
        setup_launchd(&ctxsync_path, interval, &workdir)
    } else if cfg!(windows) {
        setup_windows_task(&ctxsync_path, interval);
        Ok(())
    } else {
        setup_unix_cron(&ctxsync_path, interval, &workdir)
    }
}

fn remove_schedule() -> Result<()> {
    if cfg!(target_os = "macos") {
        let mut removed = false;
        for label in [LAUNCHD_LABEL, LEGACY_LAUNCHD_LABEL] {
            let plist = launchd_plist_path_for(label)?;
            let _ = Command::new("launchctl")
                .arg("unload")
                .arg(&plist)
                .stderr(Stdio::null())
                .status();
            if plist.exists() {
                std::fs::remove_file(&plist)?;
                println!("Removed launchd job {label} ({}).", plist.display());
                removed = true;
            }
        }
        if !removed {
            println!("No launchd job found.");
        }
        Ok(())
    } else if cfg!(windows) {
        println!("Run this command to remove the task:");
        println!("  schtasks /delete /tn \"ctxsync\" /f");
        Ok(())
    } else {
        let existing = read_crontab();
        let filtered: String = existing
            .lines()
            .filter(|line| !line.contains(CRON_MARKER) && !line.contains(LEGACY_CRON_MARKER))
            .map(|line| format!("{line}\n"))
            .collect();
        if filtered == existing {
            println!("No ctxsync cron entry found.");
            return Ok(());
        }
        write_crontab(&filtered)?;
        println!("Removed ctxsync cron entry.");
        Ok(())
    }
}

fn which_ctxsync() -> Option<String> {
    // Prefer the current executable; fall back to PATH lookup
    if let Ok(exe) = std::env::current_exe() {
        return Some(exe.to_string_lossy().to_string());
    }
    let output = Command::new("which").arg("ctxsync").output().ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

// --- launchd (macOS) ---

fn launchd_plist_path() -> Result<PathBuf> {
    launchd_plist_path_for(LAUNCHD_LABEL)
}

fn launchd_plist_path_for(label: &str) -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| CsError::Configuration("Cannot determine home directory".into()))?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{label}.plist")))
}

fn setup_launchd(ctxsync_path: &str, interval: usize, workdir: &std::path::Path) -> Result<()> {
    let plist_path = launchd_plist_path()?;
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let home = dirs::home_dir()
        .ok_or_else(|| CsError::Configuration("Cannot determine home directory".into()))?;
    let log_dir = home.join("Library").join("Logs");
    std::fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("ctxsync.log");

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{ctxsync_path}</string>
        <string>push</string>
    </array>
    <key>WorkingDirectory</key><string>{workdir}</string>
    <key>StartInterval</key><integer>{seconds}</integer>
    <key>RunAtLoad</key><false/>
    <key>StandardOutPath</key><string>{log}</string>
    <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        workdir = workdir.display(),
        seconds = interval * 60,
        log = log_path.display(),
    );
    std::fs::write(&plist_path, plist)?;

    // Reload if a previous version was loaded
    let _ = Command::new("launchctl")
        .arg("unload")
        .arg(&plist_path)
        .stderr(Stdio::null())
        .status();
    let status = Command::new("launchctl")
        .arg("load")
        .arg(&plist_path)
        .status()
        .map_err(|e| CsError::Other(format!("Failed to run launchctl: {e}")))?;
    if !status.success() {
        return Err(CsError::Other(format!(
            "launchctl load failed for {}",
            plist_path.display()
        )));
    }

    println!("launchd job installed: {}", plist_path.display());
    println!("It will run 'ctxsync push' in {} every {interval} minute(s).", workdir.display());
    println!("Output is logged to {}.", log_path.display());
    println!("\nTo remove it, run: ctxsync schedule --remove");
    Ok(())
}

// --- Windows ---

fn setup_windows_task(ctxsync_path: &str, interval: usize) {
    println!("Windows Task Scheduler setup:");
    let command = format!(
        "schtasks /create /tn \"ctxsync\" /tr \"{ctxsync_path} push\" /sc minute /mo {interval}"
    );
    println!("Run this command to create the task:\n{command}");
    println!("\nTo remove the task, run: schtasks /delete /tn \"ctxsync\" /f");
}

// --- cron (other Unix) ---

fn read_crontab() -> String {
    Command::new("crontab")
        .arg("-l")
        .output()
        .map(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout).to_string()
            } else {
                String::new()
            }
        })
        .unwrap_or_default()
}

fn write_crontab(content: &str) -> Result<()> {
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
        .write_all(content.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        return Err(CsError::Other("Failed to install crontab entry".into()));
    }
    Ok(())
}

fn setup_unix_cron(ctxsync_path: &str, interval: usize, workdir: &std::path::Path) -> Result<()> {
    let new_entry = format!(
        "*/{interval} * * * * cd '{}' && {ctxsync_path} push {CRON_MARKER}",
        workdir.display()
    );
    let mut crontab = read_crontab();
    if !crontab.is_empty() && !crontab.ends_with('\n') {
        crontab.push('\n');
    }
    crontab.push_str(&new_entry);
    crontab.push('\n');
    write_crontab(&crontab)?;

    println!("Cron job created successfully! It will run every {interval} minutes.");
    println!("\nTo remove it, run: ctxsync schedule --remove");
    Ok(())
}
