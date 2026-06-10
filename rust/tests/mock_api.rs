//! Integration tests against a local mock of the claude.ai API,
//! mirroring the Python `tests/mock_http_server.py` approach.

mod mock {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

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
        pub fn json(body: serde_json::Value) -> Self {
            Response {
                status: 200,
                content_type: "application/json",
                body: body.to_string(),
            }
        }

        pub fn status(status: u16, body: serde_json::Value) -> Self {
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
}

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use claudesync::provider::ClaudeProvider;
use claudesync::sync::{retry_on_403, SyncManager};
use claudesync::utils::compute_md5_hash;
use mock::{MockServer, Request, Response};

/// Stateful router mirroring the Python MockClaudeAIHandler: orgs, projects,
/// and an in-memory docs store supporting list/upload/delete.
fn claude_api_router(docs: Arc<Mutex<Vec<Value>>>) -> mock::Handler {
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
            _ => Response::not_found(),
        }
    })
}

fn provider_for(server: &MockServer) -> ClaudeProvider {
    ClaudeProvider::new(server.base_url(), "sk-ant-test".to_string())
}

#[test]
fn organizations_filtered_by_capabilities() {
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let orgs = provider.get_organizations().unwrap();
    // org2 only has "chat" and must be filtered out
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0].id, "org1");
    assert_eq!(orgs[0].name, "Test Org 1");
}

#[test]
fn projects_exclude_archived_by_default() {
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let active = provider.get_projects("org1", false).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, "proj1");

    let all = provider.get_projects("org1", true).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].archived_at.as_deref(), Some("2023-01-01"));
}

#[test]
fn create_project_and_upload_roundtrip() {
    let docs = Arc::new(Mutex::new(vec![]));
    let server = MockServer::start(claude_api_router(Arc::clone(&docs)));
    let provider = provider_for(&server);

    let project = provider.create_project("org1", "New Project", "desc").unwrap();
    assert_eq!(project["uuid"], "new_proj");

    provider
        .upload_file("org1", "proj1", "hello.txt", "hello world")
        .unwrap();
    let files = provider.list_files("org1", "proj1").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name, "hello.txt");
    assert_eq!(files[0].content, "hello world");

    provider.delete_file("org1", "proj1", &files[0].uuid).unwrap();
    assert!(provider.list_files("org1", "proj1").unwrap().is_empty());
}

#[test]
fn sync_uploads_updates_and_prunes() {
    // Remote starts with: an up-to-date file, a stale file, and a file
    // that no longer exists locally.
    let docs = Arc::new(Mutex::new(vec![
        json!({"uuid": "r1", "file_name": "same.txt", "content": "same content",
               "created_at": "2023-01-01T00:00:00Z"}),
        json!({"uuid": "r2", "file_name": "changed.txt", "content": "old version",
               "created_at": "2023-01-01T00:00:00Z"}),
        json!({"uuid": "r3", "file_name": "deleted.txt", "content": "obsolete",
               "created_at": "2023-01-01T00:00:00Z"}),
    ]));
    let server = MockServer::start(claude_api_router(Arc::clone(&docs)));
    let provider = provider_for(&server);

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("same.txt"), "same content").unwrap();
    std::fs::write(dir.path().join("changed.txt"), "new version").unwrap();
    std::fs::write(dir.path().join("new.txt"), "brand new").unwrap();

    let local_files: BTreeMap<String, String> = [
        ("same.txt", "same content"),
        ("changed.txt", "new version"),
        ("new.txt", "brand new"),
    ]
    .into_iter()
    .map(|(name, content)| (name.to_string(), compute_md5_hash(content)))
    .collect();

    let sync_manager = SyncManager {
        active_organization_id: "org1".to_string(),
        active_project_id: "proj1".to_string(),
        local_path: dir.path().to_path_buf(),
        upload_delay: 0.0,
        two_way_sync: false,
        prune_remote_files: true,
        compression_algorithm: "none".to_string(),
    };

    let remote_files = provider.list_files("org1", "proj1").unwrap();
    sync_manager
        .sync(&provider, &local_files, &remote_files)
        .unwrap();

    let final_docs = docs.lock().unwrap();
    let mut names: Vec<&str> = final_docs
        .iter()
        .filter_map(|f| f["file_name"].as_str())
        .collect();
    names.sort();
    assert_eq!(names, ["changed.txt", "new.txt", "same.txt"]);

    // The unchanged file kept its original uuid (was never re-uploaded);
    // the changed file was deleted and re-uploaded with new content.
    let by_name = |n: &str| {
        final_docs
            .iter()
            .find(|f| f["file_name"].as_str() == Some(n))
            .unwrap()
            .clone()
    };
    assert_eq!(by_name("same.txt")["uuid"], "r1");
    assert_ne!(by_name("changed.txt")["uuid"], "r2");
    assert_eq!(by_name("changed.txt")["content"], "new version");
    assert!(!final_docs.iter().any(|f| f["uuid"] == "r3"));
}

#[test]
fn send_message_streams_sse_events() {
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let mut completions = String::new();
    provider
        .send_message("org1", "chat1", "hi", "UTC", None, |event| {
            if let Some(text) = event.get("completion").and_then(Value::as_str) {
                completions.push_str(text);
            }
        })
        .unwrap();

    assert_eq!(
        completions,
        "Hello there. I apologize for the confusion. You're right."
    );
}

#[test]
fn http_403_maps_to_forbidden_error_and_is_retried() {
    // Fail with 403 twice, then succeed — retry_on_403 should recover.
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_for_handler = Arc::clone(&hits);
    let server = MockServer::start(Arc::new(move |_req: &Request| {
        if hits_for_handler.fetch_add(1, Ordering::SeqCst) < 2 {
            Response::status(403, json!({"error": "forbidden"}))
        } else {
            Response::json(json!([
                {"uuid": "org1", "name": "Test Org 1", "capabilities": ["chat", "claude_pro"]},
            ]))
        }
    }));
    let provider = provider_for(&server);

    // Direct call surfaces the mapped error message...
    let err = provider.get_organizations().unwrap_err();
    assert_eq!(err.to_string(), "Received a 403 Forbidden error.");

    // ...and the retry wrapper eventually succeeds (attempt 2 fails, 3 works).
    let orgs = retry_on_403(|| provider.get_organizations()).unwrap();
    assert_eq!(orgs.len(), 1);
    assert!(hits.load(Ordering::SeqCst) >= 3);
}

#[test]
fn http_429_reports_reset_time() {
    let server = MockServer::start(Arc::new(|_req: &Request| {
        // error.message is itself a JSON-encoded string, as claude.ai sends it
        Response::status(
            429,
            json!({"error": {"message": "{\"resetsAt\": 1750000000}"}}),
        )
    }));
    let provider = provider_for(&server);

    let err = provider.get_organizations().unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.starts_with("Message limit exceeded. Try again after "),
        "unexpected message: {msg}"
    );
}

#[test]
fn unexpected_status_includes_code_and_body() {
    let server = MockServer::start(Arc::new(|_req: &Request| {
        Response::status(500, json!({"error": "boom"}))
    }));
    let provider = provider_for(&server);

    let err = provider.get_organizations().unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("status code 500"), "unexpected message: {msg}");
    assert!(msg.contains("boom"), "unexpected message: {msg}");
}
