import SwiftUI

@main
struct ClaudeSyncBarApp: App {
    @StateObject private var controller = SyncController()

    var body: some Scene {
        MenuBarExtra {
            MenuView()
                .environmentObject(controller)
        } label: {
            Image(systemName: controller.statusSymbol)
        }
        .menuBarExtraStyle(.menu)

        Window("ClaudeSync Settings", id: "settings") {
            SettingsView()
                .environmentObject(controller)
                .frame(minWidth: 460)
        }
        .windowResizability(.contentSize)
        .defaultPosition(.center)
    }
}
