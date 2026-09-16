// When to say something.
//
// Thresholds are percent *remaining*. A warning fires when a window's
// remaining share drops to or below a threshold it has not fired for yet,
// and re-arms once the share climbs back above it. Firing at 20% marks 50%
// as fired too, so a steep drop does not produce two alerts.

import Foundation

public enum QuotaThresholds {
    public static let defaults = [50, 20]

    /// Unique, descending, each within 0...99.
    public static func sanitized(_ raw: [Int]) -> [Int] {
        let cleaned = Set(raw.map { min(99, max(0, $0)) }).sorted(by: >)
        return cleaned.isEmpty ? defaults : cleaned
    }

    public static func active(_ raw: [Int]) -> [Int] { sanitized(raw).filter { $0 > 0 } }
}

public enum QuotaAlertLogic {
    /// The threshold crossed on this sample, if any.
    public static func crossedThreshold(previousRemaining: Double?, currentRemaining: Double, thresholds: [Int], alreadyFired: Set<Int>) -> Int? {
        let eligible = QuotaThresholds.active(thresholds).filter { currentRemaining <= Double($0) && !alreadyFired.contains($0) }
        guard !eligible.isEmpty else { return nil }
        if let previousRemaining {
            return eligible.filter { previousRemaining > Double($0) }.min()
        }
        return eligible.min()
    }

    /// Everything at or above the fired threshold counts as fired.
    public static func firedThresholds(after threshold: Int, thresholds: [Int]) -> Set<Int> {
        Set(QuotaThresholds.active(thresholds).filter { $0 >= threshold })
    }

    /// Thresholds to re-arm because the window has recovered past them.
    public static func thresholdsToClear(currentRemaining: Double, alreadyFired: Set<Int>) -> Set<Int> {
        Set(alreadyFired.filter { currentRemaining > Double($0) })
    }

    static let depletedThreshold = 0.0001

    public static func isDepleted(_ remaining: Double?) -> Bool {
        remaining.map { $0 <= depletedThreshold } ?? false
    }

    public enum SessionTransition: Equatable, Sendable {
        case none
        case depleted
        case restored
    }

    public static func sessionTransition(wasDepleted: Bool?, remaining: Double?) -> SessionTransition {
        let depleted = isDepleted(remaining)
        switch (wasDepleted, depleted) {
        case (.some(false), true): return .depleted
        case (.some(true), false): return .restored
        case (.none, true): return .depleted
        default: return .none
        }
    }
}

/// Per-window alert state the app keeps between samples.
public struct WindowAlertState: Codable, Equatable, Sendable {
    public var lastRemaining: Double?
    public var fired: Set<Int> = []
    public var depleted: Bool?

    public init() {}
}

/// What happened on a sample: alerts to post, state to keep.
public struct AlertOutcome: Equatable, Sendable {
    public var warning: Int?
    public var transition: QuotaAlertLogic.SessionTransition
    public var state: WindowAlertState
}

public extension QuotaAlertLogic {
    /// Feed one sample through the state machine.
    static func evaluate(remaining: Double, thresholds: [Int], state: WindowAlertState) -> AlertOutcome {
        var next = state
        // Re-arm first, so a recovery past a threshold and a later drop both count.
        next.fired.subtract(thresholdsToClear(currentRemaining: remaining, alreadyFired: next.fired))
        let crossed = crossedThreshold(previousRemaining: state.lastRemaining, currentRemaining: remaining, thresholds: thresholds, alreadyFired: next.fired)
        if let crossed {
            next.fired.formUnion(firedThresholds(after: crossed, thresholds: thresholds))
        }
        let transition = sessionTransition(wasDepleted: state.depleted, remaining: remaining)
        next.depleted = isDepleted(remaining)
        next.lastRemaining = remaining
        return AlertOutcome(warning: crossed, transition: transition, state: next)
    }
}
