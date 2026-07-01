import SwiftUI

@main
struct CtxSyncBarApp: App {
    @StateObject private var controller = SyncController()

    var body: some Scene {
        MenuBarExtra {
            MenuView()
                .environmentObject(controller)
        } label: {
            Image(systemName: controller.statusSymbol)
        }
        .menuBarExtraStyle(.menu)

        Window("CtxSync Settings", id: "settings") {
            SettingsView()
                .environmentObject(controller)
                .frame(minWidth: 460)
        }
        .windowResizability(.contentSize)
        .defaultPosition(.center)
    }
}
