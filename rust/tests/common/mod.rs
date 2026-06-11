//! Shared test support: a minimal HTTP mock of the claude.ai API.
//! Successor to the Python `tests/mock_http_server.py`, with extras:
//! session-key and header validation, optional gzip responses, a request
//! log for ordering assertions, and chat conversations with artifacts.
//!
//! Also runnable standalone for manual CLI testing:
//! `cargo run --example mock_server -- [port]`
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
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
    /// When true the body is gzip-compressed on the wire with
    /// `Content-Encoding: gzip` (exercises the client's decompression).
    pub gzip: bool,
}

impl Response {
    pub fn json(body: Value) -> Self {
        Response {
            status: 200,
            content_type: "application/json",
            body: body.to_string(),
            gzip: false,
        }
    }

    pub fn json_gzip(body: Value) -> Self {
        Response {
            gzip: true,
            ..Response::json(body)
        }
    }

    pub fn status(status: u16, body: Value) -> Self {
        Response {
            status,
            content_type: "application/json",
            body: body.to_string(),
            gzip: false,
        }
    }

    pub fn sse(body: &str) -> Self {
        Response {
            status: 200,
            content_type: "text/event-stream",
            body: body.to_string(),
            gzip: false,
        }
    }

    pub fn no_content() -> Self {
        Response {
            status: 204,
            content_type: "application/json",
            body: String::new(),
            gzip: false,
        }
    }

    pub fn not_found() -> Self {
        Response {
            status: 404,
            content_type: "text/plain",
            body: "Not Found".to_string(),
            gzip: false,
        }
    }
}

pub type Handler = Arc<dyn Fn(&Request) -> Response + Send + Sync>;

/// Minimal single-purpose HTTP server for tests. Dispatches every request to
/// `handler` and records (method, path, body) in `requests`.
pub struct MockServer {
    pub port: u16,
    pub requests: Arc<Mutex<Vec<(String, String, String)>>>,
}

impl MockServer {
    /// Starts on an ephemeral port (for tests).
    pub fn start(handler: Handler) -> Self {
        Self::start_on("127.0.0.1:0", handler)
    }

    /// Starts on a fixed address (for the standalone example).
    pub fn start_on(addr: &str, handler: Handler) -> Self {
        let listener = TcpListener::bind(addr).expect("bind mock server");
        let port = listener.local_addr().unwrap().port();
        let requests: Arc<Mutex<Vec<(String, String, String)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let handler = Arc::clone(&handler);
                let log = Arc::clone(&log);
                std::thread::spawn(move || handle_connection(stream, handler, log));
            }
        });
        MockServer { port, requests }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/api", self.port)
    }
}

fn handle_connection(
    stream: TcpStream,
    handler: Handler,
    log: Arc<Mutex<Vec<(String, String, String)>>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.trim().is_empty() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
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
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
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
        headers,
    };
    log.lock().unwrap().push((
        request.method.clone(),
        request.path.clone(),
        request.body.clone(),
    ));
    let response = handler(&request);

    let (payload, encoding_header) = if response.gzip {
        let mut enc =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(response.body.as_bytes()).expect("gzip body");
        (
            enc.finish().expect("finish gzip"),
            "Content-Encoding: gzip\r\n",
        )
    } else {
        (response.body.into_bytes(), "")
    };

    let reason = match response.status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let mut out = stream;
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        encoding_header,
        payload.len()
    );
    let _ = out.write_all(header.as_bytes());
    let _ = out.write_all(&payload);
}

fn has_valid_session_cookie(req: &Request) -> bool {
    req.header("cookie")
        .map(|cookie| {
            cookie.split(';').any(|part| {
                part.trim()
                    .strip_prefix("sessionKey=")
                    .map(|v| v.starts_with("sk-ant"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Stateful router mirroring (and validating more strictly than) the real
/// claude.ai API: orgs, projects, chats with artifacts, Claude Code sessions,
/// and an in-memory docs store supporting list/upload/delete.
///
/// All endpoints require a `sessionKey=sk-ant...` cookie (401 otherwise);
/// /v1/ endpoints additionally require the `anthropic-version` and
/// `x-organization-uuid` headers (400 otherwise).
pub fn claude_api_router(docs: Arc<Mutex<Vec<Value>>>) -> Handler {
    let upload_counter = AtomicUsize::new(100);
    Arc::new(move |req: &Request| {
        let path = req.path.split('?').next().unwrap_or("");

        if !has_valid_session_cookie(req) {
            return Response::status(
                401,
                json!({"error": {"type": "authentication_error",
                       "message": "missing or invalid sessionKey cookie"}}),
            );
        }
        if path.starts_with("/v1/") {
            if req.header("anthropic-version").is_none() {
                return Response::status(
                    400,
                    json!({"error": "missing anthropic-version header"}),
                );
            }
            if req.header("x-organization-uuid").is_none() {
                return Response::status(
                    400,
                    json!({"error": "missing x-organization-uuid header"}),
                );
            }
        }

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
                Response::no_content()
            }
            ("POST", p) if p.ends_with("/completion") => Response::sse(concat!(
                "data: {\"completion\": \"Hello\"}\n\n",
                "data: {\"completion\": \" there. \"}\n\n",
                "data: {\"completion\": \"I apologize for the confusion. You're right.\"}\n\n",
                "event: done\n\n",
            )),
            // chat1 belongs to the project created via POST .../projects so
            // `chat pull` picks it up; chat2 has no project and is skipped.
            ("GET", p) if p.ends_with("/chat_conversations") => Response::json(json!([
                {"uuid": "chat1", "name": "Test Chat 1",
                 "project": {"uuid": "new_proj", "name": "New Project"},
                 "updated_at": "2024-01-02T00:00:00Z"},
                {"uuid": "chat2", "name": "Test Chat 2",
                 "updated_at": "2024-01-03T00:00:00Z"},
            ])),
            ("GET", p) if p.contains("/chat_conversations/") => Response::json(json!({
                "uuid": "chat1",
                "name": "Test Chat 1",
                "chat_messages": [
                    {"uuid": "msg1", "sender": "human", "text": "Hello"},
                    {"uuid": "msg2", "sender": "assistant",
                     "text": "Sure: <antArtifact identifier=\"hello-script\" type=\"application/vnd.ant.code\" title=\"Hello Script\">print(\"hi\")</antArtifact>"},
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
