use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc;
use std::time::Duration;

use chrono::{Local, TimeZone};
use regex::Regex;
use serde_json::{json, Value};

use crate::config::FileConfig;
use crate::error::{CsError, Result};

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:129.0) Gecko/20100101 Firefox/129.0";
pub const DEFAULT_SESSION_MODEL: &str = "claude-sonnet-4-5-20250929";

#[derive(Debug, Clone)]
pub struct Organization {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RemoteFile {
    pub uuid: String,
    pub file_name: String,
    pub content: String,
    pub created_at: String,
}

/// Port of `ClaudeAIProvider` (claude.ai HTTP API with sessionKey cookie auth).
pub struct ClaudeProvider {
    base_url: String,
    session_key: String,
    agent: ureq::Agent,
}

impl ClaudeProvider {
    pub fn new(base_url: String, session_key: String) -> Self {
        ClaudeProvider {
            base_url,
            session_key,
            agent: ureq::AgentBuilder::new().build(),
        }
    }

    /// Builds a provider from config, reading the stored session key.
    pub fn from_config(config: &FileConfig) -> Result<Self> {
        let base_url = config
            .get_str("claude_api_url")
            .unwrap_or_else(|| "https://claude.ai/api".to_string());
        let session_key = config
            .get_session_key("claude.ai")?
            .map(|(k, _)| k)
            .unwrap_or_default();
        Ok(Self::new(base_url, session_key))
    }

    fn v1_base_url(&self) -> String {
        self.base_url.replace("/api", "")
    }

    fn request_internal(
        &self,
        method: &str,
        url: &str,
        data: Option<&Value>,
        extra_headers: &[(&str, String)],
    ) -> Result<Value> {
        log::debug!("Making {method} request to {url}");
        let mut req = self
            .agent
            .request(method, url)
            .set("User-Agent", USER_AGENT)
            .set("Content-Type", "application/json")
            .set("Cookie", &format!("sessionKey={}", self.session_key));
        for (k, v) in extra_headers {
            req = req.set(k, v);
        }

        let result = match data {
            Some(d) => req.send_string(&serde_json::to_string(d)?),
            None => req.call(),
        };

        match result {
            Ok(resp) => {
                let mut body = String::new();
                resp.into_reader()
                    .read_to_string(&mut body)
                    .map_err(|e| CsError::Provider(format!("API request failed: {e}")))?;
                if body.trim().is_empty() {
                    return Ok(Value::Null);
                }
                serde_json::from_str(&body).map_err(|e| {
                    CsError::Provider(format!("Invalid JSON response from API: {e}"))
                })
            }
            Err(ureq::Error::Status(code, resp)) => Err(handle_http_error(code, resp)),
            Err(e) => Err(CsError::Provider(format!("API request failed: {e}"))),
        }
    }

    fn make_request(&self, method: &str, endpoint: &str, data: Option<&Value>) -> Result<Value> {
        let url = format!("{}{}", self.base_url, endpoint);
        self.request_internal(method, &url, data, &[])
    }

    /// v1 API requests (not under the /api prefix), with Anthropic headers.
    fn make_request_v1(
        &self,
        method: &str,
        endpoint: &str,
        data: Option<&Value>,
        organization_id: Option<&str>,
    ) -> Result<Value> {
        let url = format!("{}{}", self.v1_base_url(), endpoint);
        let mut headers: Vec<(&str, String)> =
            vec![("anthropic-version", "2023-06-01".to_string())];
        if let Some(org) = organization_id {
            headers.push(("x-organization-uuid", org.to_string()));
        }
        self.request_internal(method, &url, data, &headers)
    }

