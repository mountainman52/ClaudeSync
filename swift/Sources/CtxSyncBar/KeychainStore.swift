import Foundation
import Security

/// Session key storage in the macOS Keychain.
///
/// Uses the same generic-password item as the Rust CLI's keyring backend
/// (service "ctxsync", account = provider name, value = JSON payload with
/// `session_key` and `session_key_expiry`), so the CLI and this app share a
/// single login. Items stored under the pre-rename "claudesync" service are
/// migrated on first read.
enum KeychainStore {
    static let service = "ctxsync"
    static let legacyService = "claudesync"

    struct SessionKey {
        let key: String
        let expiry: Date
    }

    enum KeychainError: LocalizedError {
        case unexpectedStatus(OSStatus)

        var errorDescription: String? {
            switch self {
            case .unexpectedStatus(let status):
                let message = SecCopyErrorMessageString(status, nil) as String? ?? "code \(status)"
                return "Keychain error: \(message)"
            }
        }
    }

    private static func baseQuery(account: String,
                                  service: String = KeychainStore.service) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    private static func readData(service: String, account: String) -> Data? {
        var query = baseQuery(account: account, service: service)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess else {
            return nil
        }
        return result as? Data
    }

    /// If no item exists under the new service name, moves any item stored
    /// under the legacy "claudesync" service to it. Best-effort.
    private static func migrateLegacyItem(account: String) {
        guard readData(service: service, account: account) == nil,
              let legacyData = readData(service: legacyService, account: account) else { return }
        var add = baseQuery(account: account)
        add[kSecValueData as String] = legacyData
        if SecItemAdd(add as CFDictionary, nil) == errSecSuccess {
            SecItemDelete(baseQuery(account: account, service: legacyService) as CFDictionary)
        }
    }

    static func save(account: String, sessionKey: String, expiry: Date) throws {
        let payload: [String: Any] = [
            "session_key": sessionKey,
            "session_key_expiry": ISOFormat.string(from: expiry),
        ]
        let data = try JSONSerialization.data(withJSONObject: payload)

        let attributes: [String: Any] = [kSecValueData as String: data]
        let status = SecItemUpdate(baseQuery(account: account) as CFDictionary,
                                   attributes as CFDictionary)
        if status == errSecItemNotFound {
            var add = baseQuery(account: account)
            add[kSecValueData as String] = data
            let addStatus = SecItemAdd(add as CFDictionary, nil)
            guard addStatus == errSecSuccess else {
                throw KeychainError.unexpectedStatus(addStatus)
            }
        } else if status != errSecSuccess {
            throw KeychainError.unexpectedStatus(status)
        }
    }

    /// Returns the stored key if present and unexpired.
    static func load(account: String) throws -> SessionKey? {
        guard let stored = try fetch(account: account) else { return nil }
        return stored.expiry > Date() ? stored : nil
    }

    /// The stored payload even when expired — used to preserve a
    /// user-supplied expiry (e.g. entered via the CLI) across re-logins.
    static func stored(account: String) -> SessionKey? {
        (try? fetch(account: account)) ?? nil
    }

    private static func fetch(account: String) throws -> SessionKey? {
        migrateLegacyItem(account: account)
        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else {
            throw KeychainError.unexpectedStatus(status)
        }

        guard
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let key = object["session_key"] as? String,
            let expiryString = object["session_key_expiry"] as? String,
            let expiry = ISOFormat.date(from: expiryString)
        else { return nil }

        return SessionKey(key: key, expiry: expiry)
    }

    static func delete(account: String) {
        SecItemDelete(baseQuery(account: account) as CFDictionary)
        SecItemDelete(baseQuery(account: account, service: legacyService) as CFDictionary)
    }
}

/// Naive-UTC ISO timestamps in the exact shape the Rust/Python tools write,
/// e.g. `2026-07-11T12:00:00.000000`.
enum ISOFormat {
    private static func formatter(_ format: String) -> DateFormatter {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "UTC")
        f.dateFormat = format
        return f
    }

    private static let fractional = formatter("yyyy-MM-dd'T'HH:mm:ss.SSSSSS")
    private static let whole = formatter("yyyy-MM-dd'T'HH:mm:ss")

    static func string(from date: Date) -> String {
        fractional.string(from: date)
    }

    static func date(from string: String) -> Date? {
        fractional.date(from: string) ?? whole.date(from: string)
    }
}
