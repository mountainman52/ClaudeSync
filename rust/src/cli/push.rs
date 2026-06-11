use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::config::FileConfig;
use crate::error::Result;
use crate::provider::{ClaudeProvider, RemoteFile};
use crate::sync::SyncManager;
use crate::utils::{compute_md5_hash, get_local_files, validate_and_get_provider};

fn resolve_category(config: &FileConfig, category: Option<String>) -> Option<String> {
    match category {
        Some(c) => Some(c),
        None => {
            let default = config.get_default_category();
            if let Some(ref c) = default {
                println!("Using default category: {c}");
            }
            default
        }
    }
}

/// Synchronize the project files, optionally including submodules
/// (port of the `push` command).
pub fn push(
    config: &FileConfig,
    category: Option<String>,
    uberproject: bool,
    dryrun: bool,
) -> Result<()> {
    let provider = validate_and_get_provider(config, true, true)?;
    let category = resolve_category(config, category);

    let active_organization_id = config.get_str("active_organization_id").unwrap_or_default();
    let active_project_id = config.get_str("active_project_id").unwrap_or_default();
    let active_project_name = config.get_str("active_project_name").unwrap_or_default();

    let local_path = match config.get_local_path() {
        Some(p) => p,
        None => {
            println!(
                "No .claudesync directory found in this directory or any parent directories. \
                 Please run 'claudesync project create' or 'claudesync project set' first."
            );
            return Ok(());
        }
    };

    let submodules: Vec<Value> = config
        .get("submodules")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    // Detect if we're inside one of the configured submodules
    let current_dir = std::env::current_dir()?;
    let current_submodule = submodules.iter().find(|sm| {
        sm.get("relative_path")
            .and_then(Value::as_str)
            .map(|rel| local_path.join(rel) == current_dir)
            .unwrap_or(false)
    });

    if let Some(submodule) = current_submodule {
        let name = submodule
            .get("active_project_name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        println!("Syncing submodule {name} [{}]", current_dir.display());
        sync_submodule(&provider, config, submodule, category.as_deref())?;
        return Ok(());
    }

    // Sync main project
    let sync_manager = SyncManager::new(config, &local_path);
    let remote_files = provider.list_files(&active_organization_id, &active_project_id)?;
    let local_files = get_local_files(config, &local_path, category.as_deref(), uberproject)?;

    if dryrun {
        print_dryrun_diff(config, &local_files, &remote_files);
        return Ok(());
    }

    sync_manager.sync(&provider, &local_files, &remote_files)?;
    println!(
        "Main project '{active_project_name}' synced successfully: https://claude.ai/project/{active_project_id}"
    );

    // Always sync submodules to their respective projects
    for submodule in &submodules {
        sync_submodule(&provider, config, submodule, category.as_deref())?;
    }
    Ok(())
}

/// Reports what `push` would do against the remote: new uploads, content
/// updates, prunes, and the unchanged count.
fn print_dryrun_diff(
    config: &FileConfig,
    local_files: &BTreeMap<String, String>,
    remote_files: &[RemoteFile],
) {
    let mut remote_by_name: HashMap<&str, &RemoteFile> = HashMap::new();
    for rf in remote_files {
        remote_by_name.entry(rf.file_name.as_str()).or_insert(rf);
    }

    let mut new_files = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = 0usize;
    for (file, local_hash) in local_files {
        match remote_by_name.get(file.as_str()) {
            None => new_files.push(file.as_str()),
            Some(rf) if compute_md5_hash(&rf.content) != *local_hash => {
                changed.push(file.as_str())
            }
            Some(_) => unchanged += 1,
        }
    }
    let mut to_delete: Vec<&str> = remote_files
        .iter()
        .filter(|rf| !local_files.contains_key(&rf.file_name))
        .map(|rf| rf.file_name.as_str())
        .collect();
    to_delete.sort();
    to_delete.dedup();

    if !new_files.is_empty() {
        println!("Would upload ({} new):", new_files.len());
        for f in &new_files {
            println!("  + {f}");
        }
    }
    if !changed.is_empty() {
        println!("Would update ({} changed):", changed.len());
        for f in &changed {
            println!("  ~ {f}");
        }
    }
    if config.get_bool("prune_remote_files", false) && !to_delete.is_empty() {
        println!("Would delete remotely ({}):", to_delete.len());
        for f in &to_delete {
            println!("  - {f}");
        }
    }
    println!("Unchanged: {unchanged}");
    println!("Dry run: no changes were sent.");
}

/// Polls the project for local changes and pushes whenever they appear — a
/// foreground alternative to cron/launchd scheduling.
pub fn watch(
    config: &FileConfig,
    category: Option<String>,
    uberproject: bool,
    interval: u64,
) -> Result<()> {
    let local_path = match config.get_local_path() {
        Some(p) => p,
        None => {
            println!(
                "No .claudesync directory found in this directory or any parent directories. \
                 Please run 'claudesync project create' or 'claudesync project set' first."
            );
            return Ok(());
        }
    };
    let category = resolve_category(config, category);
    println!(
        "Watching {} for changes every {interval}s (Ctrl+C to stop)...",
        local_path.display()
    );

    let mut last: Option<BTreeMap<String, String>> = None;
    loop {
        let snapshot = get_local_files(config, &local_path, category.as_deref(), uberproject)?;
        if last.as_ref() != Some(&snapshot) {
            if last.is_some() {
                println!("[watch] Change detected; pushing...");
            } else {
                println!("[watch] Initial push...");
            }
            match push(config, category.clone(), uberproject, false) {
                Ok(()) => last = Some(snapshot),
                // Keep watching through transient provider errors (rate
                // limits, network blips); they'll be retried next cycle.
                Err(e) if e.is_handled() => println!("Error: {e}"),
                Err(e) => return Err(e),
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}

fn sync_submodule(
    provider: &ClaudeProvider,
    config: &FileConfig,
    submodule: &Value,
    category: Option<&str>,
) -> Result<()> {
    let relative_path = submodule
        .get("relative_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let sm_org_id = submodule
        .get("active_organization_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let sm_project_id = submodule
        .get("active_project_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let sm_project_name = submodule
        .get("active_project_name")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let submodule_path = config
        .get_local_path()
        .unwrap_or_default()
        .join(relative_path);
    let submodule_files = get_local_files(config, &submodule_path, category, false)?;
    let remote_submodule_files = provider.list_files(sm_org_id, sm_project_id)?;

    let sync_manager = SyncManager::new(config, &submodule_path).with_project(sm_project_id);
    sync_manager.sync(provider, &submodule_files, &remote_submodule_files)?;
    println!(
        "Submodule '{sm_project_name}' synced successfully: https://claude.ai/project/{sm_project_id}"
    );
    Ok(())
}

/// Generate a (compressed) text embedding of the project without uploading.
pub fn embedding(config: &FileConfig, category: Option<String>, uberproject: bool) -> Result<()> {
    let category = resolve_category(config, category);

    let local_path = match config.get_local_path() {
        Some(p) => p,
        None => {
            println!(
                "No .claudesync directory found in this directory or any parent directories. \
                 Please run 'claudesync project create' or 'claudesync project set' first."
            );
            return Ok(());
        }
    };

    let sync_manager = SyncManager::new(config, &local_path);
    let local_files = get_local_files(config, &local_path, category.as_deref(), uberproject)?;
    let output = sync_manager.embedding(&local_files)?;
    println!("{output}");
    Ok(())
}
