//! Shared test support: a minimal HTTP mock of the claude.ai API,
//! mirroring the Python `tests/mock_http_server.py`.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use claudesync::provider::ClaudeProvider;

pub struct Request {
    pub method: String,
    pub path: String,
    pub body: String,
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Response {
    pub fn json(body: Value) -> Self {
        Response {
            status: 200,
            content_type: "application/json",
            body: body.to_string(),
        }
    }

    pub fn status(status: u16, body: Value) -> Self {
        Response {
            status,
            content_type: "application/json",
            body: body.to_string(),
        }
    }

    pub fn sse(body: &str) -> Self {
        Response {
            status: 200,
            content_type: "text/event-stream",
            body: body.to_string(),
        }
    }

    pub fn not_found() -> Self {
        Response {
            status: 404,
            content_type: "text/plain",
            body: "Not Found".to_string(),
        }
    }
}

pub type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

/// Minimal single-purpose HTTP server for tests. Listens on an ephemeral
/// port and dispatches every request to `handler`.
pub struct MockServer {
    pub port: u16,
}

impl MockServer {
    pub fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let handler = Arc::clone(&handler);
                std::thread::spawn(move || handle_connection(stream, handler));
            }
        });
        MockServer { port }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/api", self.port)
    }
}

fn handle_connection(stream: TcpStream, handler: Handler) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }

    let request = Request {
        method,
        path,
        body: String::from_utf8_lossy(&body).to_string(),
    };
    let response = handler(&request);

    let reason = match response.status {
        200 => "OK",
        204 => "No Content",
        403 => "Forbidden",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let mut out = stream;
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    let _ = out.write_all(header.as_bytes());
    let _ = out.write_all(response.body.as_bytes());
}

/// Stateful router mirroring the Python MockClaudeAIHandler: orgs, projects,
/// chats, Claude Code sessions, and an in-memory docs store supporting
/// list/upload/delete.
pub fn claude_api_router(docs: Arc<Mutex<Vec<Value>>>) -> Handler {
    let upload_counter = AtomicUsize::new(100);
    Arc::new(move |req: &Request| {
        let path = req.path.split('?').next().unwrap_or("");
        match (req.method.as_str(), path) {
            ("GET", "/api/organizations") => Response::json(json!([
                {"uuid": "org1", "name": "Test Org 1", "capabilities": ["chat", "claude_pro"]},
                {"uuid": "org2", "name": "Test Org 2", "capabilities": ["chat"]},
            ])),
            ("GET", p) if p.starts_with("/api/organizations/") && p.ends_with("/projects") => {
                Response::json(json!([
                    {"uuid": "proj1", "name": "Test Project 1", "archived_at": null},
                    {"uuid": "proj2", "name": "Test Project 2", "archived_at": "2023-01-01"},
                ]))
            }
            ("POST", p) if p.starts_with("/api/organizations/") && p.ends_with("/projects") => {
                Response::json(json!({"uuid": "new_proj", "name": "New Project"}))
            }
            ("GET", p) if p.starts_with("/api/organizations/") && p.ends_with("/docs") => {
                Response::json(Value::Array(docs.lock().unwrap().clone()))
            }
            ("POST", p) if p.starts_with("/api/organizations/") && p.ends_with("/docs") => {
                let data: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
                let file = json!({
                    "uuid": format!("file_{}", upload_counter.fetch_add(1, Ordering::SeqCst)),
                    "file_name": data["file_name"],
                    "content": data["content"],
                    "created_at": "2023-01-01T00:00:00Z",
                });
                docs.lock().unwrap().push(file.clone());
                Response::json(file)
            }
            ("DELETE", p) if p.contains("/docs/") => {
                let uuid = p.rsplit('/').next().unwrap_or("");
                docs.lock()
                    .unwrap()
                    .retain(|f| f["uuid"].as_str() != Some(uuid));
                Response {
                    status: 204,
                    content_type: "application/json",
                    body: String::new(),
                }
            }
            ("POST", p) if p.ends_with("/completion") => Response::sse(concat!(
                "data: {\"completion\": \"Hello\"}\n\n",
                "data: {\"completion\": \" there. \"}\n\n",
                "data: {\"completion\": \"I apologize for the confusion. You're right.\"}\n\n",
                "event: done\n\n",
            )),
            ("GET", p) if p.ends_with("/chat_conversations") => Response::json(json!([
                {"uuid": "chat1", "name": "Test Chat 1"},
                {"uuid": "chat2", "name": "Test Chat 2"},
            ])),
            ("GET", p) if p.contains("/chat_conversations/") => Response::json(json!({
                "uuid": "chat1",
                "name": "Test Chat 1",
                "messages": [
                    {"uuid": "msg1", "content": "Hello"},
                    {"uuid": "msg2", "content": "World"},
                ],
            })),
            ("POST", p) if p.ends_with("/chat_conversations") => {
                Response::json(json!({"uuid": "new_chat", "name": "New Chat"}))
            }
            // Claude Code v1 session endpoints (not under /api)
            ("POST", "/v1/sessions") => {
                let data: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
                Response::json(json!({
                    "id": "session_test123",
                    "title": data["title"],
                    "environment_id": data["environment_id"],
                    "session_status": "running",
                    "session_context": data["session_context"],
                    "type": "session",
                    "created_at": "2025-11-16T08:09:45.536149996Z",
                    "updated_at": "2025-11-16T08:09:45.536149996Z",
                }))
            }
            ("GET", p) if p.starts_with("/v1/sessions/") && p.ends_with("/events") => {
                Response::sse(concat!(
                    "data: {\"type\":\"session_status\",\"status\":\"running\"}\n\n",
                    "data: {\"type\":\"message\",\"content\":\"Starting Claude Code session...\"}\n\n",
                    "data: {\"type\":\"message\",\"content\":\"Environment initialized\"}\n\n",
                    "event: done\n\n",
                ))
            }
            // Only the /input variant exists; /prompt, /message and /messages
            // 404 so the provider's endpoint fallback is exercised.
            ("POST", p) if p.starts_with("/v1/sessions/") && p.ends_with("/input") => {
                let data: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
                Response::json(json!({
                    "status": "accepted",
                    "input_received": data["input"],
                }))
            }
            _ => Response::not_found(),
        }
    })
}

pub fn provider_for(server: &MockServer) -> ClaudeProvider {
    ClaudeProvider::new(server.base_url(), "sk-ant-test".to_string())
}
