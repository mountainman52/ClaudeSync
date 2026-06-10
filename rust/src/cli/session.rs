use regex::Regex;
use serde_json::Value;

use crate::cli::{confirm, prompt_index, prompt_string, prompt_usize};
use crate::config::FileConfig;
use crate::error::Result;
use crate::utils::{parse_rfc3339_utc, validate_and_get_provider};

fn format_timestamp(value: Option<&str>) -> String {
    let raw = value.unwrap_or("N/A");
    match parse_rfc3339_utc(raw) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => raw.to_string(),
    }
}

/// Extracts (owner_login, repo_name) from a repos-list element as returned
/// by `get_code_repos`.
fn repo_owner_name(repo_data: &Value) -> Option<(String, String)> {
    let repo = repo_data.get("repo")?;
    let owner = repo.get("owner")?.get("login")?.as_str()?;
    let name = repo.get("name")?.as_str()?;
    Some((owner.to_string(), name.to_string()))
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `session ls` — list all web sessions.
pub fn ls(config: &FileConfig, show_all: bool, json_output: bool) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let sessions_data = provider.get_sessions(&org_id)?;

    let sessions = sessions_data
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    let sessions: Vec<Value> = if show_all {
        sessions
    } else {
        sessions
            .into_iter()
            .filter(|s| {
                matches!(
                    s.get("session_status").and_then(Value::as_str),
                    Some("running") | Some("idle")
                )
            })
            .collect()
    };

    if sessions.is_empty() {
        println!("No active sessions found. Use --all to show archived sessions.");
        return Ok(());
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }

    println!("Found {} session(s):", sessions.len());
    for (idx, sess) in sessions.iter().enumerate() {
        let session_id = sess.get("id").and_then(Value::as_str).unwrap_or("N/A");
        let title = sess.get("title").and_then(Value::as_str).unwrap_or("Untitled");
        let status = sess
            .get("session_status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        println!("\n{}. {}", idx + 1, title);
        println!("  ID: {session_id}");
        println!("  Status: {}", capitalize(status));
        println!(
            "  Created: {}",
            format_timestamp(sess.get("created_at").and_then(Value::as_str))
        );
        println!(
            "  Updated: {}",
            format_timestamp(sess.get("updated_at").and_then(Value::as_str))
        );

        // Repository info, if present in session outcomes
        if let Some(outcomes) = sess
            .get("session_context")
            .and_then(|c| c.get("outcomes"))
            .and_then(Value::as_array)
        {
            for outcome in outcomes {
                if outcome.get("type").and_then(Value::as_str) == Some("git_repository") {
                    let git_info = outcome.get("git_info").cloned().unwrap_or(Value::Null);
                    if let Some(repo) = git_info.get("repo").and_then(Value::as_str) {
                        println!("  Repository: {repo}");
                        if let Some(branch) = git_info
                            .get("branches")
                            .and_then(Value::as_array)
                            .and_then(|b| b.first())
                            .and_then(Value::as_str)
                        {
                            println!("  Branch: {branch}");
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// `session archive` — archive one or all active sessions.
pub fn archive(config: &FileConfig, archive_all: bool, yes: bool) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let sessions_data = provider.get_sessions(&org_id)?;

    let all_sessions = sessions_data
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if all_sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    let sessions: Vec<Value> = all_sessions
        .into_iter()
        .filter(|s| {
            matches!(
                s.get("session_status").and_then(Value::as_str),
                Some("running") | Some("idle")
            )
        })
        .collect();

    if sessions.is_empty() {
        println!("No active sessions found.");
        return Ok(());
    }

    if archive_all {
        if !yes {
            println!("The following sessions will be archived:");
            for sess in &sessions {
                println!(
                    "  - {} (ID: {})",
                    sess.get("title").and_then(Value::as_str).unwrap_or("Untitled"),
                    sess.get("id").and_then(Value::as_str).unwrap_or("N/A")
                );
            }
            if !confirm("Are you sure you want to archive all sessions?")? {
                println!("Operation cancelled.");
                return Ok(());
            }
        }

        let mut success_count = 0;
        let mut failure_count = 0;
        let pb = indicatif::ProgressBar::new(sessions.len() as u64);
        pb.set_message("Archiving sessions");
        for sess in &sessions {
            let session_id = sess.get("id").and_then(Value::as_str).unwrap_or_default();
            match provider.archive_session(&org_id, session_id) {
                Ok(_) => success_count += 1,
                Err(e) => {
                    failure_count += 1;
                    let title = sess.get("title").and_then(Value::as_str).unwrap_or("Untitled");
                    println!("\nFailed to archive session '{title}': {e}");
                }
            }
            pb.inc(1);
        }
        pb.finish();
        println!(
            "\nArchive operation completed. Successfully archived: {success_count}, Failed: {failure_count}"
        );
        return Ok(());
    }

    // Single session archival
    println!("Available sessions to archive:");
    for (idx, sess) in sessions.iter().enumerate() {
        println!(
            "  {}. {} (ID: {})",
            idx + 1,
            sess.get("title").and_then(Value::as_str).unwrap_or("Untitled"),
            sess.get("id").and_then(Value::as_str).unwrap_or("N/A")
        );
    }
    let Some(idx) = prompt_index("Enter the number of the session to archive", sessions.len(), None)?
    else {
        return Ok(());
    };
    let selected = &sessions[idx];
    let title = selected
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled");
    if yes
        || confirm(&format!(
            "Are you sure you want to archive the session '{title}'? Archived sessions cannot be modified but can still be viewed."
        ))?
    {
        let session_id = selected.get("id").and_then(Value::as_str).unwrap_or_default();
        match provider.archive_session(&org_id, session_id) {
            Ok(_) => println!("Session '{title}' has been archived."),
            Err(e) => println!("Failed to archive session '{title}': {e}"),
        }
    }
    Ok(())
}

/// `session environment ls` — list all Claude Code environments.
pub fn environment_ls(config: &FileConfig, json_output: bool) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();

    let environments_data = match provider.get_environments(&org_id) {
        Ok(d) => d,
        Err(e) => {
            println!("Failed to list environments: {e}");
            return Ok(());
        }
    };
    let environments = environments_data
        .get("environments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if environments.is_empty() {
        println!("No environments found.");
        return Ok(());
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&environments)?);
        return Ok(());
    }

    println!("Found {} environment(s):", environments.len());
    for (idx, env) in environments.iter().enumerate() {
        println!(
            "\n{}. {}",
            idx + 1,
            env.get("name").and_then(Value::as_str).unwrap_or("Unnamed Environment")
        );
        println!(
            "  ID: {}",
            env.get("environment_id").and_then(Value::as_str).unwrap_or("N/A")
        );
        println!(
            "  Kind: {}",
            env.get("kind").and_then(Value::as_str).unwrap_or("N/A")
        );
        println!(
            "  State: {}",
            env.get("state").and_then(Value::as_str).unwrap_or("unknown")
        );
    }
    Ok(())
}

/// `session branch ls` — list repositories available for Claude Code sessions.
pub fn branch_ls(config: &FileConfig, json_output: bool, search: Option<String>) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();

    let repos_data = match provider.get_code_repos(&org_id, true) {
        Ok(d) => d,
        Err(e) => {
            println!("Failed to list repositories: {e}");
            return Ok(());
        }
    };
    let mut repos = repos_data
        .get("repos")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if repos.is_empty() {
        println!("No repositories found.");
        return Ok(());
    }

    if let Some(search) = &search {
        let needle = search.to_lowercase();
        repos.retain(|r| {
            r.get("repo")
                .and_then(|repo| repo.get("name"))
                .and_then(Value::as_str)
                .map(|n| n.to_lowercase().contains(&needle))
                .unwrap_or(false)
        });
        if repos.is_empty() {
            println!("No repositories found matching '{search}'.");
            return Ok(());
        }
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&repos)?);
        return Ok(());
    }

    println!("Found {} repository(ies):", repos.len());
    for (idx, repo_data) in repos.iter().enumerate() {
        let (owner, name) = repo_owner_name(repo_data)
            .unwrap_or_else(|| ("Unknown".to_string(), "Unknown".to_string()));
        let default_branch = repo_data
            .get("repo")
            .and_then(|r| r.get("default_branch"))
            .and_then(Value::as_str)
            .unwrap_or("N/A");
        println!("\n{}. {owner}/{name}", idx + 1);
        println!("  Default branch: {default_branch}");
    }
    Ok(())
}

/// Detect a GitHub repository from the `origin` remote of the cwd.
fn detect_local_git_repo() -> Option<(String, String)> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let remote = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let ssh = Regex::new(r"^git@github\.com:([^/]+)/(.+?)(?:\.git)?$").unwrap();
    let https = Regex::new(r"^https://github\.com/([^/]+)/(.+?)(?:\.git)?$").unwrap();
    let captures = ssh.captures(&remote).or_else(|| https.captures(&remote))?;
    Some((captures[1].to_string(), captures[2].to_string()))
}

