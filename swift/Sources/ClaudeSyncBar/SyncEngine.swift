import Foundation

struct SyncSummary {
    var uploaded = 0
    var updated = 0
    var deleted = 0
    var unchanged = 0

    var text: String {
        var parts: [String] = []
        if uploaded > 0 { parts.append("\(uploaded) new") }
        if updated > 0 { parts.append("\(updated) updated") }
        if deleted > 0 { parts.append("\(deleted) pruned") }
        if parts.isEmpty { parts.append("up to date") }
        return parts.joined(separator: ", ")
    }
}

enum SyncError: LocalizedError {
    case refusingToPruneEverything(Int)

    var errorDescription: String? {
        switch self {
        case .refusingToPruneEverything(let count):
            return "No local files found but the remote project has \(count) — "
                + "refusing to prune everything. If the project really is empty, "
                + "remove the remote files on claude.ai."
        }
    }
}

/// The CLI's sync algorithm: upload new files, delete-and-reupload changed
/// files, prune remote-only files. Each request is retried independently on
/// transient 403s, so a delete+reupload pair can't half-complete because of
/// one blip.
struct SyncEngine {
    let client: ClaudeClient
    let organizationId: String
    let projectId: String
    let uploadDelay: TimeInterval
    let pruneRemoteFiles: Bool

    private func pause() async {
        if uploadDelay > 0 {
            try? await Task.sleep(nanoseconds: UInt64(uploadDelay * 1_000_000_000))
        }
    }

    /// claude.ai intermittently returns 403s mid-sync; mirror the CLI's
    /// retry_on_403 (3 attempts, 1s apart).
    private func withRetry<T>(_ operation: () async throws -> T) async throws -> T {
        var attempt = 0
        while true {
            do {
                return try await operation()
            } catch ClaudeError.forbidden where attempt < 2 {
                attempt += 1
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
    }

    func sync(localFiles: [String: String], root: URL,
              progress: @escaping @Sendable (String) -> Void) async throws -> SyncSummary {
        progress("Fetching remote file list…")
        let remoteFiles = try await withRetry {
            try await client.listFiles(organizationId: organizationId, projectId: projectId)
        }
        // First match wins for duplicate names, mirroring the CLI.
        var remoteByName: [String: RemoteFile] = [:]
        for file in remoteFiles where remoteByName[file.fileName] == nil {
            remoteByName[file.fileName] = file
        }

        var summary = SyncSummary()
        let total = localFiles.count
        var done = 0

        for (relPath, localHash) in localFiles.sorted(by: { $0.key < $1.key }) {
            done += 1
            let remote = remoteByName[relPath]
            if let remote,
               FileScanner.md5Hex(Data(remote.content.utf8)) == localHash {
                summary.unchanged += 1
                continue
            }

            progress("Syncing \(done)/\(total): \(relPath)")
            let content = try String(contentsOf: root.appendingPathComponent(relPath),
                                     encoding: .utf8)
            if let remote {
                try await withRetry {
                    try await client.deleteFile(organizationId: organizationId,
                                                projectId: projectId,
                                                uuid: remote.uuid)
                }
                summary.updated += 1
            } else {
                summary.uploaded += 1
            }
            try await withRetry {
                try await client.uploadFile(organizationId: organizationId,
                                            projectId: projectId,
                                            fileName: relPath,
                                            content: content)
            }
            await pause()
        }

        if pruneRemoteFiles {
            let staleNames = remoteByName.keys.filter { localFiles[$0] == nil }
            // An empty scan against a populated project is far more likely a
            // failure upstream than a deliberate wipe.
            if localFiles.isEmpty && !staleNames.isEmpty {
                throw SyncError.refusingToPruneEverything(staleNames.count)
            }
            for name in staleNames.sorted() {
                guard let remote = remoteByName[name] else { continue }
                progress("Removing remote \(name)…")
                try await withRetry {
                    try await client.deleteFile(organizationId: organizationId,
                                                projectId: projectId,
                                                uuid: remote.uuid)
                }
                summary.deleted += 1
                await pause()
            }
        }

        return summary
    }
}
