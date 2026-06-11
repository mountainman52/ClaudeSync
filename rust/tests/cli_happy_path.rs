//! End-to-end happy path driving the actual `claudesync` binary against the
//! mock API (port of tests/test_happy_path.py and test_chat_happy_path.py).
//!
//! The binary runs with HOME pointed at a temp directory (isolating global
//! config and session keys) and a fake `ssh-keygen` on PATH so login's key
//! type check works without openssh installed.

mod common;

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use common::{claude_api_router, MockServer};

struct CliEnv {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
    fakebin: tempfile::TempDir,
}

impl CliEnv {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let fakebin = tempfile::tempdir().unwrap();

        // An "SSH key" only needs to provide bytes for key derivation
        let ssh_dir = home.path().join(".ssh");
        std::fs::create_dir(&ssh_dir).unwrap();
        std::fs::write(ssh_dir.join("id_ed25519"), "fake ed25519 key material").unwrap();

        // Fake ssh-keygen: login shells out to `ssh-keygen -l -f <key>` to
        // verify the key type; answer like a real ed25519 fingerprint.
        let fake_keygen = fakebin.path().join("ssh-keygen");
        std::fs::write(
            &fake_keygen,
            "#!/bin/sh\necho \"256 SHA256:fakefingerprint test@example.com (ED25519)\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_keygen, std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }

        CliEnv {
            home,
            project,
            fakebin,
        }
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> (bool, String) {
        let path = format!(
            "{}:{}",
            self.fakebin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let output = Command::new(env!("CARGO_BIN_EXE_claudesync"))
            .args(args)
            .current_dir(cwd)
            .env("HOME", self.home.path())
            .env("PATH", path)
            .output()
            .expect("run claudesync binary");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (output.status.success(), combined)
    }

    fn run_ok(&self, cwd: &Path, args: &[&str]) -> String {
        let (ok, output) = self.run(cwd, args);
        assert!(ok, "command {args:?} failed:\n{output}");
        output
    }
}

#[test]
fn cli_happy_path_login_init_push_and_chat() {
    let docs = Arc::new(Mutex::new(vec![]));
    let server = MockServer::start(claude_api_router(Arc::clone(&docs)));
    let env = CliEnv::new();
    let proj = env.project.path().to_path_buf();

    // Point the global config at the mock server
    let out = env.run_ok(
        &proj,
        &["config", "set", "claude_api_url", &server.base_url()],
    );
    assert!(out.contains("Configuration claude_api_url set to"), "{out}");

    // Step 1: login (non-interactive: key + auto-approved expiry)
    let out = env.run_ok(
        &proj,
        &[
            "auth",
            "login",
            "--provider",
            "claude.ai",
            "--session-key",
            "sk-ant-1234",
            "--auto-approve",
        ],
    );
    assert!(
        out.contains("Successfully authenticated with claude.ai"),
        "{out}"
    );
    let out = env.run_ok(&proj, &["auth", "ls"]);
    assert!(out.contains("claude.ai"), "{out}");

    // Create the local .claudesync dir so subsequent settings persist
    std::fs::create_dir(proj.join(".claudesync")).unwrap();

    // Step 2: set organization
    let out = env.run_ok(&proj, &["organization", "set", "--org-id", "org1"]);
    assert!(out.contains("Selected organization: Test Org 1"), "{out}");

    // Step 3: create project (init --new)
    let proj_str = proj.to_string_lossy().to_string();
    let out = env.run_ok(
        &proj,
        &[
            "project",
            "init",
            "--new",
            "--name",
            "New Project",
            "--description",
            "Test description",
            "--local-path",
            &proj_str,
        ],
    );
    assert!(
        out.contains("Project 'New Project' (uuid: new_proj) has been created successfully"),
        "{out}"
    );
    assert!(
        out.contains("Remote URL: https://claude.ai/project/new_proj"),
        "{out}"
    );

    // Step 4: push a file
    std::fs::write(proj.join("test.txt"), "hello from the happy path").unwrap();
    let out = env.run_ok(&proj, &["push"]);
    assert!(
        out.contains("Main project 'New Project' synced successfully"),
        "{out}"
    );
    {
        let docs = docs.lock().unwrap();
        assert_eq!(docs.len(), 1, "expected one uploaded doc: {docs:?}");
        assert_eq!(docs[0]["file_name"], "test.txt");
        assert_eq!(docs[0]["content"], "hello from the happy path");
    }

    // Step 5: send a chat message (streams the mocked SSE completion)
    let out = env.run_ok(&proj, &["chat", "message", "Hello, Claude!"]);
    assert!(out.contains("New chat created with ID: new_chat"), "{out}");
    assert!(out.contains("Hello there."), "{out}");
    assert!(
        out.contains("I apologize for the confusion. You're right."),
        "{out}"
    );

    // Step 6: pull chats — metadata, messages, and extracted artifacts land
    // on disk; the chat belonging to another project is skipped
    env.run_ok(&proj, &["chat", "pull"]);
    let chat_dir = proj.join("claude_chats").join("chat1");
    assert!(chat_dir.join("metadata.json").exists());
    assert!(chat_dir.join("msg1.json").exists());
    assert!(chat_dir.join("msg2.json").exists());
    let artifact = chat_dir.join("artifacts").join("hello-script.txt");
    let artifact_content = std::fs::read_to_string(&artifact).expect("artifact file written");
    assert_eq!(artifact_content, "print(\"hi\")");
    assert!(!proj.join("claude_chats").join("chat2").exists());

    // Step 7: dry run reports a diff instead of pushing
    std::fs::write(proj.join("test.txt"), "edited content").unwrap();
    std::fs::write(proj.join("second.txt"), "another file").unwrap();
    let out = env.run_ok(&proj, &["push", "--dryrun"]);
    assert!(out.contains("+ second.txt"), "{out}");
    assert!(out.contains("~ test.txt"), "{out}");
    assert!(out.contains("Dry run: no changes were sent."), "{out}");
    {
        // Nothing was actually pushed
        let docs = docs.lock().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["content"], "hello from the happy path");
    }
}
