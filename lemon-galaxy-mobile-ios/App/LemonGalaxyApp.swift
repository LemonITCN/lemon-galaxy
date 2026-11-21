import SwiftUI
import GalaxyKit

@main
struct LemonGalaxyApp: App {
    @StateObject private var app = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(app)
                .environmentObject(app.account)
                .environmentObject(app.settings)
                .environmentObject(app.ai)
                .environmentObject(app.transfers)
                .environmentObject(app.sessions)
        }
        .onChange(of: scenePhase) { app.scenePhaseChanged($0) }
    }
}
