import CryptoKit
import Foundation

/// Walks a project directory applying the same filters as the CLI:
/// excluded VCS/app directories, .gitignore and .claudeignore rules, the
/// max-file-size limit, the editor-backup (`~`) rule, and the null-byte
/// text-file heuristic. Returns relative path → MD5 of UTF-8 content.
enum FileScanner {
    static let excludedDirs: Set<String> = [
        ".git", ".svn", ".hg", ".bzr", "_darcs", "CVS", "claude_chats", ".claudesync",
    ]

    static func md5Hex(_ data: Data) -> String {
        Insecure.MD5.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    static func isTextFile(_ url: URL) -> Bool {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return false }
        defer { try? handle.close() }
        let sample = (try? handle.read(upToCount: 8192)) ?? Data()
        return !sample.contains(0)
    }

    static func scan(root: URL, maxFileSize: Int) -> [String: String] {
        let rootPath = root.standardizedFileURL.path
        let gitignore = IgnoreMatcher(contentsOf: root.appendingPathComponent(".gitignore"))
        let claudeignore = IgnoreMatcher(contentsOf: root.appendingPathComponent(".claudeignore"))

        func ignored(_ relPath: String, isDirectory: Bool) -> Bool {
            (gitignore?.isIgnored(relPath, isDirectory: isDirectory) ?? false)
                || (claudeignore?.isIgnored(relPath, isDirectory: isDirectory) ?? false)
        }

        var files: [String: String] = [:]
        let keys: [URLResourceKey] = [.isDirectoryKey, .isRegularFileKey, .fileSizeKey]
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: keys,
            options: []
        ) else { return files }

        for case let url as URL in enumerator {
            let path = url.standardizedFileURL.path
            guard path.hasPrefix(rootPath + "/") else { continue }
            let relPath = String(path.dropFirst(rootPath.count + 1))

            let values = try? url.resourceValues(forKeys: Set(keys))
            let isDirectory = values?.isDirectory ?? false

            if isDirectory {
                let name = url.lastPathComponent
                if excludedDirs.contains(name) || ignored(relPath, isDirectory: true) {
                    enumerator.skipDescendants()
                }
                continue
            }
            guard values?.isRegularFile ?? false else { continue }

            // Skip editor backup files
            if url.lastPathComponent.hasSuffix("~") { continue }
            if let size = values?.fileSize, size > maxFileSize { continue }
            if ignored(relPath, isDirectory: false) { continue }
            guard isTextFile(url) else { continue }

            guard
                let data = try? Data(contentsOf: url),
                String(data: data, encoding: .utf8) != nil
            else { continue }
            files[relPath] = md5Hex(data)
        }

        return files
    }
}
