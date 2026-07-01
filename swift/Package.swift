// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "CtxSyncBar",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(
            name: "CtxSyncBar",
            path: "Sources/CtxSyncBar"
        )
    ]
)
