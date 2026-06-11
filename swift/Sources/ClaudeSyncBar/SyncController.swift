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

    // Account
    @Published var sessionActive = false

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

    private var sessionKey: String?
    private var watcher: FSWatcher?
    private var debounceTask: Task<Void, Never>?
    private var syncQueuedWhileBusy = false

    init() {
        restore()
    }

    // MARK: - Derived state

    var readyToSync: Bool {
        sessionActive && selectedOrgId != nil && selectedProjectId != nil
            && projectFolder != nil
    }

    var statusSymbol: String {
        if isSyncing { return "arrow.triangle.2.circlepath.icloud" }
        if lastError != nil { return "exclamationmark.icloud" }
        if !readyToSync { return "icloud.slash" }
        return autoSync ? "checkmark.icloud.fill" : "checkmark.icloud"
    }

    var statusText: String {
        if isSyncing { return progressText ?? "Syncing…" }
        if !sessionActive { return "Not logged in" }
        if !readyToSync { return "No project configured" }
        if let lastSync {
            let formatter = RelativeDateTimeFormatter()
            let when = formatter.localizedString(for: lastSync, relativeTo: Date())
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
            sessionActive = true
        }

        var config = ClaudeConfig.load(projectFolder: nil)
        if let path = config.string(forKey: "menubar_project_path") {
            let folder = URL(fileURLWithPath: path)
            if FileManager.default.fileExists(atPath: path) {
                projectFolder = folder
                config = ClaudeConfig.load(projectFolder: folder)
                selectedOrgId = config.string(forKey: "active_organization_id")
                selectedProjectId = config.string(forKey: "active_project_id")
                selectedProjectName = config.string(forKey: "active_project_name")
            }
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
            let expiry = Date().addingTimeInterval(30 * 24 * 3600)
            try KeychainStore.save(account: Self.provider, sessionKey: key, expiry: expiry)
            sessionKey = key
            sessionActive = true
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
        KeychainStore.delete(account: Self.provider)
        sessionKey = nil
        sessionActive = false
        organizations = []
        projects = []
        autoSync = false
    }

    // MARK: - Org / project / folder selection

    func refreshOrganizations() async {
        guard let client = client(), organizations.isEmpty else { return }
        do {
            organizations = try await client.organizations()
            lastError = nil
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

    /// Persists the selection in the same files the CLI uses.
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
        config.setGlobal(folder.path, forKey: "menubar_project_path")

        do {
            try config.saveLocal()
            try config.saveGlobal()
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
        guard readyToSync, let client = client(),
              let folder = projectFolder,
              let orgId = selectedOrgId,
              let projectId = selectedProjectId else { return }
        if isSyncing {
            syncQueuedWhileBusy = true
            return
        }

        isSyncing = true
        lastError = nil
        progressText = "Scanning files…"

        let config = ClaudeConfig.load(projectFolder: folder)
        let engine = SyncEngine(client: client,
                                organizationId: orgId,
                                projectId: projectId,
                                uploadDelay: config.uploadDelay,
                                pruneRemoteFiles: config.pruneRemoteFiles)
        let maxFileSize = config.maxFileSize

        do {
            let localFiles = await Task.detached(priority: .utility) {
                FileScanner.scan(root: folder, maxFileSize: maxFileSize)
            }.value

            let summary = try await engine.sync(localFiles: localFiles, root: folder) { message in
                Task { @MainActor [weak self] in
                    self?.progressText = message
                }
            }
            lastSync = Date()
            lastSummary = summary.text
        } catch ClaudeError.unauthorized {
            lastError = ClaudeError.unauthorized.errorDescription
            sessionActive = false
            sessionKey = nil
        } catch {
            lastError = error.localizedDescription
        }

        progressText = nil
        isSyncing = false

        if syncQueuedWhileBusy {
            syncQueuedWhileBusy = false
            Task { await performSync() }
        }
    }

    // MARK: - Watching

    private func updateWatcher() {
        watcher?.stop()
        watcher = nil
        guard autoSync, readyToSync, let folder = projectFolder else { return }

        watcher = FSWatcher(path: folder.path) { [weak self] in
            Task { @MainActor in
                self?.scheduleDebouncedSync()
            }
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
