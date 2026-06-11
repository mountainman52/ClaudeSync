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

/// The CLI's sync algorithm: upload new files, delete-and-reupload changed
/// files (each request retried independently), prune remote-only files.
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

    func sync(localFiles: [String: String], root: URL,
              progress: @escaping @Sendable (String) -> Void) async throws -> SyncSummary {
        progress("Fetching remote file list…")
        let remoteFiles = try await client.listFiles(organizationId: organizationId,
                                                     projectId: projectId)
        var remoteByName: [String: RemoteFile] = [:]
        for file in remoteFiles where remoteByName[file.fileName] == nil {
            remoteByName[file.fileName] = file
        }
        var remoteToDelete = Set(remoteFiles.map(\.fileName))

        var summary = SyncSummary()
        let total = localFiles.count
        var done = 0

        for (relPath, localHash) in localFiles.sorted(by: { $0.key < $1.key }) {
            done += 1
            progress("Syncing \(done)/\(total): \(relPath)")

            if let remote = remoteByName[relPath] {
                remoteToDelete.remove(relPath)
                let remoteHash = FileScanner.md5Hex(Data(remote.content.utf8))
                if remoteHash == localHash {
                    summary.unchanged += 1
                    continue
                }
                let content = try String(contentsOf: root.appendingPathComponent(relPath),
                                         encoding: .utf8)
                try await client.deleteFile(organizationId: organizationId,
                                            projectId: projectId,
                                            uuid: remote.uuid)
                try await client.uploadFile(organizationId: organizationId,
                                            projectId: projectId,
                                            fileName: relPath,
                                            content: content)
                summary.updated += 1
                await pause()
            } else {
                let content = try String(contentsOf: root.appendingPathComponent(relPath),
                                         encoding: .utf8)
                try await client.uploadFile(organizationId: organizationId,
                                            projectId: projectId,
                                            fileName: relPath,
                                            content: content)
                summary.uploaded += 1
                await pause()
            }
        }

        if pruneRemoteFiles {
            for name in remoteToDelete.sorted() {
                guard let remote = remoteByName[name] else { continue }
                progress("Removing remote \(name)…")
                try await client.deleteFile(organizationId: organizationId,
                                            projectId: projectId,
                                            uuid: remote.uuid)
                summary.deleted += 1
                await pause()
            }
        }

        return summary
    }
}
