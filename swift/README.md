# ClaudeSync Menu Bar App (Swift)

A native macOS menu bar companion for ClaudeSync: log in once, pick a
Claude.ai project and a local folder, then sync manually or automatically
whenever files change. Built with SwiftUI (`MenuBarExtra`), FSEvents, and the
Keychain — no Electron, no daemon, ~1,400 lines of Swift.

> **Status:** authored without access to a macOS toolchain (developed in a
> Linux container), so it has not been compile-tested. The code sticks to
> boring, well-trodden APIs, but expect possibly a few small fixes on first
> build. Requires macOS 13+.

## What it does

- **Menu bar status** — icon reflects state (idle / syncing / error /
  auto-sync armed); the menu shows the last sync time and result.
- **Sync Now** — the CLI's push algorithm: upload new files, delete-and-
  reupload changed files (MD5 comparison), prune remote files that no longer
  exist locally (honors `prune_remote_files`).
- **Auto-sync** — FSEvents watches the project folder and pushes ~2 seconds
  after changes settle. Events from `.git`, `.claudesync`, and `claude_chats`
  are ignored so the app doesn't react to its own writes.
- **Same filters as the CLI** — `.gitignore` + `.claudeignore` (common
  gitignore syntax: `*`, `?`, `**`, anchors, `!` negation), excluded VCS
  directories, `max_file_size`, editor backups, binary detection.
- **Login** — paste the `sessionKey` cookie (or one-click "Log In from
  Clipboard"); validated against the API before storing.
- **Launch at login** via `SMAppService` (requires the app bundle).

## Interop with the CLI

The app reads and writes the **same state** as the Rust CLI (and the original
Python tool):

- `~/.claudesync/config.json` and `<project>/.claudesync/config.local.json` —
  a project configured in the app works with `claudesync push`, and vice versa.
- The session key lives in the **same Keychain item** the Rust CLI uses
  (service `claudesync`, account `claude.ai`, identical JSON payload), so
  logging in once covers both. macOS will ask permission the first time each
  binary touches the item — "Always Allow".

Deliberately **not** in the app (use the CLI): chat/artifact pull, Claude Code
session management, compression modes, file categories, submodules, project
creation/archiving.

## Building

```bash
cd swift
swift run            # development: runs the menu bar app directly
```

For a proper installable app (menu-bar-only, supports launch-at-login):

```bash
./make-app.sh
mv ClaudeSync.app /Applications/
open /Applications/ClaudeSync.app
```

## First-run flow

1. Click the menu bar icon → **Settings…**
2. Copy the `sessionKey` cookie from claude.ai (browser dev tools →
   Application/Storage → Cookies) and click **Log In from Clipboard**.
3. Pick organization, Claude project, and the local folder; **Save Project
   Configuration**.
4. **Sync Now**, or flip on **Auto-sync when files change**.
