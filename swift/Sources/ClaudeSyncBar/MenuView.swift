import SwiftUI

struct MenuView: View {
    @EnvironmentObject var controller: SyncController
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            Text(controller.statusText)

            if let error = controller.lastError {
                Text("⚠︎ \(error)")
            }

            Divider()

            Button("Sync Now") {
                controller.syncNow()
            }
            .disabled(!controller.readyToSync || controller.isSyncing)
            .keyboardShortcut("s")

            Toggle("Auto-sync on change", isOn: $controller.autoSync)
                .disabled(!controller.readyToSync)

            Divider()

            if let name = controller.selectedProjectName, !name.isEmpty {
                Button("Open '\(name)' on Claude.ai") {
                    controller.openProjectInBrowser()
                }
            }

            Button("Settings…") {
                openWindow(id: "settings")
                NSApp.activate(ignoringOtherApps: true)
            }
            .keyboardShortcut(",")

            Divider()

            Button("Quit ClaudeSync") {
                NSApp.terminate(nil)
            }
            .keyboardShortcut("q")
        }
    }
}
