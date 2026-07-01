import SwiftUI

struct SettingsView: View {
    @EnvironmentObject var controller: SyncController

    @State private var sessionKeyInput = ""
    @State private var loggingIn = false

    var body: some View {
        Form {
            accountSection
            if controller.sessionActive {
                projectSection
                behaviorSection
            }
            if let error = controller.lastError {
                Section {
                    Text(error)
                        .foregroundColor(.red)
                        .textSelection(.enabled)
                }
            }
        }
        .formStyle(.grouped)
        .padding(.bottom, 8)
        .task {
            async let orgs: Void = controller.refreshOrganizations()
            async let projects: Void = controller.refreshProjects()
            _ = await (orgs, projects)
        }
    }

    private func runLogin(_ operation: @escaping () async -> Bool) {
        loggingIn = true
        Task {
            if await operation() {
                sessionKeyInput = ""
            }
            loggingIn = false
        }
    }

    private var accountSection: some View {
        Section("Account") {
            if controller.sessionActive {
                HStack {
                    Label("Logged in to claude.ai", systemImage: "person.crop.circle.badge.checkmark")
                    Spacer()
                    Button("Log Out", role: .destructive) {
                        controller.logout()
                    }
                }
            } else {
                SecureField("sessionKey (sk-ant-…)", text: $sessionKeyInput)
                    .textFieldStyle(.roundedBorder)
                Text("Copy the `sessionKey` cookie from claude.ai using your browser's developer tools.")
                    .font(.caption)
                    .foregroundColor(.secondary)
                HStack {
                    Button("Log In from Clipboard") {
                        runLogin { await controller.loginFromClipboard() }
                    }
                    .disabled(loggingIn)
                    Button("Log In") {
                        runLogin { await controller.login(with: sessionKeyInput) }
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(sessionKeyInput.isEmpty || loggingIn)
                    if loggingIn {
                        ProgressView().controlSize(.small)
                    }
                }
            }
        }
    }

    private var projectSection: some View {
        Section("Project") {
            Picker("Organization", selection: $controller.selectedOrgId) {
                Text("Select…").tag(String?.none)
                ForEach(controller.organizations) { org in
                    Text(org.name).tag(String?.some(org.id))
                }
            }
            .onChange(of: controller.selectedOrgId) { _ in
                // A project id from the previous org must not survive the
                // switch — it would be saved and synced against the wrong org.
                controller.selectedProjectId = nil
                controller.selectedProjectName = nil
                Task { await controller.refreshProjects() }
            }

            Picker("Claude Project", selection: $controller.selectedProjectId) {
                Text("Select…").tag(String?.none)
                ForEach(controller.projects) { project in
                    Text(project.name).tag(String?.some(project.id))
                }
            }

            HStack {
                Text("Local Folder")
                Spacer()
                Text(controller.projectFolder?.path ?? "None selected")
                    .foregroundColor(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Button("Choose…") {
                    controller.chooseFolder()
                }
            }

            Button("Save Project Configuration") {
                controller.saveProjectConfiguration()
            }
            .disabled(controller.selectedOrgId == nil
                || controller.selectedProjectId == nil
                || controller.projectFolder == nil)
        }
    }

    private var behaviorSection: some View {
        Section("Behavior") {
            Toggle("Auto-sync when files change", isOn: $controller.autoSync)
                .disabled(!controller.readyToSync)
            Toggle("Launch at login", isOn: Binding(
                get: { controller.launchAtLogin },
                set: { controller.setLaunchAtLogin($0) }
            ))
            Text("Auto-sync watches the project folder with FSEvents and pushes a couple of seconds after changes settle.")
                .font(.caption)
                .foregroundColor(.secondary)
        }
    }
}
