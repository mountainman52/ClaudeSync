use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::config::FileConfig;
use crate::error::Result;
use crate::utils::{detect_submodules, validate_and_get_provider};

fn submodule_detect_filenames(config: &FileConfig) -> Vec<String> {
    config
        .get("submodule_detect_filenames")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(|s| s.to_string())
        .collect()
}

pub fn ls(config: &FileConfig) -> Result<()> {
    let local_path = match config.get_local_path() {
        Some(p) => p,
        None => {
            println!(
                "No local project path found. Please select an existing project or create a new one using \
                 'ctxsync project select' or 'ctxsync project create'."
            );
            return Ok(());
        }
    };

    let submodules = detect_submodules(&local_path, &submodule_detect_filenames(config));
    if submodules.is_empty() {
        println!("No submodules detected in the current project.");
    } else {
        println!("Detected submodules:");
        for (submodule, detected_file) in submodules {
            println!("  - {submodule} [{detected_file}]");
        }
    }
    Ok(())
}

/// Creates new remote projects for each detected submodule that doesn't
/// already exist, and records them in the local config.
pub fn create(config: &FileConfig) -> Result<()> {
    let provider = validate_and_get_provider(config, true, true)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let active_project_id = config.get_str("active_project_id").unwrap_or_default();
    let active_project_name = config.get_str("active_project_name").unwrap_or_default();

    let local_path = match config.get_local_path() {
        Some(p) => p,
        None => {
            println!(
                "No local project path found. Please select an existing project or create a new one using \
                 'ctxsync project select' or 'ctxsync project create'."
            );
            return Ok(());
        }
    };

    let submodules_with_files =
        detect_submodules(&local_path, &submodule_detect_filenames(config));
    if submodules_with_files.is_empty() {
        println!("No submodules detected in the current project.");
        return Ok(());
    }

    let all_remote_projects = provider.get_projects(&org_id, false)?;
    println!(
        "Detected {} submodule(s). Checking for existing remote projects:",
        submodules_with_files.len()
    );

    let local_config_path = local_path
        .join(config.local_dir_name)
        .join("config.local.json");
    let mut local_config: Value =
        serde_json::from_str(&fs::read_to_string(&local_config_path)?)?;
    if local_config.get("submodules").is_none() {
        local_config["submodules"] = json!([]);
    }

    for (i, (submodule, _detected_file)) in submodules_with_files.iter().enumerate() {
        let submodule_name = Path::new(submodule)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| submodule.clone());
        let new_project_name = format!("{active_project_name}-SubModule-{submodule_name}");

        let project_id = match all_remote_projects
            .iter()
            .find(|p| p.name == new_project_name)
        {
            Some(existing) => {
                println!(
                    "{}. Submodule '{submodule_name}' already exists as project '{new_project_name}' (ID: {}). Updating local config.",
                    i + 1,
                    existing.id
                );
                existing.id.clone()
            }
            None => {
                let description = format!(
                    "Submodule '{submodule_name}' for project '{active_project_name}' (ID: {active_project_id})"
                );
                match provider.create_project(&org_id, &new_project_name, &description) {
                    Ok(new_project) => {
                        let id = new_project["uuid"].as_str().unwrap_or_default().to_string();
                        println!(
                            "{}. Created project '{new_project_name}' (ID: {id}) for submodule '{submodule_name}'",
                            i + 1
                        );
                        id
                    }
                    Err(e) => {
                        println!(
                            "Failed to create project for submodule '{submodule_name}': {e}"
                        );
                        continue;
                    }
                }
            }
        };

        let submodule_config = json!({
            "active_provider": config.get_str("active_provider"),
            "active_organization_id": org_id,
            "active_project_id": project_id,
            "active_project_name": new_project_name,
            "relative_path": submodule,
        });

        if let Some(submodules) = local_config["submodules"].as_array_mut() {
            let existing_index = submodules.iter().position(|d| {
                d.get("relative_path").and_then(Value::as_str) == Some(submodule.as_str())
            });
            match existing_index {
                Some(idx) => submodules[idx] = submodule_config,
                None => submodules.push(submodule_config),
            }
        }
    }

    fs::write(
        &local_config_path,
        serde_json::to_string_pretty(&local_config)?,
    )?;
    println!("\nSubmodule project creation and configuration update completed.");
    Ok(())
}
