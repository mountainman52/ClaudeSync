import Foundation

/// Gitignore-style pattern matching (the common subset: `*`, `?`, `**`,
/// `[...]` character classes, leading `/` anchors, trailing `/` for
/// directories, `!` negation, last-match-wins).
final class IgnoreMatcher: @unchecked Sendable {
    private struct Rule {
        let regex: NSRegularExpression
        let negated: Bool
        let dirOnly: Bool
    }

    private let rules: [Rule]

    convenience init?(contentsOf url: URL) {
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        self.init(lines: text.components(separatedBy: .newlines))
    }

    init(lines: [String]) {
        var rules: [Rule] = []
        for rawLine in lines {
            var line = rawLine
            if line.hasPrefix("#") { continue }
            // Git ignores unescaped trailing spaces but keeps leading ones.
            while line.hasSuffix(" ") && !line.hasSuffix("\\ ") {
                line.removeLast()
            }
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
        self.rules = rules
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
            case "[":
                if let (cls, next) = translateCharacterClass(chars, from: i) {
                    out += cls
                    i = next
                    continue
                }
                // Unterminated class: a literal '['
                out += "\\["
            default:
                out += NSRegularExpression.escapedPattern(for: String(c))
            }
            i += 1
        }
        return out
    }

    /// Translates a `[...]` class starting at `chars[start] == "["`; returns
    /// the regex class and the index just past the closing `]`, or nil when
    /// the class is unterminated.
    private static func translateCharacterClass(
        _ chars: [Character], from start: Int
    ) -> (String, Int)? {
        var i = start + 1
        var body = ""
        if i < chars.count && (chars[i] == "!" || chars[i] == "^") {
            body += "^"
            i += 1
        }
        // A ']' immediately after the opening (or the negation) is a literal
        // member, not the terminator.
        if i < chars.count && chars[i] == "]" {
            body += "\\]"
            i += 1
        }
        while i < chars.count && chars[i] != "]" {
            let member = chars[i]
            if member == "\\" || member == "^" || member == "[" {
                body += "\\" + String(member)
            } else {
                body += String(member)
            }
            i += 1
        }
        guard i < chars.count, !body.isEmpty, body != "^" else { return nil }
        return ("[" + body + "]", i + 1)
    }

    /// Whether `relativePath` (forward-slash separated, no leading slash)
    /// is ignored. Last matching rule wins, so scanning in reverse lets the
    /// first hit decide.
    func isIgnored(_ relativePath: String, isDirectory: Bool) -> Bool {
        let range = NSRange(relativePath.startIndex..., in: relativePath)
        for rule in rules.reversed() {
            if rule.dirOnly && !isDirectory { continue }
            if rule.regex.firstMatch(in: relativePath, range: range) != nil {
                return !rule.negated
            }
        }
        return false
    }
}
