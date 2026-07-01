import CryptoKit
import Foundation

/// Walks a project directory applying the same filters as the CLI:
/// excluded VCS/app directories, .gitignore and .claudeignore rules,
/// registered submodule paths, the max-file-size limit, the editor-backup
/// (`~`) rule, and the null-byte text-file heuristic.
/// Returns relative path → MD5 of UTF-8 content.
enum FileScanner {
    static let excludedDirs: Set<String> = [
        ".git", ".svn", ".hg", ".bzr", "_darcs", "CVS", "claude_chats", ".ctxsync",
        ".claudesync",  // legacy name, still honored
    ]

    enum ScanError: LocalizedError {
        case unreadable(String)

        var errorDescription: String? {
            switch self {
            case .unreadable(let path):
                return "Cannot read the project folder at \(path) "
                    + "(missing, unmounted, or permission denied). Sync aborted."
            }
        }
    }

    static func md5Hex(_ data: Data) -> String {
        Insecure.MD5.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    static func scan(root: URL, maxFileSize: Int,
                     excludedRelativePaths: Set<String> = []) throws -> [String: String] {
        let rootPath = root.standardizedFileURL.path
        let gitignore = IgnoreMatcher(contentsOf: root.appendingPathComponent(".gitignore"))
        let claudeignore = IgnoreMatcher(contentsOf: root.appendingPathComponent(".claudeignore"))

        func ignored(_ relPath: String, isDirectory: Bool) -> Bool {
            (gitignore?.isIgnored(relPath, isDirectory: isDirectory) ?? false)
                || (claudeignore?.isIgnored(relPath, isDirectory: isDirectory) ?? false)
        }

        // A failed enumeration must be an error, never an empty result: the
        // caller prunes remote files that are missing locally, so mistaking
        // an unreadable folder for an empty project would wipe the remote.
        var rootIsDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: rootPath, isDirectory: &rootIsDirectory),
              rootIsDirectory.boolValue else {
            throw ScanError.unreadable(rootPath)
        }

        var files: [String: String] = [:]
        let keys: [URLResourceKey] = [.isDirectoryKey, .isRegularFileKey, .fileSizeKey]
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: keys,
            options: []
        ) else { throw ScanError.unreadable(rootPath) }

        for case let url as URL in enumerator {
            let path = url.standardizedFileURL.path
            guard path.hasPrefix(rootPath + "/") else { continue }
            let relPath = String(path.dropFirst(rootPath.count + 1))

            let values = try? url.resourceValues(forKeys: Set(keys))
            let isDirectory = values?.isDirectory ?? false

            if isDirectory {
                let name = url.lastPathComponent
                if excludedDirs.contains(name)
                    || excludedRelativePaths.contains(relPath)
                    || ignored(relPath, isDirectory: true) {
                    enumerator.skipDescendants()
                }
                continue
            }
            guard values?.isRegularFile ?? false else { continue }

            // Skip editor backup files
            if url.lastPathComponent.hasSuffix("~") { continue }
            if let size = values?.fileSize, size > maxFileSize { continue }
            if ignored(relPath, isDirectory: false) { continue }

            guard
                let data = try? Data(contentsOf: url),
                !data.prefix(8192).contains(0),
                String(data: data, encoding: .utf8) != nil
            else { continue }
            files[relPath] = md5Hex(data)
        }

        return files
    }
}
