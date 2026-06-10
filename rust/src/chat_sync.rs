use std::fs;
use std::path::Path;

use regex::Regex;
use serde_json::Value;

use crate::config::FileConfig;
use crate::error::{CsError, Result};
use crate::provider::ClaudeProvider;
use crate::sync::retry_on_403;

/// Synchronize chats and their artifacts from the remote source
/// (port of `chat_sync.sync_chats`).
pub fn sync_chats(provider: &ClaudeProvider, config: &FileConfig, sync_all: bool) -> Result<()> {
    let local_path = config.get_str("local_path").ok_or_else(|| {
        CsError::Configuration(
            "Local path not set. Use 'claudesync project set' or 'claudesync project create' to set it."
                .into(),
        )
    })?;

    let chat_destination = Path::new(&local_path).join("claude_chats");
    fs::create_dir_all(&chat_destination)?;

    let organization_id = config.get_str("active_organization_id").ok_or_else(|| {
        CsError::Configuration("No active organization set. Please set an organization.".into())
    })?;

    let active_project_id = config.get_str("active_project_id");
    if active_project_id.is_none() && !sync_all {
        return Err(CsError::Configuration(
            "No active project set. Please set a project or use the -a flag to sync all chats."
                .into(),
        ));
    }

    log::debug!("Fetching chats for organization {organization_id}");
    let chats = provider.get_chat_conversations(&organization_id)?;
    let chats = chats.as_array().cloned().unwrap_or_default();
    log::debug!("Found {} chats", chats.len());

    let pb = indicatif::ProgressBar::new(chats.len() as u64);
    pb.set_message("Chats");
    for chat in &chats {
        sync_chat(
            active_project_id.as_deref(),
            chat,
            &chat_destination,
            &organization_id,
            provider,
            sync_all,
        )?;
        pb.inc(1);
    }
    pb.finish();

    log::debug!(
        "Chats and artifacts synchronized to {}",
        chat_destination.display()
    );
    Ok(())
}

fn sync_chat(
    active_project_id: Option<&str>,
    chat: &Value,
    chat_destination: &Path,
    organization_id: &str,
    provider: &ClaudeProvider,
    sync_all: bool,
) -> Result<()> {
    let chat_uuid = chat.get("uuid").and_then(Value::as_str).unwrap_or_default();
    let belongs_to_project = chat
        .get("project")
        .and_then(|p| p.get("uuid"))
        .and_then(Value::as_str)
        == active_project_id;

    if !(sync_all || belongs_to_project) {
        log::debug!("Skipping chat {chat_uuid} as it doesn't belong to the active project");
        return Ok(());
    }

    log::debug!("Processing chat {chat_uuid}");
    let chat_folder = chat_destination.join(chat_uuid);
    fs::create_dir_all(&chat_folder)?;

    let metadata_file = chat_folder.join("metadata.json");
    if !metadata_file.exists() {
        fs::write(&metadata_file, serde_json::to_string_pretty(chat)?)?;
    }

    log::debug!("Fetching full conversation for chat {chat_uuid}");
    let full_chat =
        retry_on_403(|| provider.get_chat_conversation(organization_id, chat_uuid))?;

    let messages = full_chat
        .get("chat_messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for message in &messages {
        let message_uuid = message
            .get("uuid")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message_file = chat_folder.join(format!("{message_uuid}.json"));
        if message_file.exists() {
            log::debug!("Skipping existing message {message_uuid}");
            continue;
        }
        fs::write(&message_file, serde_json::to_string_pretty(message)?)?;

        if message.get("sender").and_then(Value::as_str) == Some("assistant") {
            let text = message.get("text").and_then(Value::as_str).unwrap_or("");
            let artifacts = extract_artifacts(text);
            if !artifacts.is_empty() {
                log::info!(
                    "Found {} artifacts in message {message_uuid}",
                    artifacts.len()
                );
                save_artifacts(&artifacts, &chat_folder)?;
            }
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
pub struct Artifact {
    pub identifier: String,
    pub artifact_type: String,
    pub content: String,
}

fn save_artifacts(artifacts: &[Artifact], chat_folder: &Path) -> Result<()> {
    let artifact_folder = chat_folder.join("artifacts");
    fs::create_dir_all(&artifact_folder)?;
    for artifact in artifacts {
        let file = artifact_folder.join(format!(
            "{}.{}",
            artifact.identifier,
            get_file_extension(&artifact.artifact_type)
        ));
        if !file.exists() {
            fs::write(file, &artifact.content)?;
        }
    }
    Ok(())
}

/// Maps artifact MIME types to file extensions.
pub fn get_file_extension(artifact_type: &str) -> &'static str {
    match artifact_type {
        "text/html" => "html",
        "application/vnd.ant.code" => "txt",
        "image/svg+xml" => "svg",
        "application/vnd.ant.mermaid" => "mmd",
        "application/vnd.ant.react" => "jsx",
        _ => "txt",
    }
}

/// Extracts `<antArtifact>` blocks from assistant message text.
pub fn extract_artifacts(text: &str) -> Vec<Artifact> {
    let pattern = Regex::new(
        r#"(?s)<antArtifact\s+identifier="([^"]+)"\s+type="([^"]+)"\s+title="([^"]+)">(.*?)</antArtifact>"#,
    )
    .unwrap();

    pattern
        .captures_iter(text)
        .map(|cap| Artifact {
            identifier: cap[1].to_string(),
            artifact_type: cap[2].to_string(),
            content: cap[4].trim().to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_artifacts_from_text() {
        let text = r#"Here is some code:
<antArtifact identifier="my-script" type="application/vnd.ant.code" title="My Script">
print("hello")
</antArtifact>
And an svg:
<antArtifact identifier="pic" type="image/svg+xml" title="Pic"><svg></svg></antArtifact>"#;

        let artifacts = extract_artifacts(text);
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].identifier, "my-script");
        assert_eq!(artifacts[0].artifact_type, "application/vnd.ant.code");
        assert_eq!(artifacts[0].content, "print(\"hello\")");
        assert_eq!(artifacts[1].identifier, "pic");
        assert_eq!(get_file_extension(&artifacts[1].artifact_type), "svg");
    }

    #[test]
    fn no_artifacts_in_plain_text() {
        assert!(extract_artifacts("just a normal message").is_empty());
    }
}