    pub fn get_organizations(&self) -> Result<Vec<Organization>> {
        let response = self.make_request("GET", "/organizations", None)?;
        let orgs = response.as_array().ok_or_else(|| {
            CsError::Provider("Unable to retrieve organization information".into())
        })?;
        let required: [&[&str]; 3] = [
            &["chat", "claude_pro"],
            &["chat", "raven"],
            &["chat", "claude_max"],
        ];
        Ok(orgs
            .iter()
            .filter(|org| {
                let caps: Vec<&str> = org
                    .get("capabilities")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                required
                    .iter()
                    .any(|set| set.iter().all(|c| caps.contains(c)))
            })
            .map(|org| Organization {
                id: org["uuid"].as_str().unwrap_or_default().to_string(),
                name: org["name"].as_str().unwrap_or_default().to_string(),
            })
            .collect())
    }

    pub fn get_projects(
        &self,
        organization_id: &str,
        include_archived: bool,
    ) -> Result<Vec<Project>> {
        let response = self.make_request(
            "GET",
            &format!("/organizations/{organization_id}/projects"),
            None,
        )?;
        let projects = response
            .as_array()
            .ok_or_else(|| CsError::Provider("Unable to retrieve projects".into()))?;
        Ok(projects
            .iter()
            .filter(|p| include_archived || p.get("archived_at").is_none_or(Value::is_null))
            .map(|p| Project {
                id: p["uuid"].as_str().unwrap_or_default().to_string(),
                name: p["name"].as_str().unwrap_or_default().to_string(),
                archived_at: p
                    .get("archived_at")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string()),
            })
            .collect())
    }

    pub fn list_files(&self, organization_id: &str, project_id: &str) -> Result<Vec<RemoteFile>> {
        let response = self.make_request(
            "GET",
            &format!("/organizations/{organization_id}/projects/{project_id}/docs"),
            None,
        )?;
        let files = response
            .as_array()
            .ok_or_else(|| CsError::Provider("Unable to list files".into()))?;
        Ok(files
            .iter()
            .map(|f| RemoteFile {
                uuid: f["uuid"].as_str().unwrap_or_default().to_string(),
                file_name: f["file_name"].as_str().unwrap_or_default().to_string(),
                content: f["content"].as_str().unwrap_or_default().to_string(),
                created_at: f["created_at"].as_str().unwrap_or_default().to_string(),
            })
            .collect())
    }

    pub fn upload_file(
        &self,
        organization_id: &str,
        project_id: &str,
        file_name: &str,
        content: &str,
    ) -> Result<Value> {
        let data = json!({ "file_name": file_name, "content": content });
        self.make_request(
            "POST",
            &format!("/organizations/{organization_id}/projects/{project_id}/docs"),
            Some(&data),
        )
    }

    pub fn delete_file(
        &self,
        organization_id: &str,
        project_id: &str,
        file_uuid: &str,
    ) -> Result<Value> {
        self.make_request(
            "DELETE",
            &format!("/organizations/{organization_id}/projects/{project_id}/docs/{file_uuid}"),
            None,
        )
    }

    pub fn archive_project(&self, organization_id: &str, project_id: &str) -> Result<Value> {
        let data = json!({ "is_archived": true });
        self.make_request(
            "PUT",
            &format!("/organizations/{organization_id}/projects/{project_id}"),
            Some(&data),
        )
    }

    pub fn create_project(
        &self,
        organization_id: &str,
        name: &str,
        description: &str,
    ) -> Result<Value> {
        let data = json!({ "name": name, "description": description, "is_private": true });
        self.make_request(
            "POST",
            &format!("/organizations/{organization_id}/projects"),
            Some(&data),
        )
    }

    pub fn get_chat_conversations(&self, organization_id: &str) -> Result<Value> {
        self.make_request(
            "GET",
            &format!("/organizations/{organization_id}/chat_conversations"),
            None,
        )
    }

    // Kept for API parity with the Python provider even though no CLI
    // command currently calls it.
    #[allow(dead_code)]
    pub fn get_published_artifacts(&self, organization_id: &str) -> Result<Value> {
        self.make_request(
            "GET",
            &format!("/organizations/{organization_id}/published_artifacts"),
            None,
        )
    }

