//! Integration tests against a local mock of the claude.ai API,
//! mirroring the Python `tests/test_claude_ai.py` approach.

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use claudesync::sync::{retry_on_403, SyncManager};
use claudesync::utils::compute_md5_hash;
use common::{claude_api_router, provider_for, MockServer, Request, Response};

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
fn chat_conversations_listed_and_fetched() {
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let chats = provider.get_chat_conversations("org1").unwrap();
    let chats = chats.as_array().unwrap();
    assert_eq!(chats.len(), 2);
    assert_eq!(chats[0]["uuid"], "chat1");
    assert_eq!(chats[0]["name"], "Test Chat 1");

    let chat = provider.get_chat_conversation("org1", "chat1").unwrap();
    assert_eq!(chat["uuid"], "chat1");
    assert_eq!(chat["messages"].as_array().unwrap().len(), 2);

    let new_chat = provider.create_chat("org1", "New Chat", Some("proj1"), None).unwrap();
    assert_eq!(new_chat["uuid"], "new_chat");
}

#[test]
fn create_session_with_explicit_branch() {
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let result = provider
        .create_session(
            "org1",
            "Test Session",
            "env_test",
            Some("https://github.com/test/repo"),
            Some("test"),
            Some("repo"),
            Some("claude/test-branch"),
            "claude-sonnet-4-5-20250929",
        )
        .unwrap();

    assert_eq!(result["title"], "Test Session");
    assert_eq!(result["session_status"], "running");
    assert_eq!(result["environment_id"], "env_test");

    let outcomes = result["session_context"]["outcomes"].as_array().unwrap();
    assert_eq!(outcomes[0]["type"], "git_repository");
    assert_eq!(outcomes[0]["git_info"]["repo"], "test/repo");
    assert_eq!(outcomes[0]["git_info"]["branches"][0], "claude/test-branch");
}

#[test]
fn create_session_auto_generates_branch_from_title() {
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let result = provider
        .create_session(
            "org1",
            "Auto Branch Session!",
            "env_test",
            Some("https://github.com/test/repo"),
            Some("test"),
            Some("repo"),
            None, // no branch: should be generated from the title
            "claude-sonnet-4-5-20250929",
        )
        .unwrap();

    let branch = result["session_context"]["outcomes"][0]["git_info"]["branches"][0]
        .as_str()
        .unwrap();
    assert_eq!(branch, "claude/auto-branch-session");
}

#[test]
fn create_session_minimal_has_no_git_context() {
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let result = provider
        .create_session(
            "org1",
            "Minimal Session",
            "env_test",
            None,
            None,
            None,
            None,
            "claude-sonnet-4-5-20250929",
        )
        .unwrap();

    assert_eq!(result["title"], "Minimal Session");
    let context = &result["session_context"];
    assert!(context.get("sources").is_none());
    assert!(context.get("outcomes").is_none());
}

#[test]
fn stream_session_events_collects_events_until_done() {
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let mut events: Vec<Value> = Vec::new();
    provider
        .stream_session_events("org1", "session_test123", |event| {
            events.push(event.clone());
            true
        })
        .unwrap();

    assert!(events.len() >= 3, "expected >= 3 events, got {events:?}");
    assert_eq!(events[0]["type"], "session_status");
    assert_eq!(events[0]["status"], "running");
    assert_eq!(events[1]["type"], "message");
    assert!(events[1]["content"]
        .as_str()
        .unwrap()
        .contains("Starting Claude Code"));
}

#[test]
fn send_session_input_falls_back_to_working_endpoint() {
    // /prompt, /message and /messages 404; the provider must fall through
    // to /input and succeed.
    let server = MockServer::start(claude_api_router(Arc::new(Mutex::new(vec![]))));
    let provider = provider_for(&server);

    let result = provider
        .send_session_input("org1", "session_test123", "Hello, please help me fix a bug")
        .unwrap();
    assert_eq!(result["status"], "accepted");
    assert_eq!(result["input_received"], "Hello, please help me fix a bug");
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
