import Foundation

/// Reads and writes the same configuration files as the CLI:
/// `~/.claudesync/config.json` (global) and
/// `<project>/.claudesync/config.local.json` (per project), so projects set
/// up in either tool work in the other.
struct ClaudeConfig {
    static var globalDir: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".claudesync")
    }
    static var globalFile: URL { globalDir.appendingPathComponent("config.json") }

    var global: [String: Any]
    var projectFolder: URL?
    var local: [String: Any]

    static func localFile(in folder: URL) -> URL {
        folder.appendingPathComponent(".claudesync").appendingPathComponent("config.local.json")
    }

    private static func readJSON(at url: URL) -> [String: Any] {
        guard
            let data = try? Data(contentsOf: url),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return [:] }
        return object
    }

    static func load(projectFolder: URL?) -> ClaudeConfig {
        let global = readJSON(at: globalFile)
        let local = projectFolder.map { readJSON(at: localFile(in: $0)) } ?? [:]
        return ClaudeConfig(global: global, projectFolder: projectFolder, local: local)
    }

    /// Local value first, then global, then built-in default.
    func value(forKey key: String) -> Any? {
        local[key] ?? global[key]
    }

    func string(forKey key: String) -> String? {
        value(forKey: key) as? String
    }

    var apiBaseURL: URL {
        URL(string: string(forKey: "claude_api_url") ?? "https://claude.ai/api")!
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

    mutating func setGlobal(_ value: Any?, forKey key: String) {
        if let value { global[key] = value } else { global.removeValue(forKey: key) }
    }

    mutating func setLocal(_ value: Any?, forKey key: String) {
        if let value { local[key] = value } else { local.removeValue(forKey: key) }
    }

    func saveGlobal() throws {
        try FileManager.default.createDirectory(at: Self.globalDir,
                                                withIntermediateDirectories: true)
        let data = try JSONSerialization.data(withJSONObject: global,
                                              options: [.prettyPrinted, .sortedKeys])
        try data.write(to: Self.globalFile)
    }

    func saveLocal() throws {
        guard let folder = projectFolder else { return }
        let file = Self.localFile(in: folder)
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(),
                                                withIntermediateDirectories: true)
        let data = try JSONSerialization.data(withJSONObject: local,
                                              options: [.prettyPrinted, .sortedKeys])
        try data.write(to: file)
    }
}
