import Foundation

/// Gitignore-style pattern matching (the common subset: `*`, `?`, `**`,
/// leading `/` anchors, trailing `/` for directories, `!` negation,
/// last-match-wins).
final class IgnoreMatcher {
    private struct Rule {
        let regex: NSRegularExpression
        let negated: Bool
        let dirOnly: Bool
    }

    private var rules: [Rule] = []

    convenience init?(contentsOf url: URL) {
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        self.init(lines: text.components(separatedBy: .newlines))
    }

    init(lines: [String]) {
        for rawLine in lines {
            var line = rawLine
            if line.hasPrefix("#") { continue }
            // Trailing spaces are ignored unless escaped; keep it simple
            line = line.trimmingCharacters(in: .whitespaces)
            if line.isEmpty { continue }

            var negated = false
            if line.hasPrefix("!") {
                negated = true
                line.removeFirst()
            }

            var dirOnly = false
            if line.hasSuffix("/") {
                dirOnly = true
                line.removeLast()
            }

            // A slash anywhere (after stripping the trailing one) anchors
            // the pattern to the root; otherwise it matches at any depth.
            var anchored = line.contains("/")
            if line.hasPrefix("/") {
                anchored = true
                line.removeFirst()
            }
            guard !line.isEmpty else { continue }

            let body = Self.translate(line)
            let pattern = anchored
                ? "^\(body)$"
                : "(^|.*/)\(body)$"
            guard let regex = try? NSRegularExpression(pattern: pattern) else { continue }
            rules.append(Rule(regex: regex, negated: negated, dirOnly: dirOnly))
        }
    }

    /// Translates one gitignore glob into a regex body.
    private static func translate(_ glob: String) -> String {
        var out = ""
        let chars = Array(glob)
        var i = 0
        while i < chars.count {
            let c = chars[i]
            switch c {
            case "*":
                if i + 1 < chars.count && chars[i + 1] == "*" {
                    // "**" — optionally swallowing the following slash
                    if i + 2 < chars.count && chars[i + 2] == "/" {
                        out += "(?:[^/]+/)*"
                        i += 3
                    } else {
                        out += ".*"
                        i += 2
                    }
                    continue
                }
                out += "[^/]*"
            case "?":
                out += "[^/]"
            default:
                out += NSRegularExpression.escapedPattern(for: String(c))
            }
            i += 1
        }
        return out
    }

    /// Whether `relativePath` (forward-slash separated, no leading slash)
    /// is ignored. Last matching rule wins.
    func isIgnored(_ relativePath: String, isDirectory: Bool) -> Bool {
        var ignored = false
        let range = NSRange(relativePath.startIndex..., in: relativePath)
        for rule in rules {
            if rule.dirOnly && !isDirectory { continue }
            if rule.regex.firstMatch(in: relativePath, range: range) != nil {
                ignored = !rule.negated
            }
        }
        return ignored
    }
}
