import Foundation

struct Organization: Identifiable, Hashable {
    let id: String
    let name: String
}

struct Project: Identifiable, Hashable {
    let id: String
    let name: String
}

struct RemoteFile {
    let uuid: String
    let fileName: String
    let content: String
    let createdAt: String
}

enum ClaudeError: LocalizedError {
    case unauthorized
    case forbidden
    case http(Int, String)
    case invalidResponse

    var errorDescription: String? {
        switch self {
        case .unauthorized:
            return "Session key rejected (401). Please log in again."
        case .forbidden:
            return "Received a 403 Forbidden error."
        case .http(let code, let body):
            return "API request failed with status code \(code): \(body.prefix(200))"
        case .invalidResponse:
            return "Invalid response from the API."
        }
    }
}

/// Minimal claude.ai API client covering what the menu bar app needs:
/// organizations, projects, and project documents.
final class ClaudeClient {
    private let baseURL: URL
    private let sessionKey: String
    private let session: URLSession

    private static let userAgent =
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:129.0) Gecko/20100101 Firefox/129.0"

    init(baseURL: URL, sessionKey: String) {
        self.baseURL = baseURL
        self.sessionKey = sessionKey
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        self.session = URLSession(configuration: configuration)
    }

    private func request(_ method: String, _ path: String,
                         body: [String: Any]? = nil) async throws -> Data {
        guard let url = URL(string: baseURL.absoluteString + path) else {
            throw ClaudeError.invalidResponse
        }
        var req = URLRequest(url: url)
        req.httpMethod = method
        req.setValue(Self.userAgent, forHTTPHeaderField: "User-Agent")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("sessionKey=\(sessionKey)", forHTTPHeaderField: "Cookie")
        if let body {
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
        }

        let (data, response) = try await session.data(for: req)
        guard let http = response as? HTTPURLResponse else {
            throw ClaudeError.invalidResponse
        }
        switch http.statusCode {
        case 200...299:
            return data
        case 401:
            throw ClaudeError.unauthorized
        case 403:
            throw ClaudeError.forbidden
        default:
            throw ClaudeError.http(http.statusCode,
                                   String(data: data, encoding: .utf8) ?? "")
        }
    }

    private func requestJSON(_ method: String, _ path: String,
                             body: [String: Any]? = nil) async throws -> Any {
        let data = try await request(method, path, body: body)
        guard !data.isEmpty else { return NSNull() }
        return try JSONSerialization.jsonObject(with: data)
    }

    /// Organizations with chat plus a paid-plan capability, as the CLI filters.
    func organizations() async throws -> [Organization] {
        guard let list = try await requestJSON("GET", "/organizations") as? [[String: Any]] else {
            throw ClaudeError.invalidResponse
        }
        let required: [Set<String>] = [
            ["chat", "claude_pro"], ["chat", "raven"], ["chat", "claude_max"],
        ]
        return list.compactMap { org in
            let capabilities = Set(org["capabilities"] as? [String] ?? [])
            guard required.contains(where: { $0.isSubset(of: capabilities) }) else { return nil }
            guard let id = org["uuid"] as? String, let name = org["name"] as? String else {
                return nil
            }
            return Organization(id: id, name: name)
        }
    }

    /// Active (non-archived) projects.
    func projects(organizationId: String) async throws -> [Project] {
        guard let list = try await requestJSON(
            "GET", "/organizations/\(organizationId)/projects") as? [[String: Any]] else {
            throw ClaudeError.invalidResponse
        }
        return list.compactMap { project in
            guard project["archived_at"] == nil || project["archived_at"] is NSNull else {
                return nil
            }
            guard let id = project["uuid"] as? String, let name = project["name"] as? String else {
                return nil
            }
            return Project(id: id, name: name)
        }
    }

    func listFiles(organizationId: String, projectId: String) async throws -> [RemoteFile] {
        guard let list = try await requestJSON(
            "GET", "/organizations/\(organizationId)/projects/\(projectId)/docs")
            as? [[String: Any]] else {
            throw ClaudeError.invalidResponse
        }
        return list.compactMap { file in
            guard let uuid = file["uuid"] as? String,
                  let name = file["file_name"] as? String else { return nil }
            return RemoteFile(uuid: uuid,
                              fileName: name,
                              content: file["content"] as? String ?? "",
                              createdAt: file["created_at"] as? String ?? "")
        }
    }

    func uploadFile(organizationId: String, projectId: String,
                    fileName: String, content: String) async throws {
        _ = try await request(
            "POST", "/organizations/\(organizationId)/projects/\(projectId)/docs",
            body: ["file_name": fileName, "content": content])
    }

    func deleteFile(organizationId: String, projectId: String, uuid: String) async throws {
        _ = try await request(
            "DELETE", "/organizations/\(organizationId)/projects/\(projectId)/docs/\(uuid)")
    }
}