    pub fn get_chat_conversation(
        &self,
        organization_id: &str,
        conversation_id: &str,
    ) -> Result<Value> {
        self.make_request(
            "GET",
            &format!(
                "/organizations/{organization_id}/chat_conversations/{conversation_id}?rendering_mode=raw"
            ),
            None,
        )
    }

    #[allow(dead_code)]
    pub fn get_artifact_content(
        &self,
        organization_id: &str,
        artifact_uuid: &str,
    ) -> Result<String> {
        let artifacts = self.get_published_artifacts(organization_id)?;
        if let Some(list) = artifacts.as_array() {
            for artifact in list {
                if artifact.get("published_artifact_uuid").and_then(Value::as_str)
                    == Some(artifact_uuid)
                {
                    return Ok(artifact
                        .get("artifact_content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string());
                }
            }
        }
        Err(CsError::Provider(format!(
            "Artifact with UUID {artifact_uuid} not found"
        )))
    }

    pub fn delete_chat(&self, organization_id: &str, conversation_uuids: &[String]) -> Result<Value> {
        let data = json!({ "conversation_uuids": conversation_uuids });
        self.make_request(
            "POST",
            &format!("/organizations/{organization_id}/chat_conversations/delete_many"),
            Some(&data),
        )
    }

    pub fn create_chat(
        &self,
        organization_id: &str,
        chat_name: &str,
        project_uuid: Option<&str>,
        model: Option<&str>,
    ) -> Result<Value> {
        let mut data = json!({
            "uuid": uuid::Uuid::new_v4().to_string(),
            "name": chat_name,
            "project_uuid": project_uuid,
        });
        if let Some(m) = model {
            data["model"] = Value::String(m.to_string());
        }
        self.make_request(
            "POST",
            &format!("/organizations/{organization_id}/chat_conversations"),
            Some(&data),
        )
    }

    /// Sends a message and invokes `on_event` for each SSE event payload.
    pub fn send_message(
        &self,
        organization_id: &str,
        chat_id: &str,
        prompt: &str,
        timezone: &str,
        model: Option<&str>,
        mut on_event: impl FnMut(&Value),
    ) -> Result<()> {
        let endpoint =
            format!("/organizations/{organization_id}/chat_conversations/{chat_id}/completion");
        let mut data = json!({
            "prompt": prompt,
            "timezone": timezone,
            "rendering_mode": "messages",
            "attachments": [],
            "files": [],
        });
        if let Some(m) = model {
            data["model"] = Value::String(m.to_string());
        }

        let url = format!("{}{}", self.base_url, endpoint);
        let req = self
            .agent
            .request("POST", &url)
            .set("User-Agent", USER_AGENT)
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .set("Cookie", &format!("sessionKey={}", self.session_key));

        let resp = match req.send_string(&serde_json::to_string(&data)?) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, resp)) => return Err(handle_http_error(code, resp)),
            Err(e) => return Err(CsError::Provider(format!("API request failed: {e}"))),
        };

