# ClaudeSync (Rust)

A Rust port of [ClaudeSync](https://github.com/jahwag/ClaudeSync), the CLI tool
that synchronizes local files with [Claude.ai](https://claude.ai) Projects.

The port mirrors the Python implementation feature-for-feature: same commands,
same configuration files, and the same on-disk formats — so it can be used as a
drop-in replacement on a machine where the Python version was previously set up.

## Building

```bash
cd rust
cargo build --release
# binary at target/release/claudesync
```

Install into your cargo bin directory:

```bash
cargo install --path rust
```

## Commands

| Command | Description |
|---|---|
| `claudesync auth login/logout/ls` | Manage session-key authentication |
| `claudesync organization ls/set` | Select the active organization |
| `claudesync project init/create/set/ls/archive/truncate` | Manage Claude.ai projects |
| `claudesync project submodule ls/create` | Detect and manage submodule projects |
| `claudesync project file ls` | List remote project files |
| `claudesync push [--category --uberproject --dryrun]` | Sync local files to the remote project |
| `claudesync embedding` | Pack (and optionally compress) the project into a single text blob |
| `claudesync chat pull/ls/rm/init/message` | Sync and manage chats and artifacts |
| `claudesync session ls/create/archive` | Manage Claude Code web sessions |
| `claudesync session environment ls`, `session branch ls` | List environments / connected repos |
| `claudesync config set/get/ls`, `config category ...` | Manage configuration and file categories |
| `claudesync schedule` | Install a cron entry for periodic syncing |
| `claudesync install-completion [shell]` | Print shell completion script |

## Compatibility with the Python version

- **Configuration**: reads/writes the same `~/.claudesync/config.json` and
  `<project>/.claudesync/config.local.json` files.
- **Session keys**: stored in `~/.claudesync/claude.ai.key`, encrypted with a
  Fernet key derived from your SSH private key via PBKDF2-HMAC-SHA256 with the
  same salt and iteration count — keys written by one implementation can be
  read by the other.
- **File filtering**: honors `.gitignore`, `.claudeignore`, file categories,
  the `max_file_size` limit, and the text-file heuristic, with the same
  excluded directories (`.git`, `claude_chats`, `.claudesync`, ...).
- **Compression**: all algorithms are supported (`zlib`, `bz2`, `lzma`,
  `brotli`, `dictionary`, `rle`, `huffman`, `lzw`, `pack`, `none`).

### Not ported (intentionally)

- `InMemoryConfigManager` — existed to support Python's in-process tests and
  submodule config cloning. Rust tests construct providers and sync managers
  directly, and submodule syncing uses `SyncManager::with_project` instead.
- `BaseProvider` / `BaseClaudeAIProvider` abstract classes — with a single
  concrete provider there is no need for a trait; the hierarchy is folded
  into `ClaudeProvider`.
- `tests/logging_test_case.py` — unittest logging scaffolding with no Rust
  equivalent needed (`cargo test` captures output natively).

### Intentional differences

- `claudesync schedule` installs the cron entry to run `claudesync push`
  (the Python version wrote `claudesync sync`, a command that does not exist).
- `claudesync upgrade` prints upgrade instructions instead of self-upgrading
  via pip.
- `install-completion` prints the clap-generated completion script to stdout
  rather than editing shell rc files.
- The Python `_unpack_files` kept a trailing ` ---` in unpacked file names;
  the Rust version strips the marker correctly.

## Development

```bash
cargo test     # unit + integration tests
cargo clippy
```

Unit tests cover the pure logic (compression roundtrips, config defaults,
session-key crypto and SSH key discovery, artifact extraction).

Integration tests run against a local mock of the claude.ai API
(`tests/common/mod.rs`, the Rust counterpart of the Python
`tests/mock_http_server.py`):

- `tests/mock_api.rs` — provider-level coverage: capability filtering, the
  full sync flow (upload/update/prune), chat conversations, Claude Code
  session creation (including branch auto-generation), SSE streaming for
  chats and session events, the `send_session_input` endpoint fallback, and
  403/429/5xx error mapping including retry-on-403.
- `tests/cli_happy_path.rs` — drives the compiled binary end to end (port of
  `test_happy_path.py` / `test_chat_happy_path.py`): login → organization
  set → project init → push → chat message, with HOME isolated to a temp
  directory and a stub `ssh-keygen` on PATH.

To test the built binary manually against a mock, point `claude_api_url` at a
local server (e.g. the Python mock):

```bash
python tests/mock_http_server.py &           # from the repository root
claudesync config set claude_api_url http://127.0.0.1:8000/api
```

Source layout mirrors the Python package:

| Rust module | Python origin |
|---|---|
| `src/config.rs` | `configmanager/` |
| `src/session_key.rs` | `session_key_manager.py` |
| `src/provider.rs` | `providers/` + `provider_factory.py` |
| `src/sync.rs` | `syncmanager.py` |
| `src/chat_sync.rs` | `chat_sync.py` |
| `src/compression.rs` | `compression.py` |
| `src/utils.rs` | `utils.py` |
| `src/cli/` | `cli/` |
