// Whether the provider itself is having a bad day.
//
// Both Anthropic and OpenAI publish a Statuspage-style feed; the
// `status.json` document carries one indicator for the whole service, which
// is all a menu bar needs.

import Foundation

public enum StatusIndicator: String, Codable, Sendable {
    case none
    case minor
    case major
    case critical
    case maintenance
    case unknown

    public var isIncident: Bool { self != .none && self != .unknown }

    public var label: String {
        switch self {
        case .none: "Operational"
        case .minor: "Degraded"
        case .major: "Partial outage"
        case .critical: "Major outage"
        case .maintenance: "Maintenance"
        case .unknown: "Status unknown"
        }
    }
}

public struct ProviderStatus: Codable, Equatable, Sendable {
    public var indicator: StatusIndicator
    public var description: String?
    public var updatedAt: Date?
    public var checkedAt: Date

    public init(indicator: StatusIndicator, description: String? = nil, updatedAt: Date? = nil, checkedAt: Date = Date()) {
        self.indicator = indicator
        self.description = description
        self.updatedAt = updatedAt
        self.checkedAt = checkedAt
    }
}

public enum StatusFeed {
    public static func url(for provider: Provider) -> URL {
        switch provider {
        case .claude: URL(string: "https://status.claude.com/api/v2/status.json")!
        case .codex: URL(string: "https://status.openai.com/api/v2/status.json")!
        }
    }

    public static func page(for provider: Provider) -> URL {
        switch provider {
        case .claude: URL(string: "https://status.claude.com/")!
        case .codex: URL(string: "https://status.openai.com/")!
        }
    }

    /// Parse a `status.json` document.
    public static func parse(_ data: Data, checkedAt: Date = Date()) -> ProviderStatus? {
        struct Response: Decodable {
            struct Status: Decodable {
                var indicator: String
                var description: String?
            }
            struct Page: Decodable {
                var updated_at: String?
            }
            var status: Status
            var page: Page?
        }
        guard let response = try? JSONDecoder().decode(Response.self, from: data) else { return nil }
        let indicator = StatusIndicator(rawValue: response.status.indicator) ?? .unknown
        let updated = response.page?.updated_at.flatMap { text in
            ISO8601DateFormatter.withFraction.date(from: text) ?? ISO8601DateFormatter.plain.date(from: text)
        }
        return ProviderStatus(indicator: indicator, description: response.status.description, updatedAt: updated, checkedAt: checkedAt)
    }

    public static func fetch(_ provider: Provider, session: URLSession = .shared) async -> ProviderStatus? {
        var request = URLRequest(url: url(for: provider))
        request.timeoutInterval = 10
        request.cachePolicy = .reloadIgnoringLocalCacheData
        guard let (data, response) = try? await session.data(for: request),
              let http = response as? HTTPURLResponse, http.statusCode == 200 else { return nil }
        return parse(data)
    }
}

extension ISO8601DateFormatter {
    static let withFraction: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    static let plain: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()
}
