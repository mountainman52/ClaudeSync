// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ClaudeSyncBar",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(
            name: "ClaudeSyncBar",
            path: "Sources/ClaudeSyncBar"
        )
    ]
)
