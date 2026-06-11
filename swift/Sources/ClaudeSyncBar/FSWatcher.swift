import CoreServices
import Foundation

/// Thin FSEvents wrapper: reports file-system changes under a directory,
/// excluding paths the sync itself writes to (.claudesync, claude_chats, .git).
final class FSWatcher {
    private var stream: FSEventStreamRef?
    private let queue = DispatchQueue(label: "claudesync.fswatcher")
    private let onChange: () -> Void

    init?(path: String, latency: TimeInterval = 1.0, onChange: @escaping () -> Void) {
        self.onChange = onChange

        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil,
            release: nil,
            copyDescription: nil
        )

        let callback: FSEventStreamCallback = { _, info, numEvents, eventPaths, _, _ in
            guard let info else { return }
            let watcher = Unmanaged<FSWatcher>.fromOpaque(info).takeUnretainedValue()
            let paths = unsafeBitCast(eventPaths, to: NSArray.self) as? [String] ?? []
            let relevant = paths.prefix(Int(numEvents)).contains { path in
                !path.contains("/.claudesync/")
                    && !path.contains("/.git/")
                    && !path.contains("/claude_chats/")
            }
            if relevant {
                watcher.onChange()
            }
        }

        guard let stream = FSEventStreamCreate(
            kCFAllocatorDefault,
            callback,
            &context,
            [path] as CFArray,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
            latency,
            FSEventStreamCreateFlags(kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagFileEvents)
        ) else { return nil }

        self.stream = stream
        FSEventStreamSetDispatchQueue(stream, queue)
        FSEventStreamStart(stream)
    }

    func stop() {
        guard let stream else { return }
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
        self.stream = nil
    }

    deinit {
        stop()
    }
}
