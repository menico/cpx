// How often to ask again.
//
// A pure function of what the app can observe — when the menu was last
// opened, whether the machine is under pressure, whether an agent is
// running — so the cadence can be reasoned about and tested. The delay is
// always between two and thirty minutes.

import Foundation

public enum RefreshCadence: String, Codable, CaseIterable, Sendable {
    case adaptive
    case manual
    case every1m
    case every2m
    case every5m
    case every15m
    case every30m

    public var label: String {
        switch self {
        case .adaptive: "Adaptive"
        case .manual: "Manual"
        case .every1m: "Every minute"
        case .every2m: "Every 2 minutes"
        case .every5m: "Every 5 minutes"
        case .every15m: "Every 15 minutes"
        case .every30m: "Every 30 minutes"
        }
    }

    /// The fixed interval, or nil for adaptive and manual.
    public var fixedInterval: TimeInterval? {
        switch self {
        case .every1m: 60
        case .every2m: 120
        case .every5m: 300
        case .every15m: 900
        case .every30m: 1800
        case .adaptive, .manual: nil
        }
    }
}

public struct RefreshInput: Sendable {
    public var now: Date
    public var lastMenuOpenAt: Date?
    /// The newest transcript write seen, when the app watches for that.
    public var lastCodingActivityAt: Date?
    public var lowPowerMode: Bool
    /// Thermal state serious or critical.
    public var thermalPressure: Bool

    public init(now: Date, lastMenuOpenAt: Date? = nil, lastCodingActivityAt: Date? = nil, lowPowerMode: Bool = false, thermalPressure: Bool = false) {
        self.now = now
        self.lastMenuOpenAt = lastMenuOpenAt
        self.lastCodingActivityAt = lastCodingActivityAt
        self.lowPowerMode = lowPowerMode
        self.thermalPressure = thermalPressure
    }
}

public enum RefreshReason: String, Sendable {
    case constrained
    case recentInteraction
    case warm
    case idle
    case longIdle
    case codingActivity
}

public struct RefreshDecision: Equatable, Sendable {
    public var delay: TimeInterval
    public var reason: RefreshReason
}

public enum RefreshPolicy {
    static let recentInteractionThreshold: TimeInterval = 5 * 60
    static let warmThreshold: TimeInterval = 60 * 60
    static let idleThreshold: TimeInterval = 4 * 60 * 60
    static let codingActivityThreshold: TimeInterval = 5 * 60

    public static let recentInteractionDelay: TimeInterval = 2 * 60
    static let warmDelay: TimeInterval = 5 * 60
    static let idleDelay: TimeInterval = 15 * 60
    static let longIdleDelay: TimeInterval = 30 * 60
    static let constrainedDelay: TimeInterval = 30 * 60
    static let codingActivityCap: TimeInterval = 5 * 60

    public static func nextDelay(for input: RefreshInput) -> RefreshDecision {
        if input.lowPowerMode || input.thermalPressure {
            return RefreshDecision(delay: constrainedDelay, reason: .constrained)
        }
        let base = menuDecision(for: input)
        if let activity = input.lastCodingActivityAt,
           input.now.timeIntervalSince(activity) < codingActivityThreshold,
           base.delay > codingActivityCap {
            return RefreshDecision(delay: codingActivityCap, reason: .codingActivity)
        }
        return base
    }

    private static func menuDecision(for input: RefreshInput) -> RefreshDecision {
        guard let opened = input.lastMenuOpenAt else {
            return RefreshDecision(delay: longIdleDelay, reason: .longIdle)
        }
        // A negative age (clock moved back) reads as recent.
        let age = input.now.timeIntervalSince(opened)
        if age <= recentInteractionThreshold { return RefreshDecision(delay: recentInteractionDelay, reason: .recentInteraction) }
        if age <= warmThreshold { return RefreshDecision(delay: warmDelay, reason: .warm) }
        if age < idleThreshold { return RefreshDecision(delay: idleDelay, reason: .idle) }
        return RefreshDecision(delay: longIdleDelay, reason: .longIdle)
    }
}
