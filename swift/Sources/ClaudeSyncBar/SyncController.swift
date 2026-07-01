import AppKit
import Foundation
import ServiceManagement
import SwiftUI

/// App state machine: login, org/project/folder selection, manual and
/// FSEvents-triggered sync, and menu bar status.
@MainActor
final class SyncController: ObservableObject {
    static let provider = "claude.ai"
    private static let autoSyncDefaultsKey = "autoSyncEnabled"
    private static let projectPathDefaultsKey = "projectPath"

    // Account. `sessionKey` is the single source of truth for login state.
    @Published private var sessionKey: String?
    var sessionActive: Bool { sessionKey != nil }

    // Selection state
    @Published var organizations: [Organization] = []
    @Published var projects: [Project] = []
    @Published var selectedOrgId: String?
    @Published var selectedProjectId: String?
    @Published var selectedProjectName: String?
    @Published var projectFolder: URL?

    // Sync state
    @Published var isSyncing = false
    @Published var lastSync: Date?
    @Published var lastSummary: String?
    @Published var lastError: String?
    @Published var progressText: String?

    @Published var autoSync = false {
        didSet {
            UserDefaults.standard.set(autoSync, forKey: Self.autoSyncDefaultsKey)
            updateWatcher()
        }
    }

    private var watcher: FSWatcher?
    private var debounceTask: Task<Void, Never>?
    private var syncQueuedWhileBusy = false

    private static let relativeFormatter = RelativeDateTimeFormatter()

    init() {
        restore()
    }

    // MARK: - Derived state

    var readyToSync: Bool {
        sessionActive && selectedOrgId != nil && selectedProjectId != nil
            && projectFolder != nil
    }

    var statusSymbol: String {
        if isSyncing { return "arrow.clockwise.icloud" }
        if lastError != nil { return "exclamationmark.icloud" }
        if !readyToSync { return "icloud.slash" }
        return autoSync ? "checkmark.icloud.fill" : "checkmark.icloud"
    }

    var statusText: String {
        if isSyncing { return progressText ?? "Syncing…" }
        if !sessionActive { return "Not logged in" }
        if !readyToSync { return "No project configured" }
        if let lastSync {
            let when = Self.relativeFormatter.localizedString(for: lastSync, relativeTo: Date())
            let summary = lastSummary.map { " (\($0))" } ?? ""
            return "Synced \(when)\(summary)"
        }
        return "Ready to sync"
    }

    private func client() -> ClaudeClient? {
        guard let sessionKey else { return nil }
        let config = ClaudeConfig.load(projectFolder: projectFolder)
        return ClaudeClient(baseURL: config.apiBaseURL, sessionKey: sessionKey)
    }

    // MARK: - Startup

    private func restore() {
        autoSync = UserDefaults.standard.bool(forKey: Self.autoSyncDefaultsKey)

        if let stored = try? KeychainStore.load(account: Self.provider) {
            sessionKey = stored.key
        }

        if let path = UserDefaults.standard.string(forKey: Self.projectPathDefaultsKey),
           FileManager.default.fileExists(atPath: path) {
            let folder = URL(fileURLWithPath: path)
            projectFolder = folder
            let config = ClaudeConfig.load(projectFolder: folder)
            selectedOrgId = config.string(forKey: "active_organization_id")
            selectedProjectId = config.string(forKey: "active_project_id")
            selectedProjectName = config.string(forKey: "active_project_name")
        }
        updateWatcher()
    }

    // MARK: - Account

    func login(with rawKey: String) async -> Bool {
        let key = rawKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard key.hasPrefix("sk-ant") else {
            lastError = "Invalid sessionKey format. Must start with 'sk-ant'."
            return false
        }
        let config = ClaudeConfig.load(projectFolder: projectFolder)
        let candidate = ClaudeClient(baseURL: config.apiBaseURL, sessionKey: key)
        do {
            let orgs = try await candidate.organizations()
            guard !orgs.isEmpty else {
                lastError = "No organizations with the required capabilities found."
                return false
            }
            // Re-validating the same cookie (e.g. one stored via the CLI with
            // a user-supplied expiry): keep that expiry rather than clobbering
            // the shared Keychain payload with an invented one.
            let expiry: Date
            if let stored = KeychainStore.stored(account: Self.provider),
               stored.key == key, stored.expiry > Date() {
                expiry = stored.expiry
            } else {
                expiry = Date().addingTimeInterval(30 * 24 * 3600)
            }
            try KeychainStore.save(account: Self.provider, sessionKey: key, expiry: expiry)
            sessionKey = key
            organizations = orgs
            lastError = nil
            return true
        } catch {
            lastError = error.localizedDescription
            return false
        }
    }

