// The typed contract with the `cpx` command.
//
// Every shape here mirrors what a `cpx … --json` call prints. The app never
// re-derives a decision from these: it renders them.

import Foundation

public enum Provider: String, Codable, Hashable, Sendable, CaseIterable {
    case claude
    case codex

    public var displayName: String {
        switch self {
        case .claude: "Claude Code"
        case .codex: "Codex"
        }
    }

    public var shortName: String {
        switch self {
        case .claude: "Claude"
        case .codex: "Codex"
        }
    }

    public var binaryName: String { rawValue }
}

public struct Paths: Codable, Sendable {
    public var home: String
    public var root: String
    public var config: String
    public var initialised: Bool
    public var bindings: String
    public var state: String
}

public struct ProfileRow: Codable, Hashable, Identifiable, Sendable {
    public var name: String
    public var provider: Provider
    public var description: String
    public var color: String?
    public var adopted: Bool
    public var directory: String
    public var applied: Bool
    public var authenticated: Bool
    public var account: String?
    public var organization: String?
    public var plan: String?
    public var expired: Bool?
    /// "Keychain", "File", "ApiKey" or "None".
    public var source: String
    public var model: String?
    public var command: String

    public var id: String { name }
}

public struct DefaultSession: Codable, Hashable, Sendable {
    public var provider: Provider
    public var directory: String
    public var account: String?
    public var signedIn: Bool
    public var isSource: Bool
    public var claimedBy: String?
}

public struct ProfileList: Codable, Sendable {
    public var profiles: [ProfileRow]
    public var defaultSessions: [DefaultSession]
}

public struct ResourceRow: Codable, Hashable, Identifiable, Sendable {
    public var resource: String
    public var mode: String
    public var isDirectory: Bool
    public var supportsMerge: Bool
    public var patch: AnyJSON?

    public var id: String { resource }
    public var hasPatch: Bool { patch != nil && patch != .null }
}

public struct ProfileDetail: Codable, Sendable {
    public var name: String
    public var provider: Provider
    public var description: String
    public var color: String?
    public var adopted: Bool
    public var directory: String
    public var applied: Bool
    public var authenticated: Bool
    public var account: String?
    public var plan: String?
    public var source: String
    public var model: String?
    public var command: String
    public var addDirs: [String]
    public var env: [String: String]
    public var resources: [ResourceRow]
    public var credentialLocation: String
    public var sourceDir: String

    public var row: ProfileRow {
        ProfileRow(
            name: name, provider: provider, description: description, color: color, adopted: adopted,
            directory: directory, applied: applied, authenticated: authenticated, account: account,
            organization: nil, plan: plan, expired: nil, source: source, model: model, command: command)
    }
}

public struct PlanLine: Codable, Hashable, Sendable {
    /// "safe", "generated" or "foreign".
    public var risk: String
    public var verb: String
    public var target: String
    public var description: String
    public var profile: String?
}

public struct Plan: Codable, Sendable {
    public var actions: [PlanLine]
    public var pendingActions: Int
    public var notes: [String]
    public var requiresForce: Bool

    public static let empty = Plan(actions: [], pendingActions: 0, notes: [], requiresForce: false)
}

public struct ApplyResult: Codable, Sendable {
    public var performed: Int
    public var backups: [[String]]
    public var notes: [String]
}

public struct BindingRow: Codable, Hashable, Identifiable, Sendable {
    public var path: String
    public var profile: String
    public var provider: Provider
    public var color: String?
    /// "Healthy", "DirectoryMissing", "ProfileMissing", "BlockAbsent", "BlockEdited", "NotAllowed".
    public var health: String
    public var healthy: Bool

    public var id: String { "\(provider.rawValue):\(path)" }

    public var problem: String? {
        switch health {
        case "Healthy": nil
        case "DirectoryMissing": "Directory is gone"
        case "ProfileMissing": "Profile no longer exists"
        case "BlockAbsent": "The .envrc no longer has its cpx block"
        case "BlockEdited": "Edited by hand"
        case "NotAllowed": "direnv has not allowed this .envrc"
        default: health
        }
    }
}

public struct Check: Codable, Hashable, Identifiable, Sendable {
    public var name: String
    /// "Ok", "Warning" or "Error".
    public var severity: String
    public var detail: String
    public var remedy: String?

    public var id: String { name + detail }
}

public struct AdoptionRow: Codable, Hashable, Identifiable, Sendable {
    public var name: String
    public var provider: Provider
    public var dir: String
    public var found: [String]
    public var taken: Bool

    public var id: String { dir }
}

public struct StatuslineRow: Codable, Sendable {
    public var target: String
    public var badge: Bool
    public var command: String?
    public var delegate: String?
    public var script: String
    public var needsApply: Bool
}

public struct ScriptFile: Codable, Sendable {
    public var path: String
    public var contents: String
    public var owned: Bool
    public var managedBy: String?
}

public struct Skill: Codable, Hashable, Identifiable, Sendable {
    public var name: String
    public var description: String?
    public var enabled: Bool
    public var path: String
    public var id: String { name }
}

public struct PluginSkills: Codable, Hashable, Identifiable, Sendable {
    public var key: String
    public var plugin: String
    public var marketplace: String
    public var enabled: Bool
    public var skills: Int
    public var names: [String]
    public var id: String { key }
}

public struct SkillInventory: Codable, Sendable {
    public var own: [Skill]
    public var plugins: [PluginSkills]
    public var shared: Bool
}

public struct PluginChange: Codable, Sendable {
    public var plugin: String
    public var enabled: Bool
    public var needsApply: Bool
}

