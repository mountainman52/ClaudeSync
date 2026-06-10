use serde_json::Value;

use crate::chat_sync::sync_chats;
use crate::cli::{confirm, prompt_string, prompt_usize};
use crate::config::FileConfig;
use crate::error::Result;
use crate::provider::ClaudeProvider;
use crate::utils::validate_and_get_provider;

/// Synchronize chats and their artifacts from the remote source.
pub fn pull(config: &FileConfig) -> Result<()> {
    let provider = validate_and_get_provider(config, true, true)?;
    sync_chats(&provider, config, false)
}

pub fn ls(config: &FileConfig) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();
    let chats = provider.get_chat_conversations(&org_id)?;

    for chat in chats.as_array().cloned().unwrap_or_default() {
        let project_name = chat
            .get("project")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        println!(
            "UUID: {}, Name: {}, Project: {}, Updated: {}",
            chat.get("uuid").and_then(Value::as_str).unwrap_or("Unknown"),
            chat.get("name").and_then(Value::as_str).unwrap_or("Unnamed"),
            project_name,
            chat.get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or("Unknown"),
        );
    }
    Ok(())
}

pub fn rm(config: &FileConfig, delete_all: bool) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let org_id = config.get_str("active_organization_id").unwrap_or_default();

    if delete_all {
        delete_all_chats(&provider, &org_id)
    } else {
        delete_single_chat(&provider, &org_id)
    }
}

fn delete_chats(
    provider: &ClaudeProvider,
    organization_id: &str,
    uuids: &[String],
) -> Result<(usize, usize)> {
    match provider.delete_chat(organization_id, uuids) {
        Ok(result) => Ok((result.as_array().map(|a| a.len()).unwrap_or(0), 0)),
        Err(e) => {
            log::error!("Error deleting chats: {e}");
            println!("Error occurred while deleting chats: {e}");
            Ok((0, uuids.len()))
        }
    }
}

fn delete_all_chats(provider: &ClaudeProvider, organization_id: &str) -> Result<()> {
    if confirm("Are you sure you want to delete all chats?")? {
        let mut total_deleted = 0;
        loop {
            let chats = provider.get_chat_conversations(organization_id)?;
            let chats = chats.as_array().cloned().unwrap_or_default();
            if chats.is_empty() {
                break;
            }
            let uuids: Vec<String> = chats
                .iter()
                .take(50)
                .filter_map(|c| c.get("uuid").and_then(Value::as_str))
                .map(|s| s.to_string())
                .collect();
            let (deleted, _) = delete_chats(provider, organization_id, &uuids)?;
            total_deleted += deleted;
        }
        println!("Chat deletion complete. Total chats deleted: {total_deleted}");
    }
    Ok(())
}

