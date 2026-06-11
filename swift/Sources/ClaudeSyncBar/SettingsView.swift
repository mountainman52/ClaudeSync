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
            await controller.refreshOrganizations()
            await controller.refreshProjects()
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
                        loggingIn = true
                        Task {
                            _ = await controller.loginFromClipboard()
                            loggingIn = false
                        }
                    }
                    Button("Log In") {
                        loggingIn = true
                        Task {
                            if await controller.login(with: sessionKeyInput) {
                                sessionKeyInput = ""
                            }
                            loggingIn = false
                        }
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(sessionKeyInput.isEmpty)
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