        for event in SseReader::new(resp.into_reader()) {
            let event = event?;
            if !event.data.is_empty() {
                match serde_json::from_str::<Value>(&event.data) {
                    Ok(v) => on_event(&v),
                    Err(_) => on_event(&json!({ "error": "Failed to parse JSON" })),
                }
            }
            if event.name == "error" {
                on_event(&json!({ "error": event.data }));
            }
            if event.name == "done" {
                break;
            }
        }
        Ok(())
    }

    /// Get all web sessions from the v1 API endpoint.
    pub fn get_sessions(&self, organization_id: &str) -> Result<Value> {
        self.make_request_v1("GET", "/v1/sessions", None, Some(organization_id))
    }

    /// Get all environments from the v1 API endpoint.
    pub fn get_environments(&self, organization_id: &str) -> Result<Value> {
        let endpoint = format!(
            "/v1/environment_providers/private/organizations/{organization_id}/environments"
        );
        self.make_request_v1("GET", &endpoint, None, Some(organization_id))
    }

    /// Get all code repositories available for Claude Code sessions.
    pub fn get_code_repos(&self, organization_id: &str, skip_status: bool) -> Result<Value> {
        let params = if skip_status { "?skip_status=true" } else { "" };
        self.make_request(
            "GET",
            &format!("/organizations/{organization_id}/code/repos{params}"),
            None,
        )
    }

    pub fn archive_session(&self, organization_id: &str, session_id: &str) -> Result<Value> {
        self.make_request_v1(
            "POST",
            &format!("/v1/sessions/{session_id}/archive"),
            None,
            Some(organization_id),
        )
    }

    /// Send input/prompt to a Claude Code session. The actual endpoint is
    /// undocumented, so several candidates are tried in order (port of
    /// `send_session_input`).
    pub fn send_session_input(
        &self,
        organization_id: &str,
        session_id: &str,
        prompt: &str,
    ) -> Result<Value> {
        let attempts = [
            (
                format!("/v1/sessions/{session_id}/prompt"),
                json!({ "prompt": prompt }),
            ),
            (
                format!("/v1/sessions/{session_id}/message"),
                json!({ "message": prompt }),
            ),
            (
                format!("/v1/sessions/{session_id}/messages"),
                json!({ "content": prompt }),
            ),
            (
                format!("/v1/sessions/{session_id}/input"),
                json!({ "input": prompt }),
            ),
        ];

        let mut last_error = None;
        for (endpoint, data) in &attempts {
            match self.make_request_v1("POST", endpoint, Some(data), Some(organization_id)) {
                Ok(v) => return Ok(v),
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error.unwrap_or_else(|| CsError::Provider("No endpoints attempted".into())))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        &self,
        organization_id: &str,
        title: &str,
        environment_id: &str,
        git_repo_url: Option<&str>,
        git_repo_owner: Option<&str>,
        git_repo_name: Option<&str>,
        branch_name: Option<&str>,
        model: &str,
    ) -> Result<Value> {
        let mut data = json!({
            "title": title,
            "environment_id": environment_id,
            "session_context": { "model": model },
        });

        if let Some(url) = git_repo_url {
            data["session_context"]["sources"] =
                json!([{ "type": "git_repository", "url": url }]);
        }

        if let (Some(owner), Some(name)) = (git_repo_owner, git_repo_name) {
            let branch = match branch_name {
                Some(b) => b.to_string(),
                None => {
                    // Generate from title: lowercase, special chars to hyphens, max 50 chars
                    let re = Regex::new(r"[^a-z0-9]+").unwrap();
                    let safe: String = re
                        .replace_all(&title.to_lowercase(), "-")
                        .trim_matches('-')
                        .chars()
                        .take(50)
                        .collect();
                    format!("claude/{safe}")
                }
            };
            data["session_context"]["outcomes"] = json!([{
                "type": "git_repository",
                "git_info": {
                    "type": "github",
                    "repo": format!("{owner}/{name}"),
                    "branches": [branch],
                },
            }]);
        }

        self.make_request_v1("POST", "/v1/sessions", Some(&data), Some(organization_id))
    }

    /// Streams events from a Claude Code session. Waits up to 30s for the
    /// first event (mirrors the SIGALRM timeout in the Python version).
    pub fn stream_session_events(
        &self,
        organization_id: &str,
        session_id: &str,
        mut on_event: impl FnMut(&Value) -> bool,
    ) -> Result<()> {
        let url = format!("{}/v1/sessions/{session_id}/events", self.v1_base_url());
        log::debug!("Opening SSE stream to {url}");
        let req = self
            .agent
            .request("GET", &url)
            .set("User-Agent", USER_AGENT)
            .set("Accept", "text/event-stream")
            .set("anthropic-version", "2023-06-01")
            .set("x-organization-uuid", organization_id)
            .set("Cookie", &format!("sessionKey={}", self.session_key));

        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, resp)) => return Err(handle_http_error(code, resp)),
            Err(e) => return Err(CsError::Provider(format!("API request failed: {e}"))),
        };

        let (tx, rx) = mpsc::channel::<Result<SseEvent>>();
        std::thread::spawn(move || {
            for event in SseReader::new(resp.into_reader()) {
                if tx.send(event).is_err() {
                    break;
                }
            }
        });

        let mut first = true;
        loop {
            let event = if first {
                match rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(e) => e?,
                    Err(_) => {
                        on_event(&json!({
                            "error": "timeout",
                            "message": "No events received from session within 30 seconds",
                        }));
                        return Ok(());
                    }
                }
            } else {
                match rx.recv() {
                    Ok(e) => e?,
                    Err(_) => return Ok(()),
                }
            };
            first = false;

            if !event.data.trim().is_empty() {
                let parsed = serde_json::from_str::<Value>(&event.data).unwrap_or_else(|_| {
                    json!({ "error": "Failed to parse JSON", "raw_data": event.data })
                });
                if !on_event(&parsed) {
                    return Ok(());
                }
            }
            if event.name == "error" {
                on_event(&json!({ "error": event.data }));
                return Ok(());
            }
            if event.name == "done" {
                return Ok(());
            }
        }
    }
}