fn delete_single_chat(provider: &ClaudeProvider, organization_id: &str) -> Result<()> {
    let chats = provider.get_chat_conversations(organization_id)?;
    let chats = chats.as_array().cloned().unwrap_or_default();
    if chats.is_empty() {
        println!("No chats found.");
        return Ok(());
    }

    println!("Available chats:");
    for (idx, chat) in chats.iter().enumerate() {
        let project_name = chat
            .get("project")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        println!(
            "{}. Name: {}, Project: {}, Updated: {}",
            idx + 1,
            chat.get("name").and_then(Value::as_str).unwrap_or("Unnamed"),
            project_name,
            chat.get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or("Unknown"),
        );
    }

    let selected = loop {
        let selection =
            prompt_string("Enter the number of the chat to delete (or 'q' to quit)", None)?;
        if selection.to_lowercase() == "q" {
            return Ok(());
        }
        match selection.parse::<usize>() {
            Ok(n) if n >= 1 && n <= chats.len() => break &chats[n - 1],
            Ok(_) => println!("Invalid selection. Please try again."),
            Err(_) => println!("Invalid input. Please enter a number or 'q' to quit."),
        }
    };

    let name = selected
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Unnamed");
    if confirm(&format!(
        "Are you sure you want to delete the chat '{name}'?"
    ))? {
        let uuid = selected
            .get("uuid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let (deleted, _) = delete_chats(provider, organization_id, &[uuid])?;
        if deleted > 0 {
            println!("Successfully deleted chat: {name}");
        } else {
            println!("Failed to delete chat: {name}");
        }
    }
    Ok(())
}

/// Initializes a new chat conversation on the active provider.
pub fn init(config: &FileConfig, name: &str, project: Option<String>) -> Result<()> {
    let provider = validate_and_get_provider(config, true, false)?;
    let organization_id = config.get_str("active_organization_id").unwrap_or_default();
    let active_project_id = config.get_str("active_project_id");
    let active_project_name = config.get_str("active_project_name");
    let local_path = config.get_str("local_path");

    if organization_id.is_empty() {
        println!("No active organization set.");
        return Ok(());
    }

    let project = match project {
        Some(p) => p,
        None => match select_project(
            config,
            active_project_id.as_deref(),
            active_project_name.as_deref(),
            local_path.as_deref(),
            &organization_id,
            &provider,
        )? {
            Some(p) => p,
            None => return Ok(()),
        },
    };

    match provider.create_chat(&organization_id, name, Some(&project), None) {
        Ok(new_chat) => {
            println!(
                "Created new chat conversation: {}",
                new_chat["uuid"].as_str().unwrap_or_default()
            );
            if !name.is_empty() {
                println!("Chat name: {name}");
            }
            println!("Associated project: {project}");
        }
        Err(e) => println!("Failed to create chat conversation: {e}"),
    }
    Ok(())
}

/// Processes a single streaming event from the message response.
fn process_message_event(event: &Value) {
    use std::io::Write;
    // New API format: content_block_delta with text_delta
    if event.get("type").and_then(Value::as_str) == Some("content_block_delta") {
        if let Some(delta) = event.get("delta") {
            if delta.get("type").and_then(Value::as_str) == Some("text_delta") {
                print!("{}", delta.get("text").and_then(Value::as_str).unwrap_or(""));
                let _ = std::io::stdout().flush();
            }
        }
    } else if let Some(completion) = event.get("completion").and_then(Value::as_str) {
        print!("{completion}");
        let _ = std::io::stdout().flush();
    } else if let Some(content) = event.get("content").and_then(Value::as_str) {
        print!("{content}");
        let _ = std::io::stdout().flush();
    } else if let Some(error) = event.get("error") {
        println!("\nError: {error}");
    } else if let Some(limit) = event.get("message_limit") {
        println!(
            "\nRemaining messages: {}",
            limit.get("remaining").cloned().unwrap_or(Value::Null)
        );
    }
}

/// Send a message to a chat (creating one first if necessary).
pub fn message(
    config: &FileConfig,
    message_parts: &[String],
    chat: Option<String>,
    timezone: &str,
    model: Option<String>,
) -> Result<()> {
    let provider = validate_and_get_provider(config, true, true)?;
    let organization_id = config.get_str("active_organization_id").unwrap_or_default();
    let active_project_id = config.get_str("active_project_id");
    let active_project_name = config.get_str("active_project_name");

    let message = message_parts.join(" ");

    let chat_id = match chat {
        Some(c) => c,
        None => {
            let mut project_id = active_project_id.clone();
            if active_project_name.is_none() {
                project_id = select_project(
                    config,
                    active_project_id.as_deref(),
                    active_project_name.as_deref(),
                    config.get_str("local_path").as_deref(),
                    &organization_id,
                    &provider,
                )?;
            }
            let project_id = match project_id {
                Some(p) => p,
                None => return Ok(()),
            };
            let new_chat = provider.create_chat(
                &organization_id,
                "",
                Some(&project_id),
                model.as_deref(),
            )?;
            let chat_id = new_chat["uuid"].as_str().unwrap_or_default().to_string();
            println!("New chat created with ID: {chat_id}");
            chat_id
        }
    };

    let result = provider.send_message(
        &organization_id,
        &chat_id,
        &message,
        timezone,
        model.as_deref(),
        process_message_event,
    );
    if let Err(e) = result {
        println!("Failed to send message: {e}");
        return Ok(());
    }
    println!(); // newline at the end of the response
    Ok(())
}

/// Lets the user pick the active project or one of its submodule projects.
fn select_project(
    _config: &FileConfig,
    active_project_id: Option<&str>,
    active_project_name: Option<&str>,
    local_path: Option<&str>,
    organization_id: &str,
    provider: &ClaudeProvider,
) -> Result<Option<String>> {
    let all_projects = provider.get_projects(organization_id, false)?;
    if all_projects.is_empty() {
        println!("No projects found in the active organization.");
        return Ok(None);
    }

    let submodule_prefix = format!("{}-SubModule-", active_project_name.unwrap_or(""));
    let filtered: Vec<_> = all_projects
        .into_iter()
        .filter(|p| {
            Some(p.id.as_str()) == active_project_id
                || (p.name.starts_with(&submodule_prefix) && p.archived_at.is_none())
        })
        .collect();

    if filtered.is_empty() {
        println!("No active project or related submodules found.");
        return Ok(None);
    }

    // Default to the project matching the current working directory
    let current_dir = std::env::current_dir()?;
    let default_project: Option<usize> = local_path.and_then(|lp| {
        filtered.iter().position(|p| {
            let project_path = if Some(p.id.as_str()) == active_project_id {
                std::path::PathBuf::from(lp)
            } else {
                let submodule_name = p.name.replace(&submodule_prefix, "");
                std::path::Path::new(lp).join("services").join(submodule_name)
            };
            current_dir.starts_with(&project_path)
        })
    });

    println!("Available projects:");
    for (idx, p) in filtered.iter().enumerate() {
        let project_type = if Some(p.id.as_str()) == active_project_id {
            "Active Project"
        } else {
            "Submodule"
        };
        let default_marker = if Some(idx) == default_project {
            " (default)"
        } else {
            ""
        };
        println!(
            "{}. {} (ID: {}) - {}{}",
            idx + 1,
            p.name,
            p.id,
            project_type,
            default_marker
        );
    }

    loop {
        let mut prompt = "Enter the number of the project to associate with the chat".to_string();
        if let Some(d) = default_project {
            prompt += &format!(" (default: {} - {})", d + 1, filtered[d].name);
        }
        let selection = prompt_usize(&prompt, default_project.map(|d| d + 1))?;
        if selection >= 1 && selection <= filtered.len() {
            return Ok(Some(filtered[selection - 1].id.clone()));
        }
        println!("Invalid selection. Please try again.");
    }
}