// MARK: - Usage

public struct RateWindow: Codable, Hashable, Identifiable, Sendable {
    public var id: String
    public var title: String
    public var usedPercent: Double
    public var windowMinutes: Int?
    public var resetsAt: String?
    public var resetsAtEpoch: Int?

    public var remainingPercent: Double { max(0, 100 - usedPercent) }
    public var resetDate: Date? { resetsAtEpoch.map { Date(timeIntervalSince1970: TimeInterval($0)) } }

    public init(id: String, title: String, usedPercent: Double, windowMinutes: Int? = nil, resetsAt: String? = nil, resetsAtEpoch: Int? = nil) {
        self.id = id
        self.title = title
        self.usedPercent = usedPercent
        self.windowMinutes = windowMinutes
        self.resetsAt = resetsAt
        self.resetsAtEpoch = resetsAtEpoch
    }
}

public struct ExtraUsage: Codable, Hashable, Sendable {
    public var enabled: Bool
    public var usedUsd: Double?
    public var limitUsd: Double?
    public var utilization: Double?
}

public struct Credits: Codable, Hashable, Sendable {
    public var hasCredits: Bool
    public var unlimited: Bool
    public var balance: Double?
}

public struct UsageSnapshot: Codable, Hashable, Sendable {
    public var provider: Provider
    public var account: String?
    public var plan: String?
    public var windows: [RateWindow]
    public var extraUsage: ExtraUsage?
    public var credits: Credits?
    public var source: String
    public var fetchedAt: Int

    public var session: RateWindow? { windows.first { $0.id == "session" } }
    public var weekly: RateWindow? { windows.first { $0.id == "weekly" } }
    public var fetched: Date { Date(timeIntervalSince1970: TimeInterval(fetchedAt)) }

    public init(provider: Provider, account: String? = nil, plan: String? = nil, windows: [RateWindow], extraUsage: ExtraUsage? = nil, credits: Credits? = nil, source: String = "oauth", fetchedAt: Int = 0) {
        self.provider = provider
        self.account = account
        self.plan = plan
        self.windows = windows
        self.extraUsage = extraUsage
        self.credits = credits
        self.source = source
        self.fetchedAt = fetchedAt
    }
}

public struct UsageError: Codable, Hashable, Sendable {
    /// "notSignedIn", "tokenExpired", "keychainDenied", "unauthorized",
    /// "rateLimited", "http", "network", "parse", "cli".
    public var kind: String
    public var command: String?
    public var detail: String?
    public var retryAfterSeconds: Int?
    public var status: Int?
    public var body: String?

    /// Errors where running the account's own command would fix things.
    public var isLoginProblem: Bool {
        ["notSignedIn", "tokenExpired", "keychainDenied", "unauthorized"].contains(kind)
    }

    /// cpx has not been allowed to read the login; a refresh that may ask
    /// would fix it.
    public var needsKeychainPermission: Bool { kind == "keychainNeedsPermission" }
}

public struct UsageAccount: Codable, Hashable, Identifiable, Sendable {
    public var profile: String?
    public var provider: Provider
    public var directory: String
    public var command: String
    public var usage: UsageSnapshot?
    public var error: UsageError?
    public var message: String?

    /// A stable key: the profile name, or `@claude` / `@codex` for a
    /// default session, matching what `cpx usage` accepts.
    public var id: String { profile ?? "@\(provider.rawValue)" }
    public var isDefaultSession: Bool { profile == nil }
}

public struct UsageReport: Codable, Sendable {
    public var accounts: [UsageAccount]
}

// MARK: - Cost

public struct Tokens: Codable, Hashable, Sendable {
    public var input: Int
    public var output: Int
    public var cacheRead: Int
    public var cacheWrite: Int
    public var cacheWrite1h: Int?

    public var total: Int { input + output + cacheRead + cacheWrite }
}

public struct DaySpend: Codable, Hashable, Identifiable, Sendable {
    public var day: String
    public var tokens: Tokens
    public var costUsd: Double?
    public var partlyUnpriced: Bool
    public var id: String { day }
}

public struct Spend: Codable, Hashable, Sendable {
    public var tokens: Tokens
    public var costUsd: Double
    public var partlyUnpriced: Bool
}

public struct ModelSpend: Codable, Hashable, Identifiable, Sendable {
    public var model: String
    public var tokens: Tokens
    public var costUsd: Double?
    public var id: String { model }
}

public struct CostReport: Codable, Hashable, Sendable {
    public var provider: Provider
    public var days: [DaySpend]
    public var today: Spend
    public var last7Days: Spend
    public var last30Days: Spend
    public var models: [ModelSpend]
    public var unpricedModels: [String]
    public var filesScanned: Int
    public var filesFromCache: Int
}

public struct CostAccount: Codable, Hashable, Identifiable, Sendable {
    public var profile: String?
    public var provider: Provider
    public var directory: String
    public var cost: CostReport
    public var id: String { profile ?? "@\(provider.rawValue)" }
}

public struct CostReportList: Codable, Sendable {
    public var accounts: [CostAccount]
    public var days: Int
}

// MARK: - JSON of unknown shape

/// A JSON value the app carries without interpreting: a merge patch, say.
public indirect enum AnyJSON: Codable, Hashable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([AnyJSON])
    case object([String: AnyJSON])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([AnyJSON].self) {
            self = .array(value)
        } else if let value = try? container.decode([String: AnyJSON].self) {
            self = .object(value)
        } else {
            throw DecodingError.dataCorruptedError(in: container, debugDescription: "not JSON")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .string(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        }
    }
}