    func loginFromClipboard() async -> Bool {
        guard let pasted = NSPasteboard.general.string(forType: .string),
              !pasted.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            lastError = "Clipboard is empty. Copy the sessionKey cookie value first."
            return false
        }
        return await login(with: pasted)
    }

    func logout() {
        clearSession()
        lastError = nil
    }

    private func clearSession() {
        KeychainStore.delete(account: Self.provider)
        sessionKey = nil
        organizations = []
        projects = []
        autoSync = false
    }

    /// One place for 401s: drop the rejected key from memory AND the shared
    /// Keychain item, so a relaunch or the CLI doesn't resurrect it.
    private func handleUnauthorized() {
        clearSession()
        lastError = ClaudeError.unauthorized.errorDescription
    }

    // MARK: - Org / project / folder selection

    func refreshOrganizations() async {
        guard let client = client(), organizations.isEmpty else { return }
        do {
            organizations = try await client.organizations()
            lastError = nil
        } catch ClaudeError.unauthorized {
            handleUnauthorized()
        } catch {
            lastError = error.localizedDescription
        }
    }

    func refreshProjects() async {
        guard let client = client(), let orgId = selectedOrgId else {
            projects = []
            return
        }
        do {
            projects = try await client.projects(organizationId: orgId)
            lastError = nil
        } catch ClaudeError.unauthorized {
            handleUnauthorized()
        } catch {
            lastError = error.localizedDescription
        }
    }

    func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.message = "Choose the local project folder to sync"
        if panel.runModal() == .OK, let url = panel.url {
            projectFolder = url
            updateWatcher()
        }
    }

    /// Persists the selection in the same local config file the CLI uses;
    /// the app-private "last project" pointer stays in UserDefaults.
    func saveProjectConfiguration() {
        guard let folder = projectFolder,
              let orgId = selectedOrgId,
              let projectId = selectedProjectId else { return }
        let projectName = projects.first(where: { $0.id == projectId })?.name
            ?? selectedProjectName ?? ""
        selectedProjectName = projectName

        var config = ClaudeConfig.load(projectFolder: folder)
        config.setLocal(Self.provider, forKey: "active_provider")
        config.setLocal(folder.path, forKey: "local_path")
        config.setLocal(orgId, forKey: "active_organization_id")
        config.setLocal(projectId, forKey: "active_project_id")
        config.setLocal(projectName, forKey: "active_project_name")
        UserDefaults.standard.set(folder.path, forKey: Self.projectPathDefaultsKey)

        do {
            try config.saveLocal()
            lastError = nil
        } catch {
            lastError = "Failed to save configuration: \(error.localizedDescription)"
        }
        updateWatcher()
    }

    func openProjectInBrowser() {
        guard let projectId = selectedProjectId,
              let url = URL(string: "https://claude.ai/project/\(projectId)") else { return }
        NSWorkspace.shared.open(url)
    }

    // MARK: - Sync

    func syncNow() {
        Task { await performSync() }
    }

    private func performSync() async {
        guard readyToSync, let sessionKey,
              let folder = projectFolder,
              let orgId = selectedOrgId,
              let projectId = selectedProjectId else { return }
        if isSyncing {
            syncQueuedWhileBusy = true
            return
        }

        let config = ClaudeConfig.load(projectFolder: folder)
        // Refuse configurations only the CLI implements: syncing anyway would
        // fight the CLI over the remote doc set.
        if config.compressionAlgorithm != "none" {
            lastError = "This project uses compression_algorithm="
                + "\(config.compressionAlgorithm), which only the CLI supports. "
                + "Sync with `claudesync push` or set it to \"none\"."
            return
        }
        if let category = config.defaultSyncCategory {
            lastError = "This project sets default_sync_category=\(category), "
                + "which only the CLI applies. Syncing here would push files "
                + "outside the category."
            return
        }

        isSyncing = true
        lastError = nil
        progressText = "Scanning files…"

        let client = ClaudeClient(baseURL: config.apiBaseURL, sessionKey: sessionKey)
        let engine = SyncEngine(client: client,
                                organizationId: orgId,
                                projectId: projectId,
                                uploadDelay: config.uploadDelay,
                                pruneRemoteFiles: config.pruneRemoteFiles)
        let maxFileSize = config.maxFileSize
        let submodulePaths = Set(config.submodulePaths)

        do {
            let localFiles = try await Task.detached(priority: .utility) {
                try FileScanner.scan(root: folder, maxFileSize: maxFileSize,
                                     excludedRelativePaths: submodulePaths)
            }.value

            let summary = try await engine.sync(localFiles: localFiles, root: folder) { message in
                Task { @MainActor [weak self] in
                    self?.progressText = message
                }
            }
            lastSync = Date()
            lastSummary = summary.text
        } catch ClaudeError.unauthorized {
            handleUnauthorized()
        } catch {
            lastError = error.localizedDescription
        }

        progressText = nil
        isSyncing = false

        if syncQueuedWhileBusy {
            syncQueuedWhileBusy = false
            // Only chase the queued request when this pass succeeded — a
            // persistent error plus watcher noise must not become a tight
            // fail-retry loop.
            if lastError == nil {
                Task { await performSync() }
            }
        }
    }

    // MARK: - Watching

    private func updateWatcher() {
        watcher?.stop()
        watcher = nil
        guard autoSync, readyToSync, let folder = projectFolder else {
            debounceTask?.cancel()
            syncQueuedWhileBusy = false
            return
        }

        watcher = FSWatcher(path: folder.path,
                            isRelevant: Self.makeRelevanceFilter(root: folder)) { [weak self] in
            Task { @MainActor in
                self?.scheduleDebouncedSync()
            }
        }
    }

    /// One definition of "project content", shared with the scanner: events
    /// in excluded VCS/app directories, ignored paths (builds, node_modules,
    /// virtualenvs…), or submodules must not wake the sync.
    nonisolated private static func makeRelevanceFilter(root: URL) -> @Sendable (String) -> Bool {
        let rootPath = root.standardizedFileURL.path
        let gitignore = IgnoreMatcher(contentsOf: root.appendingPathComponent(".gitignore"))
        let claudeignore = IgnoreMatcher(contentsOf: root.appendingPathComponent(".claudeignore"))
        let excludedDirs = FileScanner.excludedDirs
        let submodules = ClaudeConfig.load(projectFolder: root).submodulePaths

        return { path in
            guard path.hasPrefix(rootPath + "/") else { return true }
            let relPath = String(path.dropFirst(rootPath.count + 1))
            let components = relPath.split(separator: "/").map(String.init)
            if components.contains(where: excludedDirs.contains) { return false }
            if submodules.contains(where: { relPath == $0 || relPath.hasPrefix($0 + "/") }) {
                return false
            }
            guard gitignore != nil || claudeignore != nil else { return true }
            // Check the path and every ancestor directory against the ignore
            // rules; FSEvents reports the leaf, but a dir-only rule like
            // "build/" is what actually excludes its contents.
            var prefix = ""
            for (index, component) in components.enumerated() {
                prefix = prefix.isEmpty ? component : prefix + "/" + component
                let isDirectory = index < components.count - 1
                if gitignore?.isIgnored(prefix, isDirectory: isDirectory) == true { return false }
                if claudeignore?.isIgnored(prefix, isDirectory: isDirectory) == true { return false }
            }
            return true
        }
    }

    private func scheduleDebouncedSync() {
        debounceTask?.cancel()
        debounceTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            guard !Task.isCancelled else { return }
            await self?.performSync()
        }
    }

    // MARK: - Launch at login

    var launchAtLogin: Bool {
        SMAppService.mainApp.status == .enabled
    }

    func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            objectWillChange.send()
        } catch {
            lastError = "Launch at login requires running from the app bundle "
                + "(see make-app.sh): \(error.localizedDescription)"
        }
    }
}
