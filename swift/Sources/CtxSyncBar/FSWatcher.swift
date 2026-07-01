import CoreServices
import Foundation

/// Thin FSEvents wrapper: reports file-system changes under a directory.
/// Which paths count is decided by the injected `isRelevant` predicate so the
/// watcher and the file scanner share one definition of project content.
final class FSWatcher {
    /// Holds the callback state. The FSEvents stream retains it through the
    /// context's retain/release callbacks, so an in-flight event on the
    /// watcher queue can never see a deallocated object even if the watcher
    /// itself is torn down concurrently from the main thread.
    private final class Sink {
        let isRelevant: @Sendable (String) -> Bool
        let onChange: @Sendable () -> Void

        init(isRelevant: @escaping @Sendable (String) -> Bool,
             onChange: @escaping @Sendable () -> Void) {
            self.isRelevant = isRelevant
            self.onChange = onChange
        }
    }

    private var stream: FSEventStreamRef?
    private let queue = DispatchQueue(label: "ctxsync.fswatcher")

    init?(path: String, latency: TimeInterval = 1.0,
          isRelevant: @escaping @Sendable (String) -> Bool,
          onChange: @escaping @Sendable () -> Void) {
        let sink = Sink(isRelevant: isRelevant, onChange: onChange)

        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(sink).toOpaque(),
            retain: { info in
                guard let info else { return nil }
                return UnsafeRawPointer(Unmanaged<Sink>.fromOpaque(info).retain().toOpaque())
            },
            release: { info in
                guard let info else { return }
                Unmanaged<Sink>.fromOpaque(info).release()
            },
            copyDescription: nil
        )

        let callback: FSEventStreamCallback = { _, info, numEvents, eventPaths, _, _ in
            guard let info else { return }
            let sink = Unmanaged<Sink>.fromOpaque(info).takeUnretainedValue()
            let paths = unsafeBitCast(eventPaths, to: NSArray.self) as? [String] ?? []
            if paths.prefix(Int(numEvents)).contains(where: sink.isRelevant) {
                sink.onChange()
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