/// `session create` — create a new Claude Code web session and stream events.
pub fn create(
    config: &FileConfig,
    title: Option<String>,
    environment_id: Option<String>,
    model: &str,
    branch: Option<String>,
    json_output: bool,
) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();

    let title = match title {
        Some(t) => t,
        None => prompt_string("Enter the session title", None)?,
    };
    if title.trim().is_empty() {
        println!("Error: Title cannot be empty.");
        return Ok(());
    }

    // Resolve the environment: flag > config > interactive selection
    let environment_id = match environment_id.or_else(|| config.get_str("active_environment_id")) {
        Some(e) => e,
        None => {
            let environments_data = match provider.get_environments(&org_id) {
                Ok(d) => d,
                Err(e) => {
                    println!("Error: Could not retrieve environments: {e}");
                    println!("Please use -e flag to specify an environment ID.");
                    return Ok(());
                }
            };
            let environments = environments_data
                .get("environments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if environments.is_empty() {
                println!("Error: No environments found.");
                println!("Please create an environment first or use -e flag to specify one.");
                return Ok(());
            }

            println!("Available environments:");
            for (idx, env) in environments.iter().enumerate() {
                println!(
                    "  {}. {} ({}) - {}",
                    idx + 1,
                    env.get("name").and_then(Value::as_str).unwrap_or("Unnamed"),
                    env.get("state").and_then(Value::as_str).unwrap_or("unknown"),
                    env.get("environment_id").and_then(Value::as_str).unwrap_or("N/A"),
                );
            }
            let Some(env_idx) = prompt_index("Select an environment number", environments.len(), Some(1))?
            else {
                return Ok(());
            };
            let env = &environments[env_idx];
            if !json_output {
                println!(
                    "Using environment: {}",
                    env.get("name").and_then(Value::as_str).unwrap_or("Unnamed")
                );
            }
            env.get("environment_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }
    };

    // Verify a locally detected repository is connected to Claude Code
    let local_repo = detect_local_git_repo();
    let mut git_repo_owner: Option<String> = None;
    let mut git_repo_name: Option<String> = None;

    // One fetch serves both the local-repo verification and the fallback
    // interactive selection below.
    let repos: Vec<Value> = if local_repo.is_some() || !json_output {
        provider
            .get_code_repos(&org_id, true)
            .ok()
            .and_then(|d| d.get("repos").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    if let Some((local_owner, local_name)) = &local_repo {
        for repo_data in &repos {
            if let Some((owner, name)) = repo_owner_name(repo_data) {
                if &owner == local_owner && &name == local_name {
                    if !json_output {
                        println!("Using detected repository: {owner}/{name}");
                    }
                    git_repo_owner = Some(owner);
                    git_repo_name = Some(name);
                    break;
                }
            }
        }
        if git_repo_owner.is_none() && !json_output {
            println!(
                "\nDetected local repository {local_owner}/{local_name}, but it's not connected to Claude Code."
            );
            println!("You need to connect this repository via GitHub OAuth first.");
            println!("Available repositories:");
        }
    }

    // No connected repo found: let the user choose one (or skip)
    if git_repo_owner.is_none() && !json_output {
        if !repos.is_empty() {
            for (idx, repo_data) in repos.iter().enumerate() {
                let (owner, name) = repo_owner_name(repo_data)
                    .unwrap_or_else(|| ("Unknown".to_string(), "Unknown".to_string()));
                println!("  {}. {owner}/{name}", idx + 1);
            }
            println!(
                "  {}. Skip (create session without repository)",
                repos.len() + 1
            );
            let selection = prompt_usize("Select a repository number", Some(repos.len() + 1))?;
            if selection >= 1 && selection <= repos.len() {
                if let Some((owner, name)) = repo_owner_name(&repos[selection - 1]) {
                    println!("Using repository: {owner}/{name}");
                    git_repo_owner = Some(owner);
                    git_repo_name = Some(name);
                }
            } else if selection == repos.len() + 1 {
                println!("Creating session without git repository context");
            } else {
                println!("Invalid selection. Creating session without repository.");
            }
        } else {
            println!("Creating session without git repository context");
        }
    }

    let git_repo_url = match (&git_repo_owner, &git_repo_name) {
        (Some(o), Some(n)) => Some(format!("https://github.com/{o}/{n}")),
        _ => None,
    };

    let result = match provider.create_session(
        &org_id,
        &title,
        &environment_id,
        git_repo_url.as_deref(),
        git_repo_owner.as_deref(),
        git_repo_name.as_deref(),
        branch.as_deref(),
        model,
    ) {
        Ok(r) => r,
        Err(e) => {
            println!("Failed to create session: {e}");
            return Ok(());
        }
    };

    let session_id = result
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("N/A")
        .to_string();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!("Session created successfully!");
    println!("ID: {session_id}");
    println!(
        "Title: {}",
        result.get("title").and_then(Value::as_str).unwrap_or("N/A")
    );
    println!(
        "Status: {}",
        result
            .get("session_status")
            .and_then(Value::as_str)
            .unwrap_or("N/A")
    );

    // Extract branch name from outcomes
    if let Some(branch) = result
        .get("session_context")
        .and_then(|c| c.get("outcomes"))
        .and_then(Value::as_array)
        .and_then(|outcomes| {
            outcomes
                .iter()
                .find(|o| o.get("type").and_then(Value::as_str) == Some("git_repository"))
        })
        .and_then(|o| o.get("git_info"))
        .and_then(|g| g.get("branches"))
        .and_then(Value::as_array)
        .and_then(|b| b.first())
        .and_then(Value::as_str)
    {
        println!("Branch: {branch}");
    }

    println!("\nView session at: https://claude.ai/code/{session_id}");
    println!("\nNote: Session starts idle. Send a message through the web UI to begin.");
    println!("\n--- Streaming session events (Ctrl+C to stop) ---\n");
    println!("Connecting to event stream...");

    let mut event_count = 0usize;
    let stream_result = provider.stream_session_events(&org_id, &session_id, |event| {
        event_count += 1;
        println!(
            "[Event {event_count}] {}",
            serde_json::to_string(event).unwrap_or_default()
        );

        if let Some(error) = event.get("error") {
            println!("Error: {error}");
            return false;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(content) = event.get("content").and_then(Value::as_str) {
                    if !content.is_empty() {
                        println!("Claude: {content}");
                    }
                }
            }
            Some("session_status") => {
                if let Some(status) = event.get("status").and_then(Value::as_str) {
                    if !status.is_empty() {
                        println!("Status: {status}");
                    }
                }
            }
            _ => {}
        }
        true
    });

    match stream_result {
        Ok(()) if event_count == 0 => {
            println!("\nNo events received from session.");
            println!("The session may still be initializing.");
        }
        Ok(()) => {}
        Err(e) => {
            println!("\nError streaming events: {e}");
            println!(
                "Session {session_id} is still running. View at: https://claude.ai/code/{session_id}"
            );
        }
    }
    Ok(())
}
