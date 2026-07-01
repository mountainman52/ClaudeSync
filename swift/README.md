# ctxsync Menu Bar App (Swift)

A native macOS menu bar companion for ctxsync: log in once, pick a
Claude.ai project and a local folder, then sync manually or automatically
whenever files change. Built with SwiftUI (`MenuBarExtra`), FSEvents, and the
Keychain — no Electron, no daemon, ~1,400 lines of Swift.

> **Renamed:** formerly the ClaudeSync menu bar app. The app migrates the
> global config dir and the Keychain item to the new names automatically and
> keeps honoring project-local `.claudesync` directories.


> **Status:** authored without access to a macOS toolchain (developed in a
> Linux container), so it has not been compile-tested. The code sticks to
> boring, well-trodden APIs, but expect possibly a few small fixes on first
> build. Requires macOS 13+.

## What it does

- **Menu bar status** — icon reflects state (idle / syncing / error /
  auto-sync armed); the menu shows the last sync time and result.
- **Sync Now** — the CLI's push algorithm: upload new files, delete-and-
  reupload changed files (MD5 comparison), prune remote files that no longer
  exist locally (honors `prune_remote_files`). Each request is retried on
  claude.ai's transient 403s, mirroring the CLI's `retry_on_403`.
- **Auto-sync** — FSEvents watches the project folder and pushes ~2 seconds
  after changes settle. Events in excluded VCS/app directories, gitignored
  paths (builds, `node_modules`, virtualenvs…), and submodules don't wake
  the sync.
- **Same filters as the CLI** — `.gitignore` + `.claudeignore` (common
  gitignore syntax: `*`, `?`, `**`, `[...]` classes, anchors, `!` negation),
  excluded VCS directories, registered submodule paths, `max_file_size`,
  editor backups, binary detection.
- **Safety** — an unreadable project folder aborts the sync instead of being
  mistaken for an empty project, and pruning refuses to delete *every*
  remote file when the local scan comes back empty.
- **Login** — paste the `sessionKey` cookie (or one-click "Log In from
  Clipboard"); validated against the API before storing. A re-login with the
  same cookie keeps the expiry you gave the CLI.
- **Launch at login** via `SMAppService` (requires the app bundle).

## Interop with the CLI

The app reads and writes the **same state** as the Rust CLI (and the original
Python tool):

- `~/.ctxsync/config.json` and `<project>/.ctxsync/config.local.json` —
  a project configured in the app works with `ctxsync push`, and vice versa.
- The session key lives in the **same Keychain item** the Rust CLI uses
  (service `ctxsync`, account `claude.ai`, identical JSON payload), so
  logging in once covers both. macOS will ask permission the first time each
  binary touches the item — "Always Allow".

Deliberately **not** in the app (use the CLI): chat/artifact pull, Claude Code
session management, compression modes, file categories, project
creation/archiving. Submodule paths are *excluded* from the app's sync (like
the CLI's parent-project push); syncing submodules to their own projects
remains CLI-only. If a project sets `compression_algorithm` or
`default_sync_category`, the app refuses to sync it rather than fight the CLI
over the remote doc set.

## Building

```bash
cd swift
swift run            # development: runs the menu bar app directly
```

For a proper installable app (menu-bar-only, supports launch-at-login):

```bash
./make-app.sh
mv CtxSync.app /Applications/
open /Applications/CtxSync.app
```

## First-run flow

1. Click the menu bar icon → **Settings…**
2. Copy the `sessionKey` cookie from claude.ai (browser dev tools →
   Application/Storage → Cookies) and click **Log In from Clipboard**.
3. Pick organization, Claude project, and the local folder; **Save Project
   Configuration**.
4. **Sync Now**, or flip on **Auto-sync when files change**.
