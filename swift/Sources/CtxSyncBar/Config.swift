import Foundation

/// Reads and writes the same configuration files as the CLI:
/// `~/.ctxsync/config.json` (global) and
/// `<project>/.ctxsync/config.local.json` (per project), so projects set
/// up in either tool work in the other.
///
/// Configuration from the tool's former name (ClaudeSync) is honored: the
/// global `~/.claudesync` directory is copied to `~/.ctxsync` on first run,
/// and project-local `.claudesync` directories keep working as-is.
struct ClaudeConfig {
    static let localDirName = ".ctxsync"
    static let legacyLocalDirName = ".claudesync"

    static var globalDir: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(localDirName)
    }
    static var globalFile: URL { globalDir.appendingPathComponent("config.json") }

    var global: [String: Any]
    var projectFolder: URL?
    var local: [String: Any]

    /// The project's config dir name: `.ctxsync`, or the legacy
    /// `.claudesync` when that's what the project already has.
    static func configDirName(in folder: URL) -> String {
        let fm = FileManager.default
        if fm.fileExists(atPath: folder.appendingPathComponent(localDirName).path) {
            return localDirName
        }
        if fm.fileExists(atPath: folder.appendingPathComponent(legacyLocalDirName).path) {
            return legacyLocalDirName
        }
        return localDirName
    }

    static func localFile(in folder: URL) -> URL {
        folder.appendingPathComponent(configDirName(in: folder))
            .appendingPathComponent("config.local.json")
    }

    /// First run after the rename: bring over the old global config so
    /// nothing breaks. Best-effort.
    private static func migrateLegacyGlobalDir() {
        let fm = FileManager.default
        let legacyDir = fm.homeDirectoryForCurrentUser
            .appendingPathComponent(legacyLocalDirName)
        guard !fm.fileExists(atPath: globalDir.path),
              fm.fileExists(atPath: legacyDir.path) else { return }
        try? fm.copyItem(at: legacyDir, to: globalDir)
    }

    private static func readJSON(at url: URL) -> [String: Any] {
        guard
            let data = try? Data(contentsOf: url),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return [:] }
        return object
    }

    static func load(projectFolder: URL?) -> ClaudeConfig {
        migrateLegacyGlobalDir()
        let global = readJSON(at: globalFile)
        let local = projectFolder.map { readJSON(at: localFile(in: $0)) } ?? [:]
        return ClaudeConfig(global: global, projectFolder: projectFolder, local: local)
    }

    /// Local value first, then global — but matching the CLI's Python-derived
    /// merge, a *falsy* local value (false, 0, "", null, [], {}) falls
    /// through to the global one.
    func value(forKey key: String) -> Any? {
        if let localValue = local[key], Self.isTruthy(localValue) { return localValue }
        return global[key]
    }

    static func isTruthy(_ value: Any) -> Bool {
        switch value {
        case is NSNull:
            return false
        case let number as NSNumber:
            // Covers Bool (false bridges to 0) and all numeric zeros.
            return number.doubleValue != 0
        case let string as String:
            return !string.isEmpty
        case let array as [Any]:
            return !array.isEmpty
        case let dictionary as [String: Any]:
            return !dictionary.isEmpty
        default:
            return true
        }
    }

    func string(forKey key: String) -> String? {
        value(forKey: key) as? String
    }

    var apiBaseURL: URL {
        let fallback = URL(string: "https://claude.ai/api")!
        guard let raw = string(forKey: "claude_api_url"), let url = URL(string: raw) else {
            return fallback
        }
        return url
    }

    var maxFileSize: Int {
        (value(forKey: "max_file_size") as? Int) ?? 32 * 1024
    }

    var uploadDelay: TimeInterval {
        (value(forKey: "upload_delay") as? Double) ?? 0.5
    }

    var pruneRemoteFiles: Bool {
        (value(forKey: "prune_remote_files") as? Bool) ?? true
    }

    var compressionAlgorithm: String {
        string(forKey: "compression_algorithm") ?? "none"
    }

    var defaultSyncCategory: String? {
        string(forKey: "default_sync_category")
    }

    /// Relative paths of registered submodules; the CLI syncs these to their
    /// own Claude projects and excludes them from the parent's walk.
    var submodulePaths: [String] {
        guard let list = value(forKey: "submodules") as? [[String: Any]] else { return [] }
        return list.compactMap { $0["relative_path"] as? String }
    }

    mutating func setGlobal(_ value: Any?, forKey key: String) {
        if let value { global[key] = value } else { global.removeValue(forKey: key) }
    }

    mutating func setLocal(_ value: Any?, forKey key: String) {
        if let value { local[key] = value } else { local.removeValue(forKey: key) }
    }

    private func write(_ object: [String: Any], to url: URL) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(),
                                                withIntermediateDirectories: true)
        let data = try JSONSerialization.data(withJSONObject: object,
                                              options: [.prettyPrinted, .sortedKeys])
        try data.write(to: url)
    }

    func saveGlobal() throws {
        try write(global, to: Self.globalFile)
    }

    func saveLocal() throws {
        guard let folder = projectFolder else { return }
        try write(local, to: Self.localFile(in: folder))
    }
}
