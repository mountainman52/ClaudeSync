use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::cli::{confirm, prompt_string, prompt_usize};
use crate::config::FileConfig;
use crate::error::{CsError, Result};
use crate::provider::{get_provider, ClaudeProvider, Project};
use crate::sync::retry_on_403;
use crate::utils::validate_and_get_provider;

/// Initialize a new project configuration; with `new` also creates the remote
/// project on Claude.ai (port of `project init` / `project create`).
pub fn init(
    config: &mut FileConfig,
    name: Option<String>,
    description: Option<String>,
    local_path: Option<String>,
    new: bool,
    provider_name: &str,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let default_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let name = match name {
        Some(n) => n,
        None => prompt_string("Enter a title for your project", Some(&default_name))?,
    };
    let description = match description {
        Some(d) => d,
        None => prompt_string(
            "Enter the project description",
            Some("Project created with ClaudeSync"),
        )?,
    };
    let local_path = match local_path {
        Some(p) => p,
        None => prompt_string(
            "Enter the absolute path to your local project directory",
            Some(&cwd.to_string_lossy()),
        )?,
    };

    let local_path_buf = Path::new(&local_path);
    if !local_path_buf.is_dir() {
        return Err(CsError::Configuration(format!(
            "Directory does not exist: {local_path}"
        )));
    }

    let claudesync_dir = local_path_buf.join(".claudesync");
    fs::create_dir_all(&claudesync_dir)?;

    config.set(
        "active_provider",
        Value::String(provider_name.to_string()),
        true,
    )?;
    config.set("local_path", Value::String(local_path.clone()), true)?;

    if new {
        let provider = get_provider(config, provider_name)?;
        let organizations = provider.get_organizations()?;
        let organization = organizations.first().ok_or_else(|| {
            CsError::Configuration("No organizations with required capabilities found.".into())
        })?;

        let new_project = provider.create_project(&organization.id, &name, &description)?;
        let project_uuid = new_project["uuid"].as_str().unwrap_or_default().to_string();
        let project_name = new_project["name"].as_str().unwrap_or_default().to_string();
        println!("Project '{project_name}' (uuid: {project_uuid}) has been created successfully.");

        config.set(
            "active_organization_id",
            Value::String(organization.id.clone()),
            true,
        )?;
        config.set(
            "active_project_id",
            Value::String(project_uuid.clone()),
            true,
        )?;
        config.set("active_project_name", Value::String(project_name), true)?;

        println!("\nProject created:");
        println!("  - Project location: {local_path}");
        println!(
            "  - Project config location: {}",
            claudesync_dir.join("config.local.json").display()
        );
        println!("  - Remote URL: https://claude.ai/project/{project_uuid}");
    } else {
        config.save_local_config()?;
        println!("\nLocal project configuration created:");
        println!("  - Project location: {local_path}");
        println!(
            "  - Project config location: {}",
            claudesync_dir.join("config.local.json").display()
        );
        println!("\nTo link to a remote project:");
        println!("1. Run 'claudesync organization set' to select an organization");
        println!("2. Run 'claudesync project set' to select an existing project");
    }
    Ok(())
}

pub fn archive(config: &FileConfig, archive_all: bool, yes: bool) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let projects = provider.get_projects(&org_id, false)?;

    if projects.is_empty() {
        println!("No active projects found.");
        return Ok(());
    }

    if archive_all {
        if !yes {
            println!("The following projects will be archived:");
            for project in &projects {
                println!("  - {} (ID: {})", project.name, project.id);
            }
            if !confirm("Are you sure you want to archive all projects?")? {
                println!("Operation cancelled.");
                return Ok(());
            }
        }
        let pb = indicatif::ProgressBar::new(projects.len() as u64);
        pb.set_message("Archiving projects");
        for project in &projects {
            if let Err(e) = provider.archive_project(&org_id, &project.id) {
                println!("\nFailed to archive project '{}': {e}", project.name);
            }
            pb.inc(1);
        }
        pb.finish();
        println!("\nArchive operation completed.");
        return Ok(());
    }

    // Single project archival
    println!("Available projects to archive:");
    for (idx, project) in projects.iter().enumerate() {
        println!("  {}. {} (ID: {})", idx + 1, project.name, project.id);
    }
    let selection = prompt_usize("Enter the number of the project to archive", None)?;
    if selection >= 1 && selection <= projects.len() {
        let selected = &projects[selection - 1];
        if yes
            || confirm(&format!(
                "Are you sure you want to archive the project '{}'? Archived projects cannot be modified but can still be viewed.",
                selected.name
            ))?
        {
            provider.archive_project(&org_id, &selected.id)?;
            println!("Project '{}' has been archived.", selected.name);
        }
    } else {
        println!("Invalid selection. Please try again.");
    }
    Ok(())
}

fn save_project_selection(config: &mut FileConfig, project: &Project) -> Result<()> {
    config.set(
        "active_project_id",
        Value::String(project.id.clone()),
        true,
    )?;
    config.set(
        "active_project_name",
        Value::String(project.name.clone()),
        true,
    )?;
    println!("Selected project: {} (ID: {})", project.name, project.id);

    fs::create_dir_all(".claudesync")?;
    let claudesync_dir = fs::canonicalize(".claudesync")?;
    config.save_local_config()?;

    println!("\nProject created:");
    println!(
        "  - Project location: {}",
        std::env::current_dir()?.display()
    );
    println!(
        "  - Project config location: {}",
        claudesync_dir.join("config.local.json").display()
    );
    Ok(())
}