fn handle_http_error(code: u16, resp: ureq::Response) -> CsError {
    let mut body = String::new();
    let _ = resp.into_reader().read_to_string(&mut body);
    log::debug!("Request failed with status {code}: {body}");

    match code {
        403 => CsError::Provider("Received a 403 Forbidden error.".into()),
        429 => {
            // The 429 body nests JSON: error.message is itself a JSON string
            // containing { "resetsAt": <unix> }.
            let msg = (|| -> Option<String> {
                let data: Value = serde_json::from_str(&body).ok()?;
                let inner = data.get("error")?.get("message")?.as_str()?;
                let inner: Value = serde_json::from_str(inner).ok()?;
                let resets_at = inner.get("resetsAt")?.as_i64()?;
                let local = Local.timestamp_opt(resets_at, 0).single()?;
                Some(format!(
                    "Message limit exceeded. Try again after {}",
                    local.format("%a %b %d %Y %H:%M:%S %z")
                ))
            })()
            .unwrap_or_else(|| {
                "HTTP 429: Too Many Requests. Failed to parse error response".to_string()
            });
            CsError::Provider(msg)
        }
        _ => CsError::Provider(format!(
            "API request failed with status code {code}: {body}"
        )),
    }
}

struct SseEvent {
    name: String,
    data: String,
}

/// Minimal SSE parser over a byte stream (replacement for Python's sseclient).
struct SseReader<R: Read> {
    lines: std::io::Lines<BufReader<R>>,
    done: bool,
}

impl<R: Read> SseReader<R> {
    fn new(reader: R) -> Self {
        SseReader {
            lines: BufReader::new(reader).lines(),
            done: false,
        }
    }
}

impl<R: Read> Iterator for SseReader<R> {
    type Item = Result<SseEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut name = String::new();
        let mut data = String::new();
        let mut saw_field = false;

        loop {
            match self.lines.next() {
                None => {
                    self.done = true;
                    if saw_field {
                        return Some(Ok(SseEvent { name, data }));
                    }
                    return None;
                }
                Some(Err(e)) => {
                    self.done = true;
                    return Some(Err(CsError::Provider(format!("Stream error: {e}"))));
                }
                Some(Ok(line)) => {
                    if line.is_empty() {
                        if saw_field {
                            return Some(Ok(SseEvent { name, data }));
                        }
                        continue;
                    }
                    if let Some(rest) = line.strip_prefix("event:") {
                        name = rest.trim().to_string();
                        saw_field = true;
                    } else if let Some(rest) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                        saw_field = true;
                    }
                    // Comments (lines starting with ':') and other fields ignored
                }
            }
        }
    }
}

/// Provider factory (port of `provider_factory.get_provider`).
pub fn get_provider(config: &FileConfig, provider_name: &str) -> Result<ClaudeProvider> {
    match provider_name {
        "claude.ai" => ClaudeProvider::from_config(config),
        other => Err(CsError::Provider(format!("Unsupported provider: {other}"))),
    }
}
