//! Standalone mock of the claude.ai API for manual CLI testing — the Rust
//! counterpart of running `python tests/mock_http_server.py`.
//!
//! Usage:
//!   cargo run --example mock_server -- [port]      (default port 8000)
//!
//! Then point the CLI at it:
//!   claudesync config set claude_api_url http://127.0.0.1:8000/api
//!
//! Log in with any key starting with `sk-ant` (the mock validates the
//! cookie prefix only).

#[path = "../tests/common/mod.rs"]
mod common;

use std::sync::{Arc, Mutex};

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(8000);

    let docs = Arc::new(Mutex::new(vec![]));
    let server = common::MockServer::start_on(
        &format!("127.0.0.1:{port}"),
        common::claude_api_router(Arc::clone(&docs)),
    );

    println!(
        "Mock claude.ai API running on http://127.0.0.1:{}/api",
        server.port
    );
    println!("Point the CLI at it with:");
    println!(
        "  claudesync config set claude_api_url http://127.0.0.1:{}/api",
        server.port
    );
    println!("Press Ctrl+C to stop. Requests are logged below.\n");

    let mut seen = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let requests = server.requests.lock().unwrap();
        for (method, path, _) in requests.iter().skip(seen) {
            println!("{method} {path}");
        }
        seen = requests.len();
    }
}