pub fn set(
    config: &mut FileConfig,
    show_all: bool,
    project_id: Option<String>,
    provider_name: &str,
) -> Result<()> {
    config.set(
        "active_provider",
        Value::String(provider_name.to_string()),
        true,
    )?;

    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let active_project_name = config.get_str("active_project_name").unwrap_or_default();
    let projects = provider.get_projects(&org_id, false)?;

    let selectable: Vec<Project> = if show_all {
        projects
    } else {
        projects
            .into_iter()
            .filter(|p| !p.name.contains("-SubModule-"))
            .collect()
    };

    if selectable.is_empty() {
        println!("No active projects found.");
        return Ok(());
    }

    if let Some(project_id) = project_id {
        match selectable.iter().find(|p| p.id == project_id) {
            Some(project) => {
                let project = project.clone();
                save_project_selection(config, &project)?;
            }
            None => {
                println!("Project with ID {project_id} not found in available projects.");
                if !show_all {
                    println!("Tip: Use --all flag to include submodule projects.");
                }
            }
        }
        return Ok(());
    }

    println!("Available projects:");
    for (idx, project) in selectable.iter().enumerate() {
        let project_type = if project
            .name
            .starts_with(&format!("{active_project_name}-SubModule-"))
        {
            "Submodule"
        } else {
            "Main Project"
        };
        println!(
            "  {}. {} (ID: {}) - {}",
            idx + 1,
            project.name,
            project.id,
            project_type
        );
    }
    let selection = prompt_usize("Enter the number of the project to select", Some(1))?;
    if selection >= 1 && selection <= selectable.len() {
        let project = selectable[selection - 1].clone();
        save_project_selection(config, &project)?;
    } else {
        println!("Invalid selection. Please try again.");
    }
    Ok(())
}

pub fn ls(config: &FileConfig, show_all: bool) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let projects = provider.get_projects(&org_id, show_all)?;
    if projects.is_empty() {
        println!("No projects found.");
    } else {
        println!("Remote projects:");
        for project in projects {
            let status = if project.archived_at.is_some() {
                " (Archived)"
            } else {
                ""
            };
            println!("  - {} (ID: {}){}", project.name, project.id, status);
        }
    }
    Ok(())
}

pub fn truncate(
    config: &FileConfig,
    include_archived: bool,
    truncate_all: bool,
    yes: bool,
) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let projects = provider.get_projects(&org_id, include_archived)?;

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    if truncate_all {
        if !yes {
            println!("This will delete ALL files from the following projects:");
            for project in &projects {
                let status = if project.archived_at.is_some() {
                    " (Archived)"
                } else {
                    ""
                };
                println!("  - {} (ID: {}){}", project.name, project.id, status);
            }
            if !confirm("Are you sure you want to continue? This may take some time.")? {
                println!("Operation cancelled.");
                return Ok(());
            }
        }
        let pb = indicatif::ProgressBar::new(projects.len() as u64);
        pb.set_message("Deleting files from projects");
        for project in &projects {
            delete_files_from_project(&provider, &org_id, &project.id, &project.name)?;
            pb.inc(1);
        }
        pb.finish();
        println!("All files have been deleted from all projects.");
        return Ok(());
    }

    println!("Available projects:");
    for (idx, project) in projects.iter().enumerate() {
        let status = if project.archived_at.is_some() {
            " (Archived)"
        } else {
            ""
        };
        println!(
            "  {}. {} (ID: {}){}",
            idx + 1,
            project.name,
            project.id,
            status
        );
    }
    let selection = prompt_usize("Enter the number of the project to truncate", None)?;
    if selection >= 1 && selection <= projects.len() {
        let selected = &projects[selection - 1];
        if yes
            || confirm(&format!(
                "Are you sure you want to delete ALL files from project '{}'?",
                selected.name
            ))?
        {
            delete_files_from_project(&provider, &org_id, &selected.id, &selected.name)?;
            println!(
                "All files have been deleted from project '{}'.",
                selected.name
            );
        }
    } else {
        println!("Invalid selection. Please try again.");
    }
    Ok(())
}

fn delete_files_from_project(
    provider: &ClaudeProvider,
    organization_id: &str,
    project_id: &str,
    project_name: &str,
) -> Result<()> {
    let result = retry_on_403(|| {
        let files = provider.list_files(organization_id, project_id)?;
        for file in &files {
            provider.delete_file(organization_id, project_id, &file.uuid)?;
        }
        Ok(())
    });
    if let Err(CsError::Provider(e)) = &result {
        println!("Error deleting files from project {project_name}: {e}");
        return Ok(());
    }
    result
}

/// `project file ls` — list files in the active remote project.
pub fn file_ls(config: &FileConfig) -> Result<()> {
    let provider = validate_and_get_provider(config, true, true)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let project_id = config.get_str("active_project_id").unwrap_or_default();
    let files = provider.list_files(&org_id, &project_id)?;
    if files.is_empty() {
        println!("No files found in the active project.");
    } else {
        println!(
            "Files in project '{}' (ID: {}):",
            config.get_str("active_project_name").unwrap_or_default(),
            project_id
        );
        for file in files {
            println!(
                "  - {} (ID: {}, Created: {})",
                file.file_name, file.uuid, file.created_at
            );
        }
    }
    Ok(())
}
