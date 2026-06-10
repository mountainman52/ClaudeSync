use serde_json::Value;

use crate::config::FileConfig;
use crate::error::Result;
use crate::provider::ClaudeProvider;
use crate::sync::SyncManager;
use crate::utils::{get_local_files, validate_and_get_provider};

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
        for file in local_files.keys() {
            println!("Would send file: {file}");
        }
        println!("Not sending files due to dry run mode.");
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
